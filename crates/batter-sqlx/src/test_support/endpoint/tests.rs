use super::*;

fn reason(admin_url: &str) -> Option<HarnessEndpointReason> {
    require_clean_environment();
    match HarnessEndpoint::from_admin_url(admin_url, Environment::CLEAN) {
        Ok(_) => None,
        Err(sqlx::Error::Configuration(source)) => Some(
            source
                .downcast_ref::<UnsupportedHarnessEndpoint>()
                .expect("refusal is UnsupportedHarnessEndpoint")
                .reason(),
        ),
        Err(error) => panic!("unexpected error {error}"),
    }
}

fn endpoint(admin_url: &str) -> HarnessEndpoint {
    require_clean_environment();
    HarnessEndpoint::from_admin_url(admin_url, Environment::CLEAN).expect("supported endpoint")
}

#[test]
fn any_pg_variable_is_refused_before_parsing() {
    let error = HarnessEndpoint::from_admin_url("not a url", Environment { pg_variables: true })
        .unwrap_err();
    let sqlx::Error::Configuration(source) = &error else {
        panic!("configuration error expected");
    };
    assert_eq!(
        source
            .downcast_ref::<UnsupportedHarnessEndpoint>()
            .map(UnsupportedHarnessEndpoint::reason),
        Some(HarnessEndpointReason::AmbientEnvironment)
    );
}

#[test]
fn endpoint_fields_come_from_the_harness_parser() {
    let tcp = endpoint(
        "postgres://owner:p%40ss@127.0.0.1:6543/postgres?sslmode=disable&application_name=fixture",
    );
    let options = tcp.options("pgh_lease_1").unwrap();
    assert_eq!(options.get_host(), "127.0.0.1");
    assert_eq!(options.get_port(), 6543);
    assert_eq!(options.get_username(), "owner");
    assert_eq!(options.get_database(), Some("pgh_lease_1"));
    assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
    assert_eq!(options.get_application_name(), Some("fixture"));
    assert_eq!(tcp.admin_database, "postgres");

    // tokio-postgres defaults the port and decodes the host; both are carried.
    let decoded = endpoint("postgres://owner@local%68ost/postgres")
        .options("db")
        .unwrap();
    assert_eq!(decoded.get_host(), "localhost");
    assert_eq!(decoded.get_port(), DEFAULT_PORT);

    let socket = endpoint("postgres://owner@%2Ftmp%2Fpg+socket:5432/postgres")
        .options("db")
        .unwrap();
    assert_eq!(socket.get_socket(), Some(&PathBuf::from("/tmp/pg+socket")));
}

#[test]
fn unsupported_endpoints_are_refused_with_their_reason() {
    use HarnessEndpointReason::*;
    for (url, expected) in [
        (
            "postgres://u@127.0.0.1:not-a-port/postgres",
            Some(Unparseable),
        ),
        (
            "postgres://u@127.0.0.1:5432/postgres?hostaddr=192.0.2.1",
            Some(HostAddress),
        ),
        ("postgres://u@db-a,db-b:5432/postgres", Some(MultipleHosts)),
        (
            "postgres://u@127.0.0.1/postgres?options=--x%3Dy",
            Some(UnsupportedSetting),
        ),
        (
            "postgres://u@127.0.0.1/postgres?options=-c%20x",
            Some(UnsupportedSetting),
        ),
        (
            "postgres://u@127.0.0.1/postgres?options=-c%20x%3Dy%20-c%20z%3Da%5C%20b",
            None,
        ),
        (
            "postgres://u@127.0.0.1/postgres?target_session_attrs=read-write",
            Some(UnsupportedSetting),
        ),
        (
            "postgres://u@127.0.0.1/postgres?channel_binding=require",
            Some(UnsupportedSetting),
        ),
        ("postgres://127.0.0.1/postgres", Some(MissingUser)),
        (
            "postgres://u@localhost/postgres?load_balance_hosts=random",
            Some(UnsupportedSetting),
        ),
        (
            "postgres://u@%2Ftmp%2F%FF:5432/postgres",
            Some(NonUtf8SocketPath),
        ),
        (
            "postgres://u@%5Bdb.example:5432/postgres",
            Some(UnsupportedHostName),
        ),
        ("postgres://u@[::1]:5432/postgres", None),
        ("postgres://u:%FF@127.0.0.1/postgres", Some(NonUtf8Password)),
        ("postgres://u@127.0.0.1:5432/postgres?sslmode=require", None),
    ] {
        assert_eq!(reason(url), expected, "{url}");
    }
    let error = HarnessEndpoint::from_admin_url(
        "postgres://u:secret-marker@127.0.0.1:5432/postgres?hostaddr=192.0.2.1",
        Environment::CLEAN,
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(
        !text.contains("secret-marker") && !text.contains("192.0.2.1"),
        "{text}"
    );
}

#[test]
fn rebuilt_urls_read_back_as_the_same_endpoint_in_both_parsers() {
    for admin in [
        "postgres://owner:p%40ss%2Bw+rd@127.0.0.1:6543/postgres?sslmode=disable&application_name=a%20b",
        "postgres://owner@[::1]:5432/postgres?sslmode=prefer",
        "postgres://owner@%2Ftmp%2Fpg%20socket:5432/postgres",
        "postgres://own%40er@db.invalid/postgres?sslmode=require",
        "postgres://owner@127.0.0.1:5432/postgres?options=-c%20search_path%3Dpublic",
    ] {
        let endpoint = endpoint(admin);
        let url = endpoint.url("pgh_template_1").expect(admin);
        let options: PgConnectOptions = url.parse().unwrap();
        assert_eq!(options.get_database(), Some("pgh_template_1"), "{url}");
        assert!(
            HarnessEndpoint::names_database(&url, "pgh_template_1"),
            "{url}"
        );
        assert!(!url.contains("secret"), "{url}");
    }
}

#[test]
fn recorded_names_and_database_overrides_are_read_from_harness_urls() {
    require_clean_environment();
    let harness_url = "postgres://u@127.0.0.1:5432/pgh_template_1?sslmode=disable";
    assert_eq!(
        HarnessEndpoint::recorded_database(harness_url).as_deref(),
        Some("pgh_template_1")
    );
    assert!(HarnessEndpoint::names_database(
        harness_url,
        "pgh_template_1"
    ));
    // A `dbname` kept from the admin URL overrides the path for every client.
    let overridden = "postgres://u@127.0.0.1:5432/pgh_template_1?dbname=postgres";
    assert!(!HarnessEndpoint::names_database(
        overridden,
        "pgh_template_1"
    ));
    let error = initializer_url(
        "postgres://u@127.0.0.1:5432/postgres?dbname=postgres",
        overridden,
        Environment::CLEAN,
    )
    .unwrap_err();
    assert!(matches!(&error, sqlx::Error::Configuration(source)
        if source.is::<super::super::LeaseDatabaseMismatch>()));
    // Paths the harness never writes select no database the initializer is
    // given: either no recorded name, or one tokio-postgres reads differently.
    let admin = "postgres://u@127.0.0.1:5432/postgres";
    for url in [
        "postgres://u@127.0.0.1:5432//jobs",
        "postgres://u@127.0.0.1:5432/one/../jobs",
        "postgres://u@127.0.0.1:5432/",
        "postgres://u@127.0.0.1:5432/Jobs%20x",
    ] {
        let recorded = HarnessEndpoint::recorded_database(url);
        assert!(
            recorded
                .as_deref()
                .is_none_or(|name| !HarnessEndpoint::names_database(url, name)),
            "{url}"
        );
        assert!(
            initializer_url(admin, url, Environment::CLEAN).is_err(),
            "{url}"
        );
    }
    let rebuilt = initializer_url(
        "postgres://u@127.0.0.1:5432/postgres?sslmode=disable",
        harness_url,
        Environment::CLEAN,
    )
    .unwrap();
    assert_eq!(
        rebuilt,
        "postgres://u@127.0.0.1:5432/pgh_template_1?sslmode=disable"
    );
}

#[test]
fn rebuilt_urls_refuse_tcp_names_sqlx_would_not_decode() {
    // tokio-postgres decodes the authority host; SQLx resolves it as written,
    // so a re-encoded `+` would name another host and must be refused.
    let endpoint = endpoint("postgres://owner@db%2Bblue.example/postgres");
    assert_eq!(
        endpoint.options("db").unwrap().get_host(),
        "db+blue.example"
    );
    let error = endpoint.url("pgh_template_1").unwrap_err();
    let sqlx::Error::Configuration(source) = &error else {
        panic!("configuration error expected");
    };
    assert_eq!(
        source
            .downcast_ref::<UnsupportedHarnessEndpoint>()
            .map(UnsupportedHarnessEndpoint::reason),
        Some(HarnessEndpointReason::RoundTrip)
    );
}

#[test]
fn explicit_empty_passwords_survive_the_rebuilt_url() {
    let endpoint = endpoint("postgres://owner@127.0.0.1:5432/postgres?password=");
    assert_eq!(endpoint.password.as_deref(), Some(""));
    let url = endpoint.url("pgh_template_1").unwrap();
    assert!(url.ends_with("&password="), "{url}");
    let again = HarnessEndpoint::from_admin_url(&url, Environment::CLEAN).unwrap();
    assert_eq!(again.password.as_deref(), Some(""), "{url}");
}

#[test]
fn debug_output_omits_the_password() {
    let endpoint = endpoint("postgres://owner:secret-marker@127.0.0.1:5432/postgres");
    let text = format!("{endpoint:?}");
    assert!(
        !text.contains("secret-marker") && text.contains("owner"),
        "{text}"
    );
}

#[test]
fn initializer_urls_keep_the_connect_timeout_and_refuse_other_tuning() {
    let endpoint = endpoint("postgres://owner@127.0.0.1:5432/postgres?connect_timeout=2");
    let url = endpoint.url("pgh_template_1").unwrap();
    assert!(url.ends_with("&connect_timeout=2"), "{url}");
    let config = tokio_postgres::Config::from_str(&url).unwrap();
    assert_eq!(
        config.get_connect_timeout(),
        Some(&std::time::Duration::from_secs(2))
    );
    for setting in [
        "tcp_user_timeout=5",
        "keepalives=0",
        "keepalives_idle=30",
        "keepalives_interval=5",
        "keepalives_retries=3",
    ] {
        let url = format!("postgres://owner@127.0.0.1:5432/postgres?{setting}");
        assert_eq!(
            reason(&url),
            Some(HarnessEndpointReason::UnsupportedSetting),
            "{url}"
        );
    }
}

#[test]
fn diagnostic_options_are_bound_to_the_lease_and_its_harness() {
    require_clean_environment();
    let admin = "postgres://owner:p@127.0.0.1:5432/postgres?sslmode=disable";
    let lease = "postgres://owner:p@127.0.0.1:5432/pgh_lease_1?sslmode=disable";
    let options = lease_options(admin, lease, "pgh_lease_1", Environment::CLEAN).unwrap();
    assert_eq!(options.get_database(), Some("pgh_lease_1"));
    // A name the lease URL does not select.
    let error = lease_options(admin, lease, "postgres", Environment::CLEAN).unwrap_err();
    assert!(matches!(&error, sqlx::Error::Configuration(source)
        if source.is::<super::super::LeaseDatabaseMismatch>()));
    // A lease reaching another endpoint: different port, or different login.
    for foreign in [
        "postgres://owner:p@127.0.0.1:6543/pgh_lease_1?sslmode=disable",
        "postgres://other:p@127.0.0.1:5432/pgh_lease_1?sslmode=disable",
    ] {
        let error = lease_options(admin, foreign, "pgh_lease_1", Environment::CLEAN).unwrap_err();
        let sqlx::Error::Configuration(source) = &error else {
            panic!("configuration error expected");
        };
        assert_eq!(
            source
                .downcast_ref::<UnsupportedHarnessEndpoint>()
                .map(UnsupportedHarnessEndpoint::reason),
            Some(HarnessEndpointReason::LeaseEndpointMismatch),
            "{foreign}"
        );
    }
    // The check is an endpoint match, not ownership: a second harness's lease
    // on the same server and login is indistinguishable and accepted.
    let sibling = "postgres://owner:p@127.0.0.1:5432/pgh_other_harness_1?sslmode=disable";
    assert!(lease_options(admin, sibling, "pgh_other_harness_1", Environment::CLEAN).is_ok());
    // Such a lease may still not name this harness's admin database: a second
    // harness whose admin database another harness leased.
    let own_admin = "postgres://owner:p@127.0.0.1:5432/postgres?sslmode=disable";
    let error = lease_options(admin, own_admin, "postgres", Environment::CLEAN).unwrap_err();
    let sqlx::Error::Configuration(source) = &error else {
        panic!("configuration error expected");
    };
    assert_eq!(
        source
            .downcast_ref::<UnsupportedHarnessEndpoint>()
            .map(UnsupportedHarnessEndpoint::reason),
        Some(HarnessEndpointReason::LeaseNamesAdminDatabase)
    );
}

#[test]
fn initializer_urls_keep_disabled_channel_binding() {
    let disabled = endpoint("postgres://owner@127.0.0.1:5432/postgres?channel_binding=disable");
    let url = disabled.url("pgh_template_1").unwrap();
    let config = tokio_postgres::Config::from_str(&url).unwrap();
    assert_eq!(
        config.get_channel_binding(),
        ChannelBinding::Disable,
        "{url}"
    );
    let preferred = endpoint("postgres://owner@127.0.0.1:5432/postgres")
        .url("pgh_template_1")
        .unwrap();
    assert!(!preferred.contains("channel_binding"), "{preferred}");
}
