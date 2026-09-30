use super::*;
use crate::single_facade_completion::{WorkAndShutdownFailure, complete};
use batter::{
    lifecycle::{ShutdownFailure, Supervisor},
    operation::{Interruption, OperationError},
    runlimit::{
        Admission, InterruptedCheck,
        native::{Capacity, ConsumptionStatus},
        postgres::{CheckError, PostgresLimiter},
    },
};
use std::time::Duration;

// Run every failure through both application expectations, not only the first.
fn expectations() -> [fn(QuotaOutcome) -> Outcome; 2] {
    [expect_admission, |result| {
        expect_exhaustion(result).map(|_| ())
    }]
}

#[test]
fn both_expectations_retain_backend_cause_and_consumption_without_formatting_them() {
    for expect in expectations() {
        for uncertain in [false, true] {
            let cause = sqlx::Error::Protocol("private-database-marker".into());
            let error = BatchCheckError::Check(if uncertain {
                CheckError::CommitOutcomeUnknown(cause)
            } else {
                CheckError::DefinitelyNotConsumed(cause)
            });
            let consumption = error.consumption();
            let failure = expect(RunResult::Backend { error, consumption }).unwrap_err();
            let retained = failure.downcast_ref::<UnexpectedQuotaOutcome>().unwrap();
            let RunResult::Backend {
                error,
                consumption: actual,
            } = &retained.result
            else {
                panic!("lost the native backend outcome")
            };
            assert_eq!(*actual, consumption);
            let cause = match (uncertain, error) {
                (true, BatchCheckError::Check(CheckError::CommitOutcomeUnknown(cause)))
                | (false, BatchCheckError::Check(CheckError::DefinitelyNotConsumed(cause))) => {
                    cause
                }
                _ => panic!("changed the native failure classification"),
            };
            assert!(
                matches!(cause, sqlx::Error::Protocol(message) if message == "private-database-marker")
            );
            assert!(retained.source().unwrap().is::<BatchCheckError>());
            assert!(!format!("{failure:?} {failure}").contains("private-database-marker"));
        }
    }
}

#[test]
fn both_expectations_retain_interruption_reason_and_check_progress() {
    for expect in expectations() {
        for reason in [Interruption::Cancelled, Interruption::DeadlineExceeded] {
            for check in [InterruptedCheck::NotStarted, InterruptedCheck::InFlight] {
                let failure = expect(RunResult::Interrupted { reason, check }).unwrap_err();
                let retained = failure.downcast_ref::<UnexpectedQuotaOutcome>().unwrap();
                assert!(matches!(retained.result, RunResult::Interrupted {
                    reason: actual_reason, check: actual_check,
                } if actual_reason == reason && actual_check == check));
            }
        }
    }
}

#[test]
fn interrupted_admitted_work_keeps_its_admission_evidence() {
    for expect in expectations() {
        let denial = QuotaDenial::new(Capacity::new(1).unwrap(), Duration::from_secs(17));
        let failure = expect(RunResult::Admitted {
            admission: Admission::ShadowDenied {
                index: 0,
                batch_size: 1.try_into().unwrap(),
                denial,
            },
            work: Err(OperationError::Interrupted(Interruption::DeadlineExceeded)),
        })
        .unwrap_err();
        let retained = failure.downcast_ref::<UnexpectedQuotaOutcome>().unwrap();
        let RunResult::Admitted {
            admission: Admission::ShadowDenied { denial: actual, .. },
            work: Err(OperationError::Interrupted(Interruption::DeadlineExceeded)),
        } = &retained.result
        else {
            panic!("lost admission or work interruption evidence")
        };
        assert_eq!(*actual, denial);
    }
}

#[tokio::test]
async fn closed_pool_failure_survives_checked_shutdown_with_and_without_cleanup_failure() {
    for cleanup_fails in [false, true] {
        // Closing a lazy pool makes acquisition fail without a server or timing race.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        pool.close().await;
        let mut supervisor = Supervisor::new(crate::budget().unwrap());
        supervisor
            .register("worker", |startup| async {
                let shutdown = startup.acknowledge_started();
                shutdown.draining().await;
                Ok(shutdown.stopped())
            })
            .unwrap();
        supervisor
            .on_cleanup("pool", move || async move {
                if cleanup_fails {
                    Err(std::io::Error::other("cleanup-marker").into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
        let running = supervisor.start();
        running.status().wait_ready().await.unwrap();
        let failure = complete(
            running,
            crate::run_limiter(PostgresLimiter::new(pool), "owner"),
        )
        .await
        .unwrap_err();
        let retained = if cleanup_fails {
            let both = failure.downcast_ref::<WorkAndShutdownFailure>().unwrap();
            let ShutdownFailure::Report(report) = &both.shutdown else {
                panic!("lost cleanup report")
            };
            assert_eq!(
                report.cleanup.records[0]
                    .error
                    .as_ref()
                    .unwrap()
                    .to_string(),
                "cleanup-marker"
            );
            both.work.downcast_ref::<UnexpectedQuotaOutcome>().unwrap()
        } else {
            failure.downcast_ref::<UnexpectedQuotaOutcome>().unwrap()
        };
        assert!(matches!(
            retained.result,
            RunResult::Backend {
                error: BatchCheckError::Check(CheckError::DefinitelyNotConsumed(
                    sqlx::Error::PoolClosed
                )),
                consumption: ConsumptionStatus::NotConsumed,
            }
        ));
        assert!(!format!("{failure:?} {failure}").contains("cleanup-marker"));
    }
}
