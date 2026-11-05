use super::*;
use batter_core::{operation::OperationOwner, service, startup::Startup};

#[tokio::test(flavor = "current_thread")]
async fn startup_output_stays_guarded_until_claimed_even_after_service_completion() {
    for claim in [false, true] {
        let ambient = Logs::new();
        let _ambient = tracing::dispatcher::set_default(&ambient.dispatch);
        let driver = Logs::new();
        let process = Supervisor::new(shutdown_budget());
        let stop = process.handle();
        let second = Duration::from_secs(1);
        let starting = driver.within("service.output", || {
            let startup = Startup::protected(
                process,
                OperationOwner::new(second).unwrap().into_context(),
                CleanupBudget::new(second, second, second).unwrap(),
                async |scope| {
                    scope.registration().register("worker", |startup| async {
                        let running = startup.acknowledge_started();
                        running.draining().await;
                        Ok(running.stopped())
                    })?;
                    Ok::<_, batter_core::RegistrationError>(DropMarker {
                        name: "service.output.drop",
                        dropped: None,
                    })
                },
            );
            service::start_with_readiness(startup, service::NoDiagnostics, None)
        });
        let observer = starting.observer();
        observer.wait_ready().await.unwrap();
        stop.request();
        assert!(observer.wait().await.readiness().is_ok());
        if claim {
            let (owner, output) = starting.into_ready().await.unwrap();
            drop(owner);
            drop(output);
            assert!(ambient.text().contains("service.output.drop"));
            assert!(!driver.text().contains("service.output.drop"));
        } else {
            drop(starting);
            driver.one_event("service.output.drop", "service.output");
            assert!(!ambient.text().contains("service.output.drop"));
        }
    }
}
