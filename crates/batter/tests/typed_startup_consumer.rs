//! External-style protected startup with an async initializer, typed output and
//! anyhow-style errors, through the facade only.

use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationOwner,
    startup::{ApplicationError, InitializationError, Startup, StartupCause, StartupError},
};
use std::{net::SocketAddr, time::Duration};

fn budgets() -> (ShutdownBudget, CleanupBudget) {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    (
        ShutdownBudget::new(second, second, second, cleanup).unwrap(),
        cleanup,
    )
}

fn settings(valid: Validation) -> anyhow::Result<&'static str> {
    let valid = valid == Validation::Valid;
    if valid {
        Ok("127.0.0.1:0")
    } else {
        Err(std::io::Error::other("private-settings-marker").into())
    }
}

/// An anyhow-style root: `?` on anyhow, std and Batter errors inside the
/// initializer, and `?` on the startup failure at the boundary, with no adapters.
async fn serve(valid: Validation) -> anyhow::Result<SocketAddr> {
    let (shutdown, cleanup) = budgets();
    let context = OperationOwner::new(Duration::from_secs(2))?.into_context();
    let mut starting = Startup::protected(
        Supervisor::new(shutdown),
        context,
        cleanup,
        async move |scope| {
            scope.stage("settings.read")?;
            let bind: SocketAddr = settings(valid)?.parse()?;
            scope.stage("listener.bind")?;
            let listener = tokio::net::TcpListener::bind(bind).await?;
            let address = listener.local_addr()?;
            scope
                .registration()
                .register("listener", move |startup| async move {
                    let shutdown = startup.acknowledge_started();
                    shutdown.draining().await;
                    drop(listener);
                    Ok(shutdown.stopped())
                })?;
            Ok::<_, ApplicationError>(address)
        },
    )
    .start();
    let (running, address) = starting.wait_with_output().await?;
    running.status().wait_ready().await.unwrap();
    running.shutdown_checked().await?;
    Ok(address)
}

#[tokio::test]
async fn anyhow_style_initializer_returns_its_bound_address_without_adapters() {
    let address = serve(Validation::Valid).await.unwrap();
    assert!(address.ip().is_loopback());
    assert_ne!(address.port(), 0);
}

#[tokio::test]
async fn anyhow_boundary_retains_the_protected_report_and_the_converted_cause() {
    let error = serve(Validation::Invalid).await.unwrap_err();
    assert!(!format!("{error}").contains("private-settings-marker"));
    let StartupError::Failed(report) = error
        .downcast_ref::<StartupError<InitializationError<ApplicationError>>>()
        .expect("the protected startup error crosses the anyhow boundary")
    else {
        panic!("missing retained startup report")
    };
    assert_eq!(report.stage, "settings.read");
    assert!(report.cleanup.is_success());
    let StartupCause::Failed(InitializationError::Application(application)) = &report.cause else {
        panic!("missing application failure")
    };
    // anyhow's own `Into<BoxError>` conversion keeps its message for trusted
    // inspection but wraps the head `io::Error`, so it is not a direct downcast.
    assert_eq!(application.get_ref().to_string(), "private-settings-marker");
    assert!(application.downcast_ref::<std::io::Error>().is_none());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Validation {
    Valid,
    Invalid,
}
