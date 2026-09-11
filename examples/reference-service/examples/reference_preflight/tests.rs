use super::*;

#[test]
fn native_entrypoint_rejects_invalid_inputs_before_live_work() {
    use process_runtime::{ExpectedExit, run};
    let endpoint = "postgres://user:secret-marker@127.0.0.1/db?sslmode=disable";
    for (env, expected) in [
        (vec![], "explicit Unicode endpoint required"),
        (
            vec![
                ("POSTGRES_TEST_ADMIN_URL", endpoint.into()),
                ("PGDATA", "secret-marker".into()),
            ],
            "native PG settings are unsupported",
        ),
        (
            vec![(
                "POSTGRES_TEST_ADMIN_URL",
                format!("{endpoint}&connect_timeout=10").into(),
            )],
            "unsupported query setting",
        ),
        (
            vec![(
                "POSTGRES_TEST_ADMIN_URL",
                "postgres://user:secret-marker@remote.invalid/db?sslmode=disable".into(),
            )],
            "local disposable endpoint",
        ),
        (
            vec![(
                "POSTGRES_TEST_ADMIN_URL",
                "postgres://user:secret-marker@[::1]:9/db?sslmode=disable".into(),
            )],
            "IPv6 literals are unsupported by the live fixture URL consumer",
        ),
        (
            vec![("POSTGRES_TEST_ADMIN_URL", endpoint.into())],
            "POSTGRES_TEST_OBSERVER_URL: explicit Unicode endpoint required",
        ),
        (
            vec![
                ("POSTGRES_TEST_ADMIN_URL", endpoint.into()),
                (
                    "POSTGRES_TEST_OBSERVER_URL",
                    "postgres://user:secret-marker@remote.invalid/db?sslmode=disable".into(),
                ),
            ],
            "POSTGRES_TEST_OBSERVER_URL: local disposable endpoint",
        ),
        (
            vec![
                ("POSTGRES_TEST_ADMIN_URL", endpoint.into()),
                (
                    "POSTGRES_TEST_OBSERVER_URL",
                    "postgres://user:secret-marker@127.0.0.1/db?sslmode=require".into(),
                ),
            ],
            "POSTGRES_TEST_OBSERVER_URL: local disposable endpoint",
        ),
        (
            vec![
                ("POSTGRES_TEST_ADMIN_URL", endpoint.into()),
                (
                    "POSTGRES_TEST_OBSERVER_URL",
                    format!("{endpoint}&host=remote.invalid").into(),
                ),
            ],
            "POSTGRES_TEST_OBSERVER_URL: unsupported query setting",
        ),
    ] {
        run("preflight-rejected", &env)
            .validate_text(
                ExpectedExit::Code(42),
                &["reference-preflight:", expected],
                &["secret-marker", "reference-preflight:ok", "panicked"],
            )
            .unwrap();
    }
}

#[test]
fn cluster_policy_preserves_signed_identity_and_rejects_failed_prerequisites() {
    for identity in [i64::MIN, -123, 0, 123, i64::MAX] {
        assert_eq!(accept_primary((true, identity)).unwrap(), identity);
        assert!(accept_primary((false, identity)).is_err());
        assert!(accept_secondary(identity, (true, identity)).is_err());
        let distinct = if identity == i64::MAX {
            i64::MIN
        } else {
            i64::MAX
        };
        assert!(accept_secondary(identity, (true, distinct)).is_ok());
        assert!(accept_secondary(identity, (false, distinct)).is_err());
    }
}
