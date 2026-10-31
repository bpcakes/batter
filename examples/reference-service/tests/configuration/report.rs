use crate::configuration::report::configuration_report::PoolClosure;
use crate::configuration::report::configuration_report::ReadinessObservation;

#[path = "../support/configuration_report.rs"]
mod configuration_report;

use batter::test_support::TestFailure;
use batter::{
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{ShutdownBudget, Supervisor},
    settings::SettingsError,
    startup::{Startup, StartupCause, StartupError},
};
use std::{error::Error, io, sync::Arc, time::Duration};

#[tokio::test]
async fn evidence_failures_retain_startup_cleanup_and_delivery_causes() {
    for (cleanup_fails, delivery_fails) in [(true, false), (false, true), (false, false)] {
        let second = Duration::from_secs(1);
        let cleanup = CleanupBudget::new(second, second, second).unwrap();
        let supervisor =
            Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
        let mut starting = Startup::new(
            supervisor,
            batter::operation::OperationOwner::new(second)
                .unwrap()
                .into_context(),
            cleanup,
            move |scope| {
                Box::pin(async move {
                    scope.stage("later.initialize").unwrap();
                    scope
                        .supervisor()
                        .on_cleanup("pool.close", move || async move {
                            if cleanup_fails {
                                Err(io::Error::other("cleanup-secret-marker").into())
                            } else {
                                Ok(())
                            }
                        })
                        .unwrap();
                    Err::<(), _>(
                        SettingsError::new("later", "initialization failed")
                            .with_cause(io::Error::other("startup-secret-marker")),
                    )
                })
            },
        )
        .start();
        let Err(StartupError::Failed(report)) = starting.wait().await else {
            panic!("expected startup failure report");
        };
        let (send, receive) = tokio::sync::oneshot::channel();
        if delivery_fails {
            drop(send);
        } else {
            send.send(()).unwrap();
        }
        let checked = configuration_report::check(
            report.clone(),
            receive.await,
            PoolClosure::Closed,
            ReadinessObservation::Unready,
        );
        if !cleanup_fails && !delivery_fails {
            checked.unwrap();
            assert_invalid_evidence(&report);
            continue;
        }
        let error = checked.unwrap_err();
        assert!(!format!("{error} {error:#?}").contains("secret-marker"));
        let TestFailure::Both {
            body: StartupError::Failed(retained),
            cleanup: observation,
        } = error
        else {
            panic!("report and observation must both survive");
        };
        assert!(Arc::ptr_eq(&retained, &report));
        let StartupCause::Failed(primary) = &retained.cause else {
            panic!("missing primary cause");
        };
        assert_eq!(
            primary
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .to_string(),
            "startup-secret-marker"
        );
        if cleanup_fails {
            assert_eq!(retained.cleanup.records[0].outcome, CleanupOutcome::Failed);
            assert_eq!(
                retained.cleanup.records[0]
                    .error
                    .as_ref()
                    .unwrap()
                    .downcast_ref::<io::Error>()
                    .unwrap()
                    .to_string(),
                "cleanup-secret-marker"
            );
        }
        if delivery_fails {
            assert!(
                observation
                    .source()
                    .unwrap()
                    .is::<tokio::sync::oneshot::error::RecvError>()
            );
        }
    }
}

fn assert_invalid_evidence(report: &Arc<batter::startup::StartupFailure<SettingsError>>) {
    for (closure, readiness) in [
        (PoolClosure::Open, ReadinessObservation::Unready),
        (PoolClosure::Closed, ReadinessObservation::Ready),
    ] {
        let error = configuration_report::check(report.clone(), Ok(()), closure, readiness)
            .expect_err("missing closure or readiness evidence must fail");
        let TestFailure::Both {
            body: StartupError::Failed(retained),
            cleanup,
        } = error
        else {
            panic!("startup and evidence failures must both survive");
        };
        assert!(Arc::ptr_eq(&retained, report));
        assert_eq!(cleanup.reason(), "pool-close evidence mismatch");
    }
}
