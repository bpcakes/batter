use super::*;
use crate::test_support::endpoint::require_clean_environment;
use crate::test_support::endpoint::{HarnessEndpointReason, UnsupportedHarnessEndpoint};

fn endpoint(admin_url: &str) -> Arc<HarnessEndpoint> {
    require_clean_environment();
    Arc::new(
        HarnessEndpoint::from_admin_url(admin_url, Environment::CLEAN).expect("supported endpoint"),
    )
}

fn reason(error: &sqlx::Error) -> Option<HarnessEndpointReason> {
    match error {
        sqlx::Error::Configuration(source) => source
            .downcast_ref::<UnsupportedHarnessEndpoint>()
            .map(UnsupportedHarnessEndpoint::reason),
        _ => None,
    }
}

#[test]
fn targets_select_the_recorded_database_on_the_harness_endpoint() {
    for (admin, harness_url) in [
        (
            "postgres://owner:p@127.0.0.1:6543/postgres?sslmode=disable&application_name=a%20b",
            "postgres://owner:p@127.0.0.1:6543/pgh_template_1?sslmode=disable&application_name=a%20b",
        ),
        // tokio-postgres decodes the authority host; the target sets it
        // through SQLx's setter, so no URL text is reread.
        (
            "postgres://owner:p@db%2Bblue.example/postgres",
            "postgres://owner:p@db%2Bblue.example/pgh_template_1",
        ),
        (
            "postgres://owner@%2Ftmp%2Fpg%20socket:5432/postgres",
            "postgres://owner@%2Ftmp%2Fpg%20socket:5432/pgh_template_1",
        ),
    ] {
        let target = TemplateTarget::new(&endpoint(admin), harness_url, Environment::CLEAN)
            .expect(harness_url);
        assert_eq!(target.database_name(), "pgh_template_1");
        let options = target.connect_options();
        assert_eq!(options.get_database(), Some("pgh_template_1"), "{admin}");
        assert_eq!(options.get_username(), "owner");
        assert!(matches!(
            options.get_ssl_mode(),
            sqlx::postgres::PgSslMode::Disable
        ));
    }
    let tcp = TemplateTarget::new(
        &endpoint("postgres://owner:p@db%2Bblue.example/postgres"),
        "postgres://owner:p@db%2Bblue.example/pgh_template_1",
        Environment::CLEAN,
    )
    .unwrap();
    assert_eq!(tcp.connect_options().get_host(), "db+blue.example");
}

#[test]
fn targets_refuse_urls_the_harness_would_not_write_for_this_endpoint() {
    let admin = endpoint("postgres://u@127.0.0.1:5432/postgres");
    // A `dbname` kept from the admin URL overrides the path for every client;
    // so do paths the harness never writes.
    for url in [
        "postgres://u@127.0.0.1:5432/pgh_template_1?dbname=postgres",
        "postgres://u@127.0.0.1:5432//jobs",
        "postgres://u@127.0.0.1:5432/one/../jobs",
        "postgres://u@127.0.0.1:5432/",
        "postgres://u@127.0.0.1:5432/Jobs%20x",
    ] {
        let error = TemplateTarget::new(&admin, url, Environment::CLEAN).unwrap_err();
        assert!(
            matches!(&error, sqlx::Error::Configuration(source)
                if source.is::<crate::test_support::LeaseDatabaseMismatch>()),
            "{url}"
        );
    }
    let foreign = TemplateTarget::new(
        &admin,
        "postgres://u@127.0.0.1:6543/pgh_template_1",
        Environment::CLEAN,
    )
    .unwrap_err();
    assert_eq!(
        reason(&foreign),
        Some(HarnessEndpointReason::LeaseEndpointMismatch)
    );
    let ambient = TemplateTarget::new(
        &admin,
        "postgres://u@127.0.0.1:5432/pgh_template_1",
        Environment::AMBIENT,
    )
    .unwrap_err();
    assert_eq!(
        reason(&ambient),
        Some(HarnessEndpointReason::AmbientEnvironment)
    );
}

#[test]
fn debug_output_names_only_the_database() {
    let target = TemplateTarget::new(
        &endpoint("postgres://owner:secret-marker@127.0.0.1:5432/postgres"),
        "postgres://owner:secret-marker@127.0.0.1:5432/pgh_template_1",
        Environment::CLEAN,
    )
    .unwrap();
    let text = format!("{target:?}");
    assert!(
        !text.contains("secret-marker") && text.contains("pgh_template_1"),
        "{text}"
    );
}
