//! Cache-hit and cache-miss controls for the fixture template entry points.
use batter_sqlx::test_support::{
    AcquisitionFailure, FixtureError, FixtureSuite, HarnessEndpointReason,
    UnsupportedHarnessEndpoint, template_spec,
};
use postgres_test_harness::{Error, HarnessConfig, POSTGRES_TEST_ADMIN_URL_ENV, PostgresHarness};

fn assert_ambient(error: &Error) {
    let Error::TemplateInitializer { source } = error else {
        panic!("expected template preparation refusal");
    };
    let Some(sqlx::Error::Configuration(source)) = source.downcast_ref::<sqlx::Error>() else {
        panic!("expected an endpoint configuration error");
    };
    assert_eq!(
        source
            .downcast_ref::<UnsupportedHarnessEndpoint>()
            .map(UnsupportedHarnessEndpoint::reason),
        Some(HarnessEndpointReason::AmbientEnvironment),
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
        let Err(FixtureError::Harness(error)) = suite
            .template(spec, |_| async {
                panic!("initializer must not run");
            })
            .await
        else {
            panic!("manual template lookup must refuse the environment");
        };
        assert_ambient(&error);
    }
    let report = suite
        .start(move |scope| {
            Box::pin(async move {
                for spec in specs {
                    let Err(FixtureError::Acquisition(error)) = scope
                        .template(spec, |_| async {
                            panic!("initializer must not run");
                        })
                        .await
                    else {
                        panic!("owned template lookup must refuse the environment");
                    };
                    assert_ambient(&error);
                }
                Ok::<(), FixtureError>(())
            })
        })
        .into_report()
        .await?;
    assert!(report.body.is_ok(), "body must observe both refusals");
    assert_eq!(
        report.acquisitions.len(),
        2,
        "retain both refused producers"
    );
    for outcome in &report.acquisitions {
        let Err(AcquisitionFailure::Native(error)) = outcome else {
            panic!("expected a retained template refusal");
        };
        assert_ambient(error);
    }
    drop(retained);
    harness.drain_deferred_cleanup().await?;
    Ok(())
}
