use super::probe::{StartupContract, authorize_witness, receive_expected_witness, startup_catalog};
use super::*;
use runledger_core::jobs::{JobContract, JobFailureKind};

#[test]
fn termination_gate_never_treats_unproven_work_as_stopped() {
    let gate = TerminationGate::new();
    assert_eq!(gate.state(), TerminationState::NotStarted);
    gate.arm().unwrap();
    assert_eq!(gate.state(), TerminationState::Unproven);
    gate.mark_cooperatively_stopped();
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
}

#[test]
fn startup_contract_is_distinct_from_delivery_execution() {
    assert_eq!(
        StartupContract::spec().job_type().as_str(),
        STARTUP_JOB_TYPE
    );
    assert_ne!(STARTUP_JOB_TYPE, crate::delivery::DELIVERY_JOB_TYPE);
}

#[test]
fn probe_catalog_claims_only_the_startup_control_type() {
    let (sender, _receiver) = mpsc::unbounded_channel();
    let witness = Uuid::from_u128(1);
    let catalog = startup_catalog(&StartupContract::spec(), sender, witness).unwrap();
    let registry = catalog.to_registry();
    let registered: Vec<_> = registry
        .registered_types()
        .into_iter()
        .map(|job_type| job_type.as_str())
        .collect();
    assert_eq!(registered, [STARTUP_JOB_TYPE]);
    assert!(!registered.contains(&crate::delivery::DELIVERY_JOB_TYPE));
}

#[test]
fn predecessor_cannot_execute_successor_witness() {
    let failure = authorize_witness(Uuid::from_u128(1), Uuid::from_u128(2)).unwrap_err();
    assert_eq!(failure.kind, JobFailureKind::Retryable);
    assert_eq!(failure.code, "startup.owner_changed");
    assert!(failure.retry_timing().is_some());
}

#[tokio::test]
async fn startup_witness_ignores_stale_control_invocations() {
    let expected_job = Uuid::from_u128(1);
    let expected_witness = Uuid::from_u128(2);
    let (sender, mut receiver) = mpsc::unbounded_channel();
    sender
        .send(StartupInvocation {
            job_id: Uuid::from_u128(3),
            run_number: 1,
            attempt: 1,
            operation_deadline: Instant::now() + Duration::from_secs(1),
            witness_id: Uuid::from_u128(4),
        })
        .unwrap();
    let expected = StartupInvocation {
        job_id: expected_job,
        run_number: 1,
        attempt: 1,
        operation_deadline: Instant::now() + Duration::from_secs(1),
        witness_id: expected_witness,
    };
    sender.send(expected).unwrap();

    assert_eq!(
        receive_expected_witness(
            expected_job,
            expected_witness,
            &mut receiver,
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap(),
        expected
    );
}

#[tokio::test]
async fn overflow_precedes_acquisition_and_cannot_park_cleanup() {
    use batter::{
        cleanup::{CleanupBudget, CleanupStack},
        settings::SettingsSource,
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/unused")
        .unwrap();
    let settings = WorkerSettings::from_source(
        &SettingsSource::from_pairs([("JOBS_WORKER_ID".into(), "overflow".into())]).unwrap(),
    )
    .unwrap();
    let gate = TerminationGate::new();
    let result = prepare_probe_worker(
        &pool,
        &settings,
        gate.clone(),
        &batter::operation::OperationContext::new(Duration::from_secs(35)).unwrap(),
        Duration::MAX,
    )
    .await;
    assert!(
        matches!(result, Err(error) if matches!(error.as_ref(), WorkerStartupError::WitnessBudgetOverflow))
    );
    assert_eq!(pool.size(), 0, "overflow must not open a connection");
    assert_eq!(gate.state(), TerminationState::NotStarted);
    let second = Duration::from_secs(1);
    let cleanup = DependencyCleanup::new(
        CleanupStack::new(),
        CleanupBudget::new(second, second, second).unwrap(),
        gate,
    );
    assert!(
        tokio::time::timeout(second, cleanup.finish())
            .await
            .unwrap()
            .is_success()
    );
}

#[test]
fn release_failure_retains_independent_native_failure() {
    let release = Arc::new(ProbeLeaseRelease {
        unlock: UnlockOutcome::TimedOut,
        close: CloseOutcome::Closed,
    });
    let native = Arc::new(runledger_runtime::Error::from(
        runledger_runtime::RuntimeError::ShutdownTimeout {
            timeout: Duration::from_secs(10),
        },
    ));
    let completion = NativeDriverCompletion::Release {
        release: release.clone(),
        native: Box::new(NativeDriverCompletion::Native(native.clone())),
    };
    let WorkerDriverError::Release {
        release: retained,
        shutdown: Some(shutdown),
    } = classify_driver_completion(completion, true).unwrap_err()
    else {
        panic!("lost release error")
    };
    assert!(Arc::ptr_eq(&release, &retained));
    let WorkerDriverError::Native(retained) = *shutdown else {
        panic!("lost native error")
    };
    assert!(Arc::ptr_eq(&native, &retained));
}
