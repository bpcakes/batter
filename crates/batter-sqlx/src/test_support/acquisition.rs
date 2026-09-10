use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use batter::telemetry::with_current_dispatch;
use postgres_test_harness::{DatabaseTemplate, Error};
use tokio::{sync::oneshot, task::JoinHandle};

use super::{AcquisitionFailure, DatabaseFixture, FixtureError};

pub(super) type Resources = Arc<Mutex<Registered>>;

pub(super) struct Registered {
    pub fixtures: Vec<DatabaseFixture>,
    pub templates: Vec<DatabaseTemplate>,
    producers: Vec<JoinHandle<Result<(), Arc<Error>>>>,
}

impl Registered {
    pub fn new(templates: Vec<DatabaseTemplate>) -> Resources {
        Arc::new(Mutex::new(Self {
            fixtures: Vec::new(),
            templates,
            producers: Vec::new(),
        }))
    }
}

/// Register the producer before yielding its cancellable delivery waiter. Work
/// publishes resource ownership before returning its value. Native failures are
/// shared with the report even when delivery is cancelled or the body handles one.
pub(super) async fn produce<T, F>(resources: &Resources, work: F) -> Result<T, FixtureError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, Error>> + Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    let producer = tokio::spawn(with_current_dispatch(async move {
        match work.await {
            Ok(value) => {
                let _ = sender.send(Ok(value));
                Ok(())
            }
            Err(error) => {
                let error = Arc::new(error);
                let _ = sender.send(Err(error.clone()));
                Err(error)
            }
        }
    }));
    resources
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .producers
        .push(producer);
    receiver
        .await
        .map_err(|_| FixtureError::AcquisitionStopped)?
        .map_err(FixtureError::Acquisition)
}

pub(super) async fn join_producers(resources: &Resources) -> Vec<Result<(), AcquisitionFailure>> {
    let producers = std::mem::take(
        &mut resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .producers,
    );
    futures_util::future::join_all(producers.into_iter().map(|producer| async move {
        producer
            .await
            .map_err(AcquisitionFailure::Task)?
            .map_err(AcquisitionFailure::Native)
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handled_producer_error_still_prevents_a_successful_report() {
        let resources = Registered::new(Vec::new());
        // Synthetic native failure verifies delivery/report identity and the
        // successful-body boundary; live tests cover real initializer failures.
        let result = produce::<(), _>(&resources, async {
            Err(Error::InvalidConfiguration {
                reason: "controlled producer failure",
            })
        })
        .await;
        let Err(FixtureError::Acquisition(delivered)) = result else {
            panic!("native cause required")
        };
        let acquisitions = join_producers(&resources).await;
        assert!(
            matches!(&acquisitions[0], Err(AcquisitionFailure::Native(reported)) if Arc::ptr_eq(&delivered, reported))
        );
        let report = super::super::FixtureReport::<(), ()> {
            body: Ok(()),
            acquisitions,
            databases: Vec::new(),
            drain: Ok(()),
        };
        assert!(!report.is_ok());
        assert!(report.into_result().is_err());
    }

    #[tokio::test]
    async fn producer_panic_remains_joinable_after_delivery_stops() {
        let resources = Registered::new(Vec::new());
        let result = produce::<(), _>(&resources, async {
            panic!("controlled native producer panic");
        })
        .await;
        assert!(matches!(result, Err(FixtureError::AcquisitionStopped)));
        let reports = join_producers(&resources).await;
        assert_eq!(reports.len(), 1);
        assert!(matches!(&reports[0], Err(AcquisitionFailure::Task(error)) if error.is_panic()));
    }
}
