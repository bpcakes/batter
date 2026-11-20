//! Cache-hit and cache-miss controls for the fixture template entry points,
//! and the pre-provisioning endpoint refusal of every acquisition path.
use batter_sqlx::test_support::{
    ConnectionPlan, FixtureError, FixtureSuite, HarnessEndpointReason, template_spec,
};
use postgres_test_harness::{HarnessConfig, POSTGRES_TEST_ADMIN_URL_ENV, PostgresHarness};
use sqlx::postgres::PgPoolOptions;

fn assert_ambient(error: &FixtureError) {
    let FixtureError::Endpoint(refusal) = error else {
        panic!("expected a pre-lookup endpoint refusal");
    };
    assert_eq!(
        refusal.reason(),
        HarnessEndpointReason::AmbientEnvironment,
        "expected an ambient PostgreSQL setting to be refused",
    );
}

#[tokio::test]
#[ignore = "requires a disposable POSTGRES_TEST_ADMIN_URL and PGHOSTADDR"]
async fn template_environment_is_checked_on_cold_and_warm_lookups()
-> Result<(), postgres_test_harness::BoxError> {
    assert!(
        std::env::var_os("PGHOSTADDR").is_some(),
        "run through the SQLx live runner"
    );
    // This input belongs to the native harness. BATTER_SQLX_ADMIN_URL belongs
    // to SQLx, whose host selection can differ for the same URI.
    let harness = PostgresHarness::start(
        HarnessConfig::new("template_env")?
            .with_admin_database_url(std::env::var(POSTGRES_TEST_ADMIN_URL_ENV)?)
            .with_cleanup_on_start(false),
    )
    .await?;
    let mut suite = FixtureSuite::new(harness.clone());
    // Seed and retain the upstream cache directly. The native harness uses
    // the supplied URL and ignores the SQLx-specific environment guard.
    let warm = template_spec(&[], "warm");
    let retained = harness.template(warm, |_| async { Ok(()) }).await?;
    let specs = [template_spec(&[], "cold"), warm];
    for spec in specs {
        let Err(error) = suite
            .template(spec, |_| async {
                panic!("initializer must not run");
            })
            .await
        else {
            panic!("manual template lookup must refuse the environment");
        };
        assert_ambient(&error);
    }
    // Acquisition is refused before the harness provisions a database.
    let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    let Err(error) = suite.empty(&plan).await else {
        panic!("manual acquisition must refuse the environment");
    };
    assert_ambient(&error);
    let report = suite
        .start(move |scope| {
            Box::pin(async move {
                for spec in specs {
                    let Err(error) = scope
                        .template(spec, |_| async {
                            panic!("initializer must not run");
                        })
                        .await
                    else {
                        panic!("owned template lookup must refuse the environment");
                    };
                    assert_ambient(&error);
                }
                let Err(error) = scope.empty(&plan).await else {
                    panic!("owned acquisition must refuse the environment");
                };
                assert_ambient(&error);
                Ok::<(), FixtureError>(())
            })
        })
        .into_report()
        .await?;
    assert!(report.body.is_ok(), "body must observe both refusals");
    assert!(
        report.acquisitions.is_empty(),
        "a refusal before the lookup or provisioning starts no producer"
    );
    assert!(report.databases.is_empty(), "no database was provisioned");
    drop(retained);
    harness.drain_deferred_cleanup().await?;
    Ok(())
}
