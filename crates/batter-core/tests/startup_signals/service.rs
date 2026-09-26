use super::*;

pub(super) fn run_startup_child() {
    runtime().block_on(async {
        let mut process = supervisor();
        process
            .reserve_cleanup("signals")
            .unwrap()
            .register(|| async {
                println!("startup-cleanup");
                Ok(())
            });
        let mut starting = Startup::scoped(process, context(5), cleanup_budget(), |_scope| {
            Box::pin(std::future::pending::<Result<(), RegistrationError>>())
        })
        .with_unix_signals("signals")
        .start();
        println!("start-returned");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        let report = failure(starting.wait().await);
        assert!(matches!(report.cause, StartupCause::Draining));
        assert!(report.cleanup.is_success());
        println!("startup-draining");
    });
}

pub(super) fn run_service_child() {
    runtime().block_on(async {
        let mut process = supervisor();
        process
            .on_cleanup("resource", || async {
                println!("service-cleaned");
                Ok(())
            })
            .unwrap();
        let specification = Startup::scoped(process, context(5), cleanup_budget(), |_| {
            Box::pin(std::future::pending::<Result<(), RegistrationError>>())
        })
        .with_unix_signals("signals");
        let owner = batter_core::service::start(specification, batter_core::service::NoDiagnostics);
        // No executor yield: the parent sends the signal while this thread is
        // blocked waiting for its acknowledgement byte.
        println!("start-returned");
        std::io::stdout().flush().unwrap();
        wait_for_parent();
        let completion = owner.wait().await;
        let batter_core::service::ServiceOutcome::StartupFailed(StartupError::Failed(report)) =
            completion.service()
        else {
            panic!("expected retained startup drain")
        };
        assert!(matches!(report.cause, StartupCause::Draining));
        assert!(report.cleanup.is_success());
    });
}
