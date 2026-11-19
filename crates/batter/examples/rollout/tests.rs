//! The example's declared mode, run whole.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]

use batter::settings::SettingsSource;

use super::{resolve, run_declared, today};

#[test]
fn the_catalog_resolves_and_today_is_a_civil_date() {
    let settings = SettingsSource::from_pairs([("FLAG_NEW_CHECKOUT".into(), "on".into())]).unwrap();
    let (resolved, _) = resolve(&settings, today().unwrap()).unwrap();
    assert!(resolved.report().to_string().contains("new_checkout=on"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_engaged_switch_stops_maintenance_and_a_released_one_lets_it_run() {
    let engaged =
        SettingsSource::from_pairs([("FLAG_OUTBOUND_HALT".into(), "engaged".into())]).unwrap();
    assert_eq!(run_declared(&engaged).await.unwrap(), 0);
    let released = SettingsSource::from_pairs([]).unwrap();
    assert!(run_declared(&released).await.unwrap() >= 2);
}

#[tokio::test]
async fn a_component_failure_before_readiness_retains_its_shutdown_and_cleanup() {
    use batter::lifecycle::ShutdownFailure;
    use batter::service::ServiceOutcome;
    let failure = super::serve_for(
        async |scope: &mut batter::startup::ProtectedStartupScope| {
            scope
                .reserve_cleanup("stop")?
                .register(|| async { Err("stop marker failed".into()) });
            scope
                .registration()
                .register("early", |_| async { Err("initialization failed".into()) })?;
            Ok(())
        },
        super::SECOND,
        |()| panic!("serving started before readiness"),
    )
    .await
    .unwrap_err();
    let completion = &failure.downcast_ref::<super::ServiceFailure>().unwrap().0;
    assert!(completion.readiness().is_err(), "readiness failed");
    let ServiceOutcome::Shutdown(Err(ShutdownFailure::Report(report))) = completion.service()
    else {
        panic!("missing shutdown report")
    };
    assert!(
        report
            .tasks
            .iter()
            .any(|task| task.name == "early" && task.error.is_some()),
        "component failure was retained"
    );
    assert!(
        report
            .cleanup
            .records
            .iter()
            .any(|hook| hook.name == "stop" && hook.error.is_some()),
        "cleanup was awaited and its failure retained"
    );
}

#[tokio::test]
async fn interrupted_migration_finishes_pool_cleanup() {
    use batter::command::{CommandCause, CommandFailure};
    let stalled = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!(
        "postgres://serving@{}/flags?sslmode=disable",
        stalled.local_addr().unwrap()
    );
    let owner = super::migration(&url).unwrap().start_without_signals();
    let (_peer, _) = tokio::time::timeout(super::SECOND * 5, stalled.accept())
        .await
        .unwrap()
        .unwrap();
    owner.cancel();
    let Err(CommandFailure::Report(report)) = owner.finish().await else {
        panic!("expected interruption")
    };
    assert!(
        matches!(report.work, Err(CommandCause::Interrupted(_))),
        "interruption retained"
    );
    let cleanup = report.cleanup.as_ref().unwrap();
    assert!(
        cleanup.is_success() && cleanup.records.iter().any(|r| r.name == "migration.pool"),
        "pool closed before completion"
    );
}
