use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    time::Duration,
};

use super::*;
use runlimit_core::{
    AdmissionOperation, AdmissionOutcome, Allowance, BatchDecisionView, BatchError, Capacity,
    ConsumptionStatus, DecisionView, Denial, FixedWindowPolicy, MAX_LIMIT, MAX_WINDOW,
    MAX_WINDOW_MILLIS, PolicyId, QuotaDenial, QuotaMode, ScopeId, SubjectKey,
};

fn capacity(value: u64) -> Capacity {
    Capacity::new(value).unwrap()
}

fn policy(id: &str, scope: &str) -> FixedWindowPolicy {
    FixedWindowPolicy::new(
        PolicyId::new(id).expect("valid policy identifier"),
        ScopeId::new(scope).expect("valid scope identifier"),
        10,
        Duration::from_mins(1),
    )
    .expect("valid policy")
}

fn key(byte: u8) -> SubjectKey {
    SubjectKey::from_digest([byte; 32])
}

#[derive(Default)]
struct RecordingObserver {
    events: Mutex<Vec<(usize, AdmissionOutcome, ConsumptionStatus)>>,
    cleanups: Mutex<Vec<(usize, CleanupOutcome)>>,
}

impl Observer for RecordingObserver {
    fn observe(&self, observation: &Observation<'_>) {
        match observation {
            Observation::Admission(admission) => self.events.lock().unwrap().push((
                match admission.operation() {
                    AdmissionOperation::Check { .. } => 1,
                    AdmissionOperation::Batch { batch_size, .. } => batch_size,
                },
                admission.outcome(),
                admission.consumption(),
            )),
            Observation::Cleanup(cleanup) => self
                .cleanups
                .lock()
                .unwrap()
                .push((cleanup.requested(), cleanup.outcome())),
            // The PostgreSQL backend has no local capacity to report.
            Observation::Capacity(_) => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordedOperation {
    Check,
    Batch { batch_size: usize, has_policy: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecordedAdmissionMetadata {
    operation: RecordedOperation,
    consumption: ConsumptionStatus,
}

#[derive(Default)]
struct AdmissionMetadataObserver {
    admissions: Mutex<Vec<RecordedAdmissionMetadata>>,
}

impl Observer for AdmissionMetadataObserver {
    fn observe(&self, observation: &Observation<'_>) {
        let Observation::Admission(admission) = observation else {
            return;
        };
        self.admissions
            .lock()
            .unwrap()
            .push(RecordedAdmissionMetadata {
                operation: match admission.operation() {
                    AdmissionOperation::Check { .. } => RecordedOperation::Check,
                    AdmissionOperation::Batch { batch_size, policy } => RecordedOperation::Batch {
                        batch_size,
                        has_policy: policy.is_some(),
                    },
                },
                consumption: admission.consumption(),
            });
    }
}

struct PanickingObserver;

impl Observer for PanickingObserver {
    fn observe(&self, _: &Observation<'_>) {
        panic!("injected observer panic");
    }
}

#[test]
fn portable_core_boundaries_fit_database_values_exactly() {
    let policy = FixedWindowPolicy::new(
        PolicyId::new("portable.maximum").unwrap(),
        ScopeId::new("subject").unwrap(),
        MAX_LIMIT,
        MAX_WINDOW,
    )
    .unwrap();
    let check = Check::new(key(1).bind(&policy))
        .with_cost(MAX_LIMIT)
        .unwrap();

    assert_eq!(database_integer(policy.limit().get()), i64::MAX);
    assert_eq!(database_integer(check.cost()), i64::MAX);
    let interval = PgInterval::try_from(policy.window().duration()).unwrap();
    assert_eq!(interval.months, 0);
    assert_eq!(interval.days, 0);
    assert_eq!(
        interval.microseconds,
        i64::try_from(MAX_WINDOW_MILLIS * 1_000).unwrap()
    );
}

#[test]
fn row_lock_permutation_orders_complete_counter_keys() {
    let alpha_client = policy("alpha", "client");
    let alpha_identity = policy("alpha", "identity");
    let beta_client = policy("beta", "client");
    let checks = [
        Check::new(key(0).bind(&beta_client)),
        Check::new(key(0).bind(&alpha_identity)),
        Check::new(key(1).bind(&alpha_client)),
        Check::new(key(0).bind(&alpha_client)),
    ];

    let input = BatchSqlInput::from_checks(&checks);
    let locked_keys = input
        .lock_input_positions
        .iter()
        .map(|position| checks[usize::try_from(*position - 1).unwrap()].counter_key())
        .collect::<Vec<_>>();

    assert!(locked_keys.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn advisory_lock_protocol_has_a_golden_vector() {
    let policy = policy("login", "client");
    let check = Check::new(key(7).bind(&policy));

    assert_eq!(
        advisory_lock_id(check.counter_key()),
        4_549_358_434_381_602_087
    );
}

#[test]
fn capacity_shard_protocol_has_a_golden_vector() {
    let policy = policy("login", "client");
    let check = Check::new(key(7).bind(&policy));

    assert_eq!(capacity_shard(check.counter_key()), 141);
}

#[tokio::test]
async fn observer_reports_connection_free_operations_and_isolates_panics() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
        .expect("syntactically valid database URL");
    let observer = Arc::new(RecordingObserver::default());
    let limiter = PostgresLimiter::new(pool.clone()).with_observer(observer.clone());

    assert!(matches!(
        limiter.check_all(&[]).await,
        Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
    ));
    assert_eq!(limiter.cleanup_expired(0).await.unwrap(), 0);
    assert_eq!(
        observer.events.lock().unwrap().as_slice(),
        [(0, AdmissionOutcome::Failed, ConsumptionStatus::NotConsumed,)]
    );
    assert_eq!(
        observer.cleanups.lock().unwrap().as_slice(),
        [(0, CleanupOutcome::Confirmed { removed: 0 })]
    );

    let panicking = PostgresLimiter::new(pool).with_observer(Arc::new(PanickingObserver));
    assert!(matches!(
        panicking.check_all(&[]).await,
        Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
    ));
    assert_eq!(panicking.cleanup_expired(0).await.unwrap(), 0);
}

#[tokio::test]
async fn failed_postgres_batches_preserve_one_item_policy_metadata() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
        .expect("syntactically valid database URL");
    let observer = Arc::new(AdmissionMetadataObserver::default());
    let limiter = PostgresLimiter::new(pool).with_observer(observer.clone());
    let first_policy = policy("observed.failure.first", "client");
    let second_policy = policy("observed.failure.second", "client");
    let first = Check::new(key(1).bind(&first_policy));
    let second = Check::new(key(2).bind(&second_policy));
    let elapsed = Duration::from_millis(5);

    limiter.observe_single_admission(&first, &Err(CheckError::CommitTimedOut), elapsed);
    limiter.observe_batch_admission(
        &[first],
        &Err(BatchCheckError::Check(CheckError::CommitTimedOut)),
        elapsed,
    );
    limiter.observe_batch_admission(
        &[first, second],
        &Err(BatchCheckError::Check(CheckError::CommitTimedOut)),
        elapsed,
    );

    assert_eq!(
        observer.admissions.lock().unwrap().as_slice(),
        [
            RecordedAdmissionMetadata {
                operation: RecordedOperation::Check,
                consumption: ConsumptionStatus::PossiblyConsumed,
            },
            RecordedAdmissionMetadata {
                operation: RecordedOperation::Batch {
                    batch_size: 1,
                    has_policy: true,
                },
                consumption: ConsumptionStatus::PossiblyConsumed,
            },
            RecordedAdmissionMetadata {
                operation: RecordedOperation::Batch {
                    batch_size: 2,
                    has_policy: false,
                },
                consumption: ConsumptionStatus::PossiblyConsumed,
            },
        ]
    );
}

#[tokio::test]
async fn cleanup_observations_map_every_backend_effect_class() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
        .expect("syntactically valid database URL");
    let observer = Arc::new(RecordingObserver::default());
    let limiter = PostgresLimiter::new(pool).with_observer(observer.clone());
    let elapsed = Duration::from_millis(5);

    limiter.observe_cleanup(7, &Ok(3), elapsed);
    limiter.observe_cleanup(
        7,
        &Err(MaintenanceError::Database(sqlx::Error::RowNotFound)),
        elapsed,
    );
    limiter.observe_cleanup(
        7,
        &Err(MaintenanceError::TimedOutBeforeCommit {
            phase: CleanupPhase::DeletingExpiredWindows,
        }),
        elapsed,
    );
    limiter.observe_cleanup(
        7,
        &Err(MaintenanceError::CommitOutcomeUnknown(
            sqlx::Error::RowNotFound,
        )),
        elapsed,
    );
    limiter.observe_cleanup(7, &Err(MaintenanceError::CommitTimedOut), elapsed);

    assert_eq!(
        observer.cleanups.lock().unwrap().as_slice(),
        [
            (7, CleanupOutcome::Confirmed { removed: 3 }),
            (7, CleanupOutcome::NoEffect),
            (7, CleanupOutcome::NoEffect),
            (7, CleanupOutcome::Unknown),
            (7, CleanupOutcome::Unknown),
        ]
    );
}

#[test]
fn response_metadata_subtracts_only_authoritative_elapsed_time() {
    let allowed = PendingAllowance::new(capacity(10), 6, Duration::from_millis(250))
        .unwrap()
        .finish(Duration::from_millis(80));
    let denied = PendingDenial::Quota(PendingQuotaDenial {
        limit: capacity(10),
        retry_from_sample: Duration::from_millis(150),
    })
    .finish(Duration::from_millis(80));

    assert_eq!(
        allowed,
        Allowance::new(capacity(10), 4, Duration::from_millis(170)).unwrap()
    );
    assert_eq!(
        denied,
        Denial::QuotaExceeded(QuotaDenial::new(capacity(10), Duration::from_millis(70)))
    );
}

#[test]
fn stored_usage_above_the_limit_cannot_construct_a_pending_allowance() {
    let malformed = PendingAllowance::new(capacity(10), 11, Duration::from_millis(250))
        .expect_err("usage above the limit cannot become pending allowance state");

    assert!(matches!(malformed, CheckError::StorageInvariant(_)));
    assert_eq!(malformed.consumption(), ConsumptionStatus::NotConsumed);
}

#[tokio::test]
async fn denied_decision_survives_rollback_failure_and_discards_connection() {
    let quota = QuotaDenial::new(capacity(10), Duration::from_millis(130));
    let outcome = finish_denied_transaction(
        Instant::now() + Duration::from_secs(1),
        BatchDecision::denied(2, 3, quota).unwrap(),
        std::future::ready(Err(sqlx::Error::Protocol(
            "injected rollback failure".to_owned(),
        ))),
    )
    .await;

    let ConnectionOutcome::MustClose(decision) = outcome else {
        panic!("rollback failure must close the connection")
    };
    assert_eq!(
        decision.view(),
        BatchDecisionView::Denied {
            index: 2,
            batch_size: NonZeroUsize::new(3).unwrap(),
            denial: Denial::QuotaExceeded(quota),
        }
    );
}

#[tokio::test]
async fn denied_decision_survives_rollback_deadline_and_discards_connection() {
    let quota = QuotaDenial::new(capacity(1), Duration::from_millis(50));
    let outcome = finish_denied_transaction(
        Instant::now(),
        Decision::denied(quota),
        std::future::pending(),
    )
    .await;

    let ConnectionOutcome::MustClose(decision) = outcome else {
        panic!("rollback deadline must close the connection")
    };
    assert_eq!(
        decision.view(),
        DecisionView::Denied {
            denial: Denial::QuotaExceeded(quota),
        }
    );
}

#[tokio::test]
async fn shadow_quota_denial_is_reported_after_rollback() {
    let quota = QuotaDenial::new(capacity(1), Duration::from_secs(1));
    let outcome = finish_denied_transaction(
        Instant::now() + Duration::from_secs(1),
        Decision::shadow_denied(quota),
        std::future::ready(Ok(())),
    )
    .await;

    let ConnectionOutcome::Reusable(decision) = outcome else {
        panic!("successful rollback must reuse the connection")
    };
    assert_eq!(
        decision.view(),
        DecisionView::ShadowDenied { denial: quota }
    );
}

#[test]
fn single_decisions_are_shaped_before_the_transaction_is_finalized() {
    let allowance = Allowance::new(capacity(10), 4, Duration::from_secs(1)).unwrap();
    let quota = QuotaDenial::new(capacity(10), Duration::from_secs(1));

    assert_eq!(
        single_decision(Admission::Allowed(vec![allowance])).unwrap(),
        Decision::allowed(allowance)
    );
    assert_eq!(
        single_decision(Admission::Denied {
            index: 0,
            denial: Denial::QuotaExceeded(quota),
        })
        .unwrap(),
        Decision::denied(quota)
    );
    assert_eq!(
        single_decision(Admission::ShadowDenied {
            index: 0,
            denial: quota,
        })
        .unwrap(),
        Decision::shadow_denied(quota)
    );

    for malformed in [
        single_decision(Admission::Allowed(vec![allowance, allowance])),
        single_decision(Admission::Denied {
            index: 1,
            denial: Denial::QuotaExceeded(quota),
        }),
        single_decision(Admission::ShadowDenied {
            index: 1,
            denial: quota,
        }),
    ] {
        let error = malformed.expect_err("a malformed single response is a failure");
        assert!(matches!(error, CheckError::StorageInvariant(_)));
        assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);
    }
}

#[test]
fn batch_decisions_are_shaped_against_the_submitted_batch() {
    let allowance = Allowance::new(capacity(10), 4, Duration::from_secs(1)).unwrap();
    let quota = QuotaDenial::new(capacity(10), Duration::from_secs(1));

    assert_eq!(
        batch_decision(2, Admission::Allowed(vec![allowance, allowance])).unwrap(),
        BatchDecision::allowed(vec![allowance, allowance]).unwrap()
    );
    assert!(matches!(
        batch_decision(2, Admission::Allowed(vec![allowance])),
        Err(CheckError::StorageInvariant(_))
    ));
    assert_eq!(
        batch_decision(
            2,
            Admission::Denied {
                index: 1,
                denial: Denial::QuotaExceeded(quota),
            }
        )
        .unwrap(),
        BatchDecision::denied(1, 2, quota).unwrap()
    );
    assert_eq!(
        batch_decision(
            2,
            Admission::ShadowDenied {
                index: 1,
                denial: quota,
            }
        )
        .unwrap(),
        BatchDecision::shadow_denied(1, 2, quota).unwrap()
    );
    let out_of_range = batch_decision(
        2,
        Admission::Denied {
            index: 2,
            denial: Denial::QuotaExceeded(quota),
        },
    )
    .expect_err("a denial outside the batch is a failure");
    assert!(matches!(out_of_range, CheckError::StorageInvariant(_)));
}

#[test]
fn configuration_bounds_batches_and_deadlines() {
    let defaults = PostgresConfig::new();
    assert_eq!(defaults.maximum_rows_per_shard(), 4_096);
    assert_eq!(defaults.max_batch_size(), 32);
    assert_eq!(defaults.pool_acquire_timeout(), Duration::from_secs(3));
    assert_eq!(defaults.operation_timeout(), Duration::from_secs(3));
    assert_eq!(
        defaults.with_maximum_rows_per_shard(0),
        Err(PostgresConfigError::ZeroMaximumRowsPerShard)
    );
    assert_eq!(
        defaults.with_maximum_rows_per_shard(HARD_MAX_ROWS_PER_SHARD + 1),
        Err(PostgresConfigError::MaximumRowsPerShardTooLarge {
            actual: HARD_MAX_ROWS_PER_SHARD + 1,
            maximum: HARD_MAX_ROWS_PER_SHARD,
        })
    );
    assert_eq!(
        defaults.with_max_batch_size(0),
        Err(PostgresConfigError::ZeroBatchSize)
    );
    assert_eq!(
        defaults.with_pool_acquire_timeout(Duration::ZERO),
        Err(PostgresConfigError::ZeroPoolAcquireTimeout)
    );
    assert_eq!(
        defaults.with_pool_acquire_timeout(Duration::from_secs(61)),
        Err(PostgresConfigError::PoolAcquireTimeoutTooLong {
            actual: Duration::from_secs(61),
            maximum: Duration::from_mins(1),
        })
    );
    assert_eq!(
        defaults.with_operation_timeout(Duration::ZERO),
        Err(PostgresConfigError::ZeroOperationTimeout)
    );
    assert_eq!(
        defaults.with_operation_timeout(Duration::from_secs(61)),
        Err(PostgresConfigError::OperationTimeoutTooLong {
            actual: Duration::from_secs(61),
            maximum: Duration::from_mins(1),
        })
    );
}

#[tokio::test]
async fn duplicate_storage_keys_fail_before_connecting() {
    let alpha = policy("alpha", "client");
    let beta = policy("beta", "client");
    let checks = [
        Check::new(key(7).bind(&beta)),
        Check::new(key(7).bind(&beta)),
        Check::new(key(8).bind(&alpha)),
        Check::new(key(8).bind(&alpha)),
    ];
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
        .expect("syntactically valid database URL");
    let limiter = PostgresLimiter::new(pool);

    let error = limiter
        .check_all(&checks)
        .await
        .expect_err("duplicate keys must be rejected");

    assert!(matches!(
        error,
        BatchCheckError::InvalidBatch(BatchError::DuplicateKey {
            first_index: 0,
            duplicate_index: 1,
        })
    ));
}

#[tokio::test]
async fn oversized_batch_fails_before_connecting() {
    let first_policy = policy("login", "client");
    let second_policy = policy("signup", "client");
    let checks = [
        Check::new(key(1).bind(&first_policy)),
        Check::new(key(2).bind(&second_policy)),
    ];
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
        .expect("syntactically valid database URL");
    let config = PostgresConfig::new()
        .with_max_batch_size(1)
        .expect("one is a valid batch limit");
    let limiter = PostgresLimiter::new(pool).with_config(config);

    let error = limiter
        .check_all(&checks)
        .await
        .expect_err("oversized batch must be rejected");

    assert!(matches!(
        error,
        BatchCheckError::InvalidBatch(BatchError::BatchTooLarge {
            actual: 2,
            maximum: 1,
        })
    ));
}

#[tokio::test]
async fn mixed_quota_modes_fail_before_connecting() {
    let enforced = policy("enforced", "client");
    let shadow = policy("shadow", "client").with_quota_mode(QuotaMode::Shadow);
    let checks = [
        Check::new(key(1).bind(&enforced)),
        Check::new(key(2).bind(&shadow)),
    ];
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
        .expect("syntactically valid database URL");
    let limiter = PostgresLimiter::new(pool);

    let error = limiter
        .check_all(&checks)
        .await
        .expect_err("mixed quota modes must be rejected");

    assert!(matches!(
        error,
        BatchCheckError::InvalidBatch(BatchError::MixedQuotaModes {
            first: QuotaMode::Enforce,
            index: 1,
            actual: QuotaMode::Shadow,
        })
    ));
}

#[test]
fn error_reports_consumption_certainty() {
    let definite = CheckError::DefinitelyNotConsumed(sqlx::Error::RowNotFound);
    let unknown = CheckError::CommitOutcomeUnknown(sqlx::Error::RowNotFound);
    let timed_out = CheckError::TimedOutBeforeCommit {
        phase: CheckPhase::AcquiringCounterRowLocks,
    };
    let invariant = CheckError::storage_invariant("injected");
    let decode_invariant =
        CheckError::storage_decode_invariant("decode injected", sqlx::Error::RowNotFound);
    let cleanup_definite = MaintenanceError::Database(sqlx::Error::RowNotFound);
    let cleanup_unknown = MaintenanceError::CommitOutcomeUnknown(sqlx::Error::RowNotFound);

    assert_eq!(definite.consumption(), ConsumptionStatus::NotConsumed);
    assert_eq!(timed_out.consumption(), ConsumptionStatus::NotConsumed);
    assert_eq!(invariant.consumption(), ConsumptionStatus::NotConsumed);
    assert_eq!(
        decode_invariant.consumption(),
        ConsumptionStatus::NotConsumed
    );
    assert_eq!(unknown.consumption(), ConsumptionStatus::PossiblyConsumed);
    assert_eq!(
        CheckError::CommitTimedOut.consumption(),
        ConsumptionStatus::PossiblyConsumed
    );
    assert_eq!(
        BatchCheckError::InvalidBatch(BatchError::EmptyBatch).consumption(),
        ConsumptionStatus::NotConsumed
    );
    assert_eq!(
        BatchCheckError::Check(CheckError::CommitTimedOut).consumption(),
        ConsumptionStatus::PossiblyConsumed
    );
    assert_eq!(
        timed_out.to_string(),
        "PostgreSQL rate-limit check timed out while acquiring counter row lock; quota was \
         not consumed"
    );
    let CheckError::StorageInvariant(semantic) = &invariant else {
        panic!("expected a storage invariant");
    };
    assert_eq!(semantic.detail(), "injected");
    assert!(std::error::Error::source(semantic).is_none());

    let CheckError::StorageInvariant(decode) = &decode_invariant else {
        panic!("expected a decode storage invariant");
    };
    assert_eq!(decode.detail(), "decode injected");
    assert!(
        std::error::Error::source(decode)
            .is_some_and(|source| source.downcast_ref::<sqlx::Error>().is_some())
    );
    assert!(!cleanup_definite.may_have_removed_rows());
    assert!(cleanup_unknown.may_have_removed_rows());
    assert!(MaintenanceError::CommitTimedOut.may_have_removed_rows());
}

#[test]
fn cleanup_query_is_bounded_and_nonblocking() {
    assert_eq!(
        CLEANUP_SQL.matches("pg_catalog.clock_timestamp()").count(),
        1
    );
    assert!(CLEANUP_SQL.contains("AS MATERIALIZED"));
    assert!(CLEANUP_SQL.contains("SELECT sampled_at\n        FROM authoritative_time"));
    assert!(!CLEANUP_SQL.contains("window_expires_at <= pg_catalog.clock_timestamp()"));
    assert!(CLEANUP_SQL.contains("LIMIT $1"));
    assert!(CLEANUP_SQL.contains("FOR UPDATE SKIP LOCKED"));
    assert!(CLEANUP_SQL.contains("ORDER BY capacity.capacity_shard"));
    assert!(CLEANUP_SQL.contains("FOR UPDATE OF capacity"));
}

#[test]
fn every_production_clock_call_is_catalog_qualified() {
    for (name, sql) in [
        ("batch preflight", BATCH_PREFLIGHT_SQL),
        ("batch upsert", BATCH_UPSERT_SQL),
        ("cleanup", CLEANUP_SQL),
    ] {
        let without_qualified_calls = sql.replace("pg_catalog.clock_timestamp()", "");
        assert!(
            !without_qualified_calls.contains("clock_timestamp()"),
            "{name} contains a search_path-resolved clock_timestamp() call"
        );
    }
}
