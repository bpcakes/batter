mod support;
use batter_core::operation::{Interruption, OperationError};
use batter_runlimit::quota::{Admission, Checks, InterruptedCheck, Quota, RunResult};
use runlimit_core::{BatchDecision, Check, ConsumptionStatus, Denial, QuotaMode};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::*;

#[tokio::test]
async fn native_quota_runs_once_and_rejects_before_work_factory() {
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let quota = Quota::new(memory());
    let calls = AtomicUsize::new(0);
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(42) }
        })
        .await;
    let RunResult::Admitted {
        admission: Admission::Allowed { allowances },
        work: Ok(42),
    } = result
    else {
        panic!("expected a consumed allowed batch");
    };
    assert_eq!(allowances.len(), 1);
    let allowance = allowances.iter().next().unwrap();
    assert_eq!(allowance.capacity().get(), 1);
    assert_eq!(allowance.available(), 0);
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(0) }
        })
        .await;
    let RunResult::Rejected {
        index,
        batch_size,
        denial,
    } = result
    else {
        panic!("expected an enforced denial");
    };
    assert_eq!(index, 0);
    assert_eq!(batch_size.get(), 1);
    assert!(matches!(denial, Denial::QuotaExceeded(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn storage_denial_retains_exact_retry_delay_without_running_work() {
    let delay = Duration::from_millis(2_001);
    let backend = Backend::new(Mode::Return(
        BatchDecision::denied(
            0,
            1,
            Denial::StorageCapacity {
                retry_after: Some(delay.into()),
            },
        )
        .unwrap(),
    ));
    let quota = Quota::new(backend);
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let calls = AtomicUsize::new(0);
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(()) }
        })
        .await;
    let RunResult::Rejected {
        index: 0,
        batch_size,
        denial,
    } = result
    else {
        panic!("expected a storage denial");
    };
    let Denial::StorageCapacity {
        retry_after: Some(retry_after),
    } = denial
    else {
        panic!("expected a typed storage retry delay");
    };
    assert_eq!(retry_after.duration(), delay);
    assert_eq!(retry_after.seconds(), 3);
    assert_eq!(batch_size.get(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn never_polled_and_pre_cancelled_do_no_backend_or_factory_work() {
    let backend = Backend::new(Mode::Return(allowed()));
    let quota = Quota::new(backend.clone());
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let owner = batter_core::operation::OperationOwner::new(Duration::from_millis(100)).unwrap();
    let context = owner.context().clone();
    let calls = AtomicUsize::new(0);
    let work = |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, Infallible>(()) }
    };
    drop(quota.run(&context, Checks::new(&checks).unwrap(), work));
    owner.cancel();
    let result = quota
        .run(&context, Checks::new(&checks).unwrap(), work)
        .await;
    assert!(matches!(
        result,
        RunResult::Interrupted {
            check: InterruptedCheck::NotStarted,
            ..
        }
    ));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn never_polled_quota_does_not_consume_native_memory() {
    let quota = Quota::new(memory());
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let context = context(100);

    drop(
        quota.run(&context, Checks::new(&checks).unwrap(), |_| async {
            Ok::<_, Infallible>(())
        }),
    );

    assert!(matches!(
        quota
            .run(&context, Checks::new(&checks).unwrap(), |_| async {
                Ok::<_, Infallible>(())
            })
            .await,
        RunResult::Admitted { work: Ok(()), .. }
    ));
    assert!(matches!(
        quota
            .run(&context, Checks::new(&checks).unwrap(), |_| async {
                Ok::<_, Infallible>(())
            })
            .await,
        RunResult::Rejected { .. }
    ));
}

#[tokio::test]
async fn native_memory_trait_futures_defer_consumption_until_polled() {
    for batch in [false, true] {
        let store = memory();
        let policy = policy("owner", 1);
        let checks = [Check::new(subject(1).bind(&policy))];

        if batch {
            drop(runlimit_core::Limiter::check_all(&store, &checks));
            let first = runlimit_core::Limiter::check_all(&store, &checks)
                .await
                .unwrap();
            assert!(matches!(
                first.view(),
                runlimit_core::BatchDecisionView::Allowed { .. }
            ));
        } else {
            drop(runlimit_core::Limiter::check(&store, &checks[0]));
            let first = runlimit_core::Limiter::check(&store, &checks[0])
                .await
                .unwrap();
            assert!(matches!(
                first.view(),
                runlimit_core::DecisionView::Allowed { .. }
            ));
        }

        assert!(matches!(
            store.check(&checks[0]).unwrap().view(),
            runlimit_core::DecisionView::Denied { .. }
        ));
    }
}

#[tokio::test(start_paused = true)]
async fn unresolved_quota_timeout_drops_backend_and_never_starts_work() {
    let backend = Backend::new(Mode::Pending);
    let quota = Quota::new(backend.clone());
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let result = quota
        .run(&context(10), Checks::new(&checks).unwrap(), |_| async {
            panic!("work must not start");
            #[allow(unreachable_code)]
            Ok::<(), Infallible>(())
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Interrupted {
            reason: Interruption::DeadlineExceeded,
            check: InterruptedCheck::InFlight
        }
    ));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_after_quota_return_preserves_grant_without_starting_work() {
    let owner =
        Arc::new(batter_core::operation::OperationOwner::new(Duration::from_millis(100)).unwrap());
    let context = owner.context().clone();
    let quota = Quota::new(Backend::new(Mode::CancelThenAllow(owner)));
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let calls = AtomicUsize::new(0);
    let result = quota
        .run(&context, Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(()) }
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Admitted {
            work: Err(OperationError::Interrupted(Interruption::Cancelled)),
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn admission_and_work_share_one_total_budget() {
    let quota = Quota::new(Backend::new(Mode::Delay(Duration::from_millis(6))));
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(1).bind(&policy))];
    let start = tokio::time::Instant::now();
    let result = quota
        .run(&context(10), Checks::new(&checks).unwrap(), |_| async {
            tokio::time::sleep(Duration::from_millis(6)).await;
            Ok::<_, Infallible>(())
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Admitted {
            work: Err(OperationError::Interrupted(Interruption::DeadlineExceeded)),
            ..
        }
    ));
    assert_eq!(start.elapsed(), Duration::from_millis(10));
}

#[tokio::test]
async fn native_atomic_batch_denial_does_not_consume_the_other_bucket() {
    let store = memory();
    let first = policy("owner", 1);
    let second = policy("peer", 1);
    // Exhaust only the second bucket before the attempted atomic admission.
    store.check(&Check::new(subject(2).bind(&second))).unwrap();
    let quota = Quota::new(store);
    let both = [
        Check::new(subject(1).bind(&first)),
        Check::new(subject(2).bind(&second)),
    ];
    let RunResult::Rejected {
        index: 1,
        batch_size,
        ..
    } = quota
        .run(&context(100), Checks::new(&both).unwrap(), |_| async {
            Ok::<_, Infallible>(())
        })
        .await
    else {
        panic!("expected the second check to deny the two-check batch");
    };
    assert_eq!(batch_size.get(), 2);
    let just_first = [Check::new(subject(1).bind(&first))];
    assert!(matches!(
        quota
            .run(
                &context(100),
                Checks::new(&just_first).unwrap(),
                |_| async { Ok::<_, Infallible>(()) }
            )
            .await,
        RunResult::Admitted { work: Ok(()), .. }
    ));
}

#[tokio::test]
async fn native_shadow_denial_allows_work_and_preserves_native_decision() {
    let policy = policy("owner", 1).with_quota_mode(QuotaMode::Shadow);
    let checks = [Check::new(subject(1).bind(&policy))];
    let quota = Quota::new(memory());
    let _ = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
            Ok::<_, Infallible>(())
        })
        .await;
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
            Ok::<_, Infallible>(99)
        })
        .await;
    let RunResult::Admitted {
        admission,
        work: Ok(99),
    } = result
    else {
        panic!("wrong result: {result:?}")
    };
    let Admission::ShadowDenied {
        index,
        batch_size,
        denial,
    } = admission
    else {
        panic!("expected shadow admission");
    };
    assert_eq!(index, 0);
    assert_eq!(batch_size.get(), 1);
    assert_eq!(denial.capacity().get(), 1);
}

#[tokio::test]
async fn backend_error_preserves_cause_and_certainty_without_retry() {
    for certainty in [
        ConsumptionStatus::Consumed,
        ConsumptionStatus::NotConsumed,
        ConsumptionStatus::PossiblyConsumed,
    ] {
        let backend = Backend::new(Mode::Fail(certainty));
        let quota = Quota::new(backend.clone());
        let policy = policy("owner", 1);
        let checks = [Check::new(subject(1).bind(&policy))];
        let result = quota
            .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
                panic!("must not run");
                #[allow(unreachable_code)]
                Ok::<(), Infallible>(())
            })
            .await;
        let RunResult::Backend { error, consumption } = result else {
            panic!("wrong result")
        };
        assert_eq!(consumption, certainty);
        assert_eq!(error.to_string(), "secret-backend-cause");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn borrowed_work_and_concrete_domain_error_need_no_box_or_static_bound() {
    let quota = Quota::new(memory());
    let policy = policy("owner", 10);
    let checks = [Check::new(subject(1).bind(&policy))];
    let mut local = String::new();
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
            local.push_str("changed");
            Err::<(), _>(17_u32)
        })
        .await;
    assert_eq!(local, "changed");
    assert!(matches!(
        result,
        RunResult::Admitted {
            work: Err(OperationError::Failed(17)),
            ..
        }
    ));
}

#[tokio::test]
async fn native_memory_grant_stays_spent_after_domain_failure() {
    let quota = Quota::new(memory());
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(31).bind(&policy))];
    let calls = AtomicUsize::new(0);
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(17_u32) }
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Admitted {
            work: Err(OperationError::Failed(17)),
            ..
        }
    ));
    let later = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<(), u32>(()) }
        })
        .await;
    assert!(matches!(later, RunResult::Rejected { index: 0, .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_memory_grant_stays_spent_after_post_grant_cancellation() {
    let quota = Quota::new(memory());
    let policy = policy("owner", 1);
    let checks = [Check::new(subject(32).bind(&policy))];
    let owner =
        Arc::new(batter_core::operation::OperationOwner::new(Duration::from_millis(100)).unwrap());
    let operation = owner.context().clone();
    let cancelling = Arc::clone(&owner);
    let calls = AtomicUsize::new(0);
    let result = quota
        .run(&operation, Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                cancelling.cancel();
                std::future::pending::<Result<(), Infallible>>().await
            }
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Admitted {
            work: Err(OperationError::Interrupted(Interruption::Cancelled)),
            ..
        }
    ));
    let later = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<(), Infallible>(()) }
        })
        .await;
    assert!(matches!(later, RunResult::Rejected { index: 0, .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn downstream_retries_do_not_repeat_http_quota_consumption() {
    let quota = Quota::new(memory());
    let policy = policy("owner", 2);
    let checks = [Check::new(subject(1).bind(&policy))];
    let mut attempts = 0;
    let result = quota
        .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
            for _ in 0..3 {
                attempts += 1;
                tokio::task::yield_now().await;
            }
            Ok::<_, Infallible>(())
        })
        .await;
    assert!(matches!(result, RunResult::Admitted { work: Ok(()), .. }));
    assert_eq!(attempts, 3);
    assert!(matches!(
        quota
            .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
                Ok::<_, Infallible>(())
            })
            .await,
        RunResult::Admitted { work: Ok(()), .. }
    ));
    assert!(matches!(
        quota
            .run(&context(100), Checks::new(&checks).unwrap(), |_| async {
                Ok::<_, Infallible>(())
            })
            .await,
        RunResult::Rejected { index: 0, .. }
    ));
}

#[tokio::test]
async fn native_mixed_mode_rejection_is_retained_not_reimplemented() {
    let first = policy("owner", 1);
    let second = policy("peer", 1).with_quota_mode(QuotaMode::Shadow);
    let both = [
        Check::new(subject(1).bind(&first)),
        Check::new(subject(2).bind(&second)),
    ];
    let result = Quota::new(memory())
        .run(&context(100), Checks::new(&both).unwrap(), |_| async {
            Ok::<_, Infallible>(())
        })
        .await;
    assert!(matches!(
        result,
        RunResult::Backend {
            consumption: ConsumptionStatus::NotConsumed,
            ..
        }
    ));
}

#[test]
fn empty_checks_rejected() {
    assert!(Checks::<runlimit_core::FixedWindowPolicy>::new(&[]).is_err());
}

#[cfg(feature = "postgres")]
#[test]
fn native_postgres_certainty_bridge_is_lossless() {
    use batter_runlimit::ConsumptionError;
    use runlimit_postgres::{BatchCheckError, CheckError, CheckPhase};
    // Mirrors the exact pinned native errors.rs classifier; no database is needed.
    // Infer SQLx's error through From<io::Error>, avoiding a test-only SQLx dependency.
    let cases = [
        (
            BatchCheckError::InvalidBatch(runlimit_core::BatchError::BatchTooLarge {
                actual: 2,
                maximum: 1,
            }),
            ConsumptionStatus::NotConsumed,
        ),
        (
            CheckError::DefinitelyNotConsumed(std::io::Error::other("before commit").into()).into(),
            ConsumptionStatus::NotConsumed,
        ),
        (
            CheckError::CommitOutcomeUnknown(std::io::Error::other("lost confirmation").into())
                .into(),
            ConsumptionStatus::PossiblyConsumed,
        ),
        (
            CheckError::TimedOutBeforeCommit {
                phase: CheckPhase::AcquiringConnection,
            }
            .into(),
            ConsumptionStatus::NotConsumed,
        ),
        (
            CheckError::CommitTimedOut.into(),
            ConsumptionStatus::PossiblyConsumed,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.consumption(), expected, "{error:?}");
        assert_eq!(ConsumptionError::consumption(&error), expected, "{error:?}");
    }
    fn postgres_type_identity(
        limiter: runlimit_postgres::PostgresLimiter,
    ) -> Quota<runlimit_postgres::PostgresLimiter> {
        Quota::new(limiter)
    }
    let _ = postgres_type_identity;
}

#[test]
fn negative_control_stacked_native_single_checks_partially_charge_a_denied_request() {
    let store = memory();
    let first = policy("owner", 1);
    let second = policy("peer", 1);
    let owner = Check::new(subject(1).bind(&first));
    let peer = Check::new(subject(2).bind(&second));
    store.check(&peer).unwrap();
    assert!(store.check(&owner).unwrap().permits_request());
    assert!(!store.check(&peer).unwrap().permits_request());
    // Unlike the atomic-batch test, the owner's allowance was lost.
    assert!(!store.check(&owner).unwrap().permits_request());
}

#[test]
fn native_memory_error_certainty_covers_every_pinned_variant() {
    use batter_runlimit::ConsumptionError;
    use runlimit_memory::MemoryBatchError;

    let errors = [
        MemoryBatchError::InvalidBatch(runlimit_core::BatchError::BatchTooLarge {
            actual: 2,
            maximum: 1,
        }),
        MemoryBatchError::BatchExceedsShardCapacity {
            shard_index: 0,
            key_count: 2,
            capacity: 1,
        },
        MemoryBatchError::PoisonedShard(runlimit_memory::PoisonedShardError { shard_index: 0 }),
    ];
    for error in errors {
        assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);
    }
}
