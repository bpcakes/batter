#[path = "../support/live_endpoint.rs"]
mod policy;

#[test]
fn live_handoff_preserves_credentials_across_native_parsers() {
    super::process::native("native-live-credentials");
}

pub(crate) fn check_credentials() {
    use sqlx::{ConnectOptions, postgres::PgConnectOptions};

    // Only fake credentials cross the native formatters; process state is cleared.
    for (userinfo, query, expected) in [
        ("user", "password=alpha+beta", "alpha beta"),
        ("user", "password=alpha%20beta", "alpha beta"),
        ("user", "password=alpha%2Bbeta+gamma", "alpha+beta gamma"),
        ("user", "password=%252B%26%3D%3F%23%C3%A9", "%2B&=?#é"),
        ("user:alpha+beta", "", "alpha+beta"),
        ("user:alpha%2Bbeta", "", "alpha+beta"),
        ("user:", "", ""),
        ("user", "password=", ""),
    ] {
        let query = if query.is_empty() {
            String::new()
        } else {
            format!("&{query}")
        };
        let url = format!(
            "postgres://{userinfo}@127.0.0.1/db?sslmode=disable&application_name=local+probe%2Btag{query}"
        );
        let (handoff, preflight) = policy::from_url_in_process(&url).unwrap();
        // The pinned harness normalizes through Url before its AdminClient.
        let normalized = url::Url::parse(handoff.expose_secret()).unwrap();
        let harness: tokio_postgres::Config = normalized.as_str().parse().unwrap();
        assert_eq!(harness.get_password(), Some(expected.as_bytes()));
        assert_eq!(harness.get_application_name(), Some("local probe+tag"));
        let fixture: PgConnectOptions = handoff.expose_secret().parse().unwrap();
        for options in [preflight, fixture] {
            let native_url = options.to_url_lossy();
            let password =
                percent_encoding::percent_decode_str(native_url.password().unwrap_or_default())
                    .decode_utf8()
                    .unwrap();
            assert_eq!(password, expected);
            assert_eq!(options.get_application_name(), Some("local probe+tag"));
        }
    }
}

#[test]
fn live_preflight_and_fixture_share_the_native_endpoint_policy() {
    super::process::native("native-live-endpoint");
}

pub(crate) fn check_policy() {
    // Child environment is empty, including PG* and HOME; no connection is made.
    assert!(policy::from_process().is_err());
    for url in [
        "postgres://user:fake@127.0.0.1/db?sslmode=disable",
        "postgres://user@localhost/db?sslmode=disable",
    ] {
        let (retained, options) = policy::from_url_in_process(url).unwrap();
        assert_eq!(
            retained.expose_secret(),
            if url.contains("user@") {
                format!("{url}&password=")
            } else {
                url.to_owned()
            }
        );
        assert!(options.get_socket().is_none());
    }
    check_ipv6_rejection();
    check_native_scalars();
    for (host, expected_host) in [
        ("local%68ost", "localhost"),
        ("%6cocalhost", "localhost"),
        ("127.0.0.%31", "127.0.0.1"),
    ] {
        let url = format!("postgres://%75ser:fake@{host}:6543/db?sslmode=disable");
        let (handoff, preflight) = policy::from_url_in_process(&url).unwrap();
        assert_eq!(preflight.get_host(), expected_host);
        // The pinned harness parses through Url before its admin client, then
        // clones that URL and replaces the database path for fixture pools.
        let mut normalized = url::Url::parse(handoff.expose_secret()).unwrap();
        let admin: tokio_postgres::Config = normalized.as_str().parse().unwrap();
        assert_eq!(
            admin.get_hosts(),
            &[tokio_postgres::config::Host::Tcp(expected_host.into())]
        );
        assert_eq!(admin.get_ports(), &[6543]);
        assert_eq!(admin.get_user(), Some("user"));
        assert_eq!(admin.get_dbname(), Some("db"));
        normalized.set_path("/fixture-db");
        let fixture: sqlx::postgres::PgConnectOptions = normalized.as_str().parse().unwrap();
        assert_eq!(fixture.get_host(), expected_host);
        assert_eq!(fixture.get_port(), 6543);
        assert_eq!(fixture.get_username(), "user");
        assert_eq!(fixture.get_database(), Some("fixture-db"));
        assert!(fixture.get_socket().is_none());
    }
    for url in [
        "postgres://user@127.0.0.1/db",
        "postgres://user@127.0.0.1/db?sslmode=require",
        "postgres://user@remote.invalid/db?sslmode=disable",
        "postgres://user@127.0.0.1/db?sslmode=disable&connect_timeout=10",
        "postgres://user@127.0.0.1/db?sslmode=disable&host=remote.invalid",
        "postgres://user@127.0.0.1/db?sslmode=disable&sslmode=disable",
        "postgres://user@127.0.0.1/db?sslmode=disable&password=%ff",
        "postgres://user:fake@127.0.0.1/db?sslmode=disable&password=conflict",
        "postgres://127.0.0.1/db?sslmode=disable",
    ] {
        assert!(
            policy::from_url_in_process(url).is_err(),
            "invalid fixture endpoint accepted"
        );
    }
}

fn check_ipv6_rejection() {
    for host in ["[::1]", "[0:0:0:0:0:0:0:1]", "%3A%3A1"] {
        let url = format!("postgres://user:fake@{host}/db?sslmode=disable");
        let error = policy::from_url_in_process(&url).unwrap_err();
        assert_eq!(error.field(), "POSTGRES_TEST_ADMIN_URL");
        assert_eq!(
            error.reason(),
            "IPv6 literals are unsupported by the live fixture URL consumer"
        );
    }
}

fn check_native_scalars() {
    for (path, expected_database) in [("db", "db"), ("/db", "/db"), ("%2Fdb", "/db")] {
        let url = format!("postgres://user:fake@localhost/{path}?sslmode=DISABLE");
        let (handoff, preflight) = policy::from_url_in_process(&url).unwrap();
        assert_eq!(preflight.get_database(), Some(expected_database));
        let normalized = url::Url::parse(handoff.expose_secret()).unwrap();
        let admin: tokio_postgres::Config = normalized.as_str().parse().unwrap();
        assert_eq!(admin.get_dbname(), Some(expected_database));
        assert_eq!(
            admin.get_ssl_mode(),
            tokio_postgres::config::SslMode::Disable
        );
        let native: sqlx::postgres::PgConnectOptions = normalized.as_str().parse().unwrap();
        assert_eq!(native.get_database(), Some(expected_database));
        assert!(matches!(
            native.get_ssl_mode(),
            sqlx::postgres::PgSslMode::Disable
        ));
    }
}

pub(crate) fn check_no_passfile(url: &str) {
    use sqlx::ConnectOptions;
    let (handoff, _) = policy::from_url_in_process(url).unwrap();
    let options: sqlx::postgres::PgConnectOptions = handoff.expose_secret().parse().unwrap();
    assert!(options.to_url_lossy().password().is_none_or(str::is_empty));
}
