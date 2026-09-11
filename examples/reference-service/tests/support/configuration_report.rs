use batter::{
    settings::SettingsError,
    startup::{StartupCause, StartupError, StartupFailure},
};
use batter_test_support::TestFailure;
use std::sync::Arc;
use tokio::sync::oneshot::error::RecvError;

pub fn check(
    report: Arc<StartupFailure<SettingsError>>,
    acquired: Result<(), RecvError>,
    closed_before_lease: bool,
    no_ready: bool,
) -> Result<(), TestFailure<StartupError<SettingsError>, SettingsError>> {
    let observation = acquired
        .map_err(|error| SettingsError::new("pool", "delivery failed").with_cause(error))
        .and_then(|()| {
            let primary = matches!(&report.cause, StartupCause::Failed(error)
                if std::error::Error::source(error).is_some_and(|cause| cause.is::<std::io::Error>()));
            if report.stage != "later.initialize"
                || !closed_before_lease
                || !no_ready
                || !primary
                || report.cleanup.records.len() != 1
                || !report.cleanup.is_success()
            {
                Err(SettingsError::new("startup", "pool-close evidence mismatch"))
            } else {
                Ok(())
            }
        });
    match observation {
        Ok(()) => Ok(()), // The deliberately injected startup failure is expected.
        Err(cleanup) => Err(TestFailure::Both {
            body: StartupError::Failed(report),
            cleanup,
        }),
    }
}
