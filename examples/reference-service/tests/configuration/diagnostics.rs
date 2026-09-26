use crate::{load, source};
use batter::settings::SettingsSource;
use batter_example_reference_service::config::{MaintenanceSettings, WorkerSettings};
use sqlx::{ConnectOptions, postgres::PgSslMode};
use std::error::Error;

#[test]
fn native_endpoint_mapping_preserves_credentials_tls_and_encoded_values() {
    super::process::native("native-options");
}

pub(crate) fn native_options() {
    for host in ["localhost", "127.0.0.1", "[::1]"] {
        let root = load(&[("DATABASE_URL", &format!("postgresql://user%40name:fake%2540pass%3Aword@{host}:6543/data%2Fbase?sslmode=verify-full&application_name=configured%20app"))]).unwrap();
        let options = root.connect_options_from_process().unwrap();
        assert_eq!(options.get_host(), host.trim_matches(['[', ']']));
        assert_eq!(options.get_port(), 6543);
        assert_eq!(options.get_username(), "user@name");
        assert_eq!(options.get_database(), Some("data/base"));
        assert!(matches!(options.get_ssl_mode(), PgSslMode::VerifyFull));
        assert_eq!(options.get_application_name(), Some("configured app"));
        assert!(options.get_socket().is_none());
        assert!(options.get_options().is_none());
        // Deliberate native URL exposure with fake credentials only.
        // SQLx 0.9's URL formatter does not bracket a setter-provided IPv6 host.
        // Credential round-tripping is checked on DNS/IPv4; the separate native
        // wire test below proves IPv6 credentials and endpoint consumption.
        if host != "[::1]" {
            let native_url = options.to_url_lossy();
            let password = percent_encoding::percent_decode_str(native_url.password().unwrap())
                .decode_utf8()
                .unwrap();
            assert_eq!(password, "fake%40pass:word");
        }
    }
    let root = load(&[(
        "DATABASE_URL",
        "postgres://user@host/db?password=fake%2Bpassword+space&sslmode=require",
    )])
    .unwrap();
    let options = root.connect_options_from_process().unwrap();
    let native_url = options.to_url_lossy();
    assert_eq!(
        percent_encoding::percent_decode_str(native_url.password().unwrap())
            .decode_utf8()
            .unwrap(),
        "fake+password space"
    );
    assert!(matches!(options.get_ssl_mode(), PgSslMode::Require));

    let maintenance = MaintenanceSettings::from_sources(
        None,
        SettingsSource::default(),
        source(&[(
            "DATABASE_URL",
            "postgres://user:secret-maintenance-marker@host/db?sslmode=disable",
        )]),
    )
    .unwrap()
    .prepare()
    .unwrap();
    for text in [
        format!("{maintenance:?}"),
        format!("{maintenance:#?}"),
        format!("{:?}", vec![&maintenance]),
    ] {
        assert!(!text.contains("secret-maintenance-marker"));
        assert!(text.contains("PreparedMaintenance([REDACTED])"));
    }
}

#[test]
fn rejected_urls_and_nested_aggregates_never_format_input() {
    for url in [
        "secret-marker",
        "postgres://user:@host/db?sslmode=disable&password=secret-marker",
        "postgres://user:secret-marker@host/db",
        "postgres://user:secret-marker@host/db?sslmode=disable#fragment",
        "postgres://user:secret-marker@host:0/db?sslmode=disable",
        "postgres://user:secret-marker@host:65536/db?sslmode=disable",
        "postgres://user:secret-marker@host/db?sslmode=disable&secret-marker=secret-marker",
        "postgres://user:secret-marker@host/db?sslmode=disable&password=secret-marker",
        "postgres://user:secret-marker@host/db?sslmode=disable&sslmode=require",
        "postgres://user:secret-marker@host/db?sslmode=secret-marker",
        "postgres://user:secret-marker%ff@host/db?sslmode=disable",
        "postgres://user:secret-marker%xx@host/db?sslmode=disable",
        "postgres://user:secret-marker@host/db?sslmode=disable&application_name=%ff",
        "postgres://user:secret-marker@host/db?sslmode=disable&application_name=x&application_name=y",
        "postgres://user:secret-marker@/db?sslmode=disable",
        "postgres://user:secret-marker@%2Ftmp/db?sslmode=disable",
        "postgres://:secret-marker@host/db?sslmode=disable",
        "postgres://user:secret-marker@host/?sslmode=disable",
        "postgres://user:secret-marker@host/db?sslmode=disable&password=%00",
    ] {
        let error = load(&[("DATABASE_URL", url)]).unwrap_err();
        let text = format!("{error} {error:#?} {:?}", vec![&error]);
        assert!(text.contains("DATABASE_URL"));
        assert!(!text.contains("secret-marker"));
    }
    let root = load(&[
        (
            "DATABASE_URL",
            "postgres://user:secret-marker@host/db?sslmode=disable",
        ),
        ("JOBS_WORKER_ID", "secret-marker"),
        ("BATTER_AUTH_TOKEN", "secret-auth-marker"),
    ])
    .unwrap();
    for text in [
        format!("{root:#?}"),
        format!("{root}"),
        format!("{:?}", vec![&root]),
        format!("{:?}", root.authenticator()),
    ] {
        assert!(!text.contains("secret-marker"));
        assert!(!text.contains("secret-auth-marker"));
    }
    let worker =
        WorkerSettings::from_source(&source(&[("JOBS_WORKER_ID", "secret-worker-marker")]))
            .unwrap();
    for text in [
        format!("{worker:#?}"),
        format!("{worker}"),
        format!("{:?}", vec![&worker]),
    ] {
        assert!(!text.contains("secret-worker-marker"));
    }
    let error = load(&[("BATTER_AUTH_TOKEN", "secret auth marker")]).unwrap_err();
    assert!(!format!("{error} {error:#?}").contains("secret auth marker"));
    for (name, value) in [
        (
            "BATTER_PROVIDER_BASE_URL",
            "https://user:secret-provider-marker@provider.example/",
        ),
        ("BATTER_PROVIDER_TOKEN", "secret provider marker"),
    ] {
        let error = load(&[(name, value)]).unwrap_err();
        let diagnostic = format!("{error} {error:#?} {:?}", vec![&error]);
        assert!(diagnostic.contains(name));
        assert!(!diagnostic.contains("secret-provider-marker"));
        assert!(!diagnostic.contains("secret provider marker"));
    }
    let error = load(&[(
        "DATABASE_URL",
        "postgres://user:secret-marker@host/db?sslmode=secret-marker",
    )])
    .unwrap_err();
    assert!(error.source().unwrap().is::<sqlx::Error>());
    assert!(
        error
            .source()
            .unwrap()
            .to_string()
            .contains("secret-marker")
    );
}

pub(crate) fn child(scenario: &str) {
    match scenario {
        "inert-serving-preparation" => inert_serving_preparation(),
        "native-options" => native_options(),
        "native-live-endpoint" => super::live_endpoint::check_policy(),
        "native-live-credentials" => super::live_endpoint::check_credentials(),
        "native-ipv6" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(native_ipv6()),
        #[cfg(feature = "metrics-export")]
        "metrics-ambient-otel" => super::metrics::ambient_child(),
        "native-worker" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(super::consumers::native_worker()),
        _ => return environment_child(scenario),
    }
    eprintln!("configuration-child:native-complete");
}

fn inert_serving_preparation() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let settings = load(&[("BATTER_BIND", &address.to_string())]).unwrap();

    // This function intentionally runs without a Tokio runtime. Spawning would
    // panic, and attempting to bind the configured address would fail because
    // this test owns it already.
    let prepared = batter_example_reference_service::runtime::prepare(settings).unwrap();
    assert_eq!(format!("{prepared:?}"), "PreparedServing([REDACTED])");
    drop(prepared);
    drop(listener);
}

fn environment_child(scenario: &str) {
    let loaded = MaintenanceSettings::from_process(None, SettingsSource::default());
    match scenario {
        "maintenance-serving-environment" => {
            let settings = loaded.unwrap();
            assert_eq!(format!("{settings:?}"), "MaintenanceSettings([REDACTED])");
            eprintln!("configuration-child:maintenance-environment-ignored");
        }
        "valid-passfile-ignored" => {
            let settings = loaded.unwrap();
            let options = settings.connect_options_from_process().unwrap();
            // Empty explicit maintenance password must not pick up the fake .pgpass value.
            assert!(options.to_url_lossy().password().is_none_or(str::is_empty));
            super::live_endpoint::check_no_passfile(&std::env::var("DATABASE_URL").unwrap());
            eprintln!("configuration-child:passfile-ignored");
        }
        "reject-pg" | "reject-url" | "reject-nonunicode" => {
            let error = loaded.expect_err("invalid configuration unexpectedly accepted");
            eprintln!("configuration-child:rejected {error}");
            std::process::exit(42);
        }
        "injected-pg" => {
            let maintenance = MaintenanceSettings::from_sources(
                None,
                SettingsSource::default(),
                source(&[("DATABASE_URL", "postgres://user@host/db?sslmode=disable")]),
            )
            .unwrap();
            let maintenance_error = maintenance.prepare().unwrap_err();
            let serving_error = batter_example_reference_service::runtime::prepare(
                load(&[]).expect("injected serving settings are valid"),
            )
            .unwrap_err();
            assert_eq!(maintenance_error.field(), "environment");
            assert_eq!(serving_error.field(), "environment");
            eprintln!("configuration-child:rejected {maintenance_error}");
            std::process::exit(42);
        }
        _ => panic!("unknown configuration fixture"),
    }
}

#[test]
fn native_ipv6_connection_sends_exact_decoded_credentials() {
    super::process::native("native-ipv6");
}

pub(crate) async fn native_ipv6() {
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::timeout,
    };
    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = load(&[(
        "DATABASE_URL",
        &format!(
            "postgres://wire%40user:fake%2540pass%3Aword@[::1]:{port}/wire%2Fdb?sslmode=disable"
        ),
    )])
    .unwrap();
    let options = root.connect_options_from_process().unwrap();
    let exchange = async {
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let length = socket.read_u32().await.unwrap();
            assert!((8..1024).contains(&length));
            let mut startup = vec![0; length as usize - 4];
            socket.read_exact(&mut startup).await.unwrap();
            assert_eq!(&startup[..4], &196608_u32.to_be_bytes());
            assert!(
                startup[4..]
                    .split(|byte| *byte == 0)
                    .any(|part| part == b"wire@user")
            );
            assert!(
                startup[4..]
                    .split(|byte| *byte == 0)
                    .any(|part| part == b"wire/db")
            );
            socket
                .write_all(&[b'R', 0, 0, 0, 8, 0, 0, 0, 3])
                .await
                .unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), b'p');
            let length = socket.read_u32().await.unwrap();
            assert!((4..1024).contains(&length));
            let mut password = vec![0; length as usize - 4];
            socket.read_exact(&mut password).await.unwrap();
            assert_eq!(password, b"fake%40pass:word\0");
            // Intentionally end the fake server after authentication evidence.
        };
        let (connected, ()) = tokio::join!(options.connect(), server);
        assert!(connected.is_err());
    };
    timeout(Duration::from_secs(3), exchange).await.unwrap();
}
