use batter_sqlx::{PgProfiledPool, PgSessionProfile};
use std::time::Duration;

fn profile(schemas: Vec<String>) -> Result<PgSessionProfile, batter_sqlx::PgProfileError> {
    PgSessionProfile::with_timeouts(
        "login",
        "serving",
        schemas,
        Duration::from_secs(30),
        Duration::from_secs(1),
        Duration::from_secs(10),
        Duration::from_secs(60),
    )
}

#[test]
fn rejects_empty_reserved_and_truncated_identifiers() {
    for schemas in [
        vec![],
        vec![String::new()],
        vec!["pg_temp".into()],
        vec!["pg_catalog".into()],
        vec!["information_schema".into()],
        vec!["x".repeat(64)],
        vec!["bad\0name".into()],
    ] {
        assert!(profile(schemas).is_err());
    }
    assert!(
        PgSessionProfile::new(
            "",
            "serving",
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO
        )
        .is_err()
    );
}

#[test]
fn settings_cannot_override_owned_authority_or_transaction_parameters() {
    for name in [
        "role",
        "session_authorization",
        "search_path",
        "statement_timeout",
        "lock_timeout",
        "idle_in_transaction_session_timeout",
        "transaction_timeout",
        "Idle_In_Transaction_Session_Timeout",
        "TRANSACTION_TIMEOUT",
        "default_transaction_read_only",
        "default_transaction_isolation",
        "row_security",
        "app..tenant",
    ] {
        assert!(
            profile(vec!["public".into()])
                .unwrap()
                .with_setting(name, "value")
                .is_err()
        );
    }
    let tenant = profile(vec!["app".into()])
        .unwrap()
        .with_setting("app.tenant", "secret-tenant")
        .unwrap();
    assert!(!format!("{tenant:?}").contains("secret"));
    assert!(tenant.with_setting("app.tenant", "another").is_err());
}

#[test]
fn timeouts_are_exact_bounded_milliseconds() {
    for value in [
        Duration::from_nanos(1),
        Duration::from_millis(i32::MAX as u64 + 1),
    ] {
        assert!(
            PgSessionProfile::new(
                "login",
                "serving",
                vec!["public".into()],
                value,
                Duration::ZERO
            )
            .is_err()
        );
    }
    assert!(
        PgSessionProfile::new(
            "login",
            "serving",
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO
        )
        .is_ok()
    );
}

#[test]
fn complete_timeouts_validate_each_selection_before_io() {
    for field in 0..4 {
        for (value, valid) in [
            (Duration::ZERO, true),
            (Duration::from_millis(1), true),
            (Duration::from_millis(i32::MAX as u64), true),
            (Duration::from_millis(i32::MAX as u64 + 1), false),
            (Duration::from_nanos(1), false),
            (Duration::from_micros(1_001), false),
        ] {
            let mut timeouts = [Duration::ZERO; 4];
            timeouts[field] = value;
            let result = PgSessionProfile::with_timeouts(
                "private-login",
                "private-role",
                vec!["private-schema".into()],
                timeouts[0],
                timeouts[1],
                timeouts[2],
                timeouts[3],
            );
            assert_eq!(result.is_ok(), valid, "field {field}, value {value:?}");
            assert!(!format!("{result:?}").contains("private"));
        }
    }
}

#[test]
fn setting_keys_are_case_insensitive_and_duplicates_are_rejected() {
    for (lower, upper) in [
        ("app.tenant", "APP.TENANT"),
        ("timezone", "TimeZone"),
        ("application_name", "APPLICATION_NAME"),
    ] {
        for (first, second) in [(lower, upper), (upper, lower)] {
            for value in ["first", "different"] {
                assert!(
                    profile(vec!["public".into()])
                        .unwrap()
                        .with_setting(first, "first")
                        .unwrap()
                        .with_setting(second, value)
                        .is_err(),
                    "case-variant duplicate accepted: {first}, {second}"
                );
            }
        }
    }
    for forbidden in ["ROLE", "Search_Path", "Statement_Timeout", "ROW_SECURITY"] {
        assert!(
            profile(vec!["public".into()])
                .unwrap()
                .with_setting(forbidden, "value")
                .is_err()
        );
    }
}

#[tokio::test]
async fn profiled_pool_rejects_zero_capacity_before_connecting() {
    let options = "postgres://login@127.0.0.1:1/unused".parse().unwrap();
    assert!(matches!(
        PgProfiledPool::connect(options, profile(vec!["public".into()]).unwrap(), 0).await,
        Err(sqlx::Error::Protocol(_))
    ));
}
