use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

const WORKER_NUMBERS: &[(&str, &str, &str)] = &[
    ("JOBS_POLL_INTERVAL_MS", "1", "0"),
    ("JOBS_CLAIM_BATCH_SIZE", "1", "1001"),
    ("JOBS_LEASE_TTL_SECONDS", "10", "9"),
    ("JOBS_MAX_GLOBAL_CONCURRENCY", "1", "0"),
    ("JOBS_REAPER_INTERVAL_SECONDS", "1", "0"),
    ("JOBS_SCHEDULE_POLL_INTERVAL_SECONDS", "1", "0"),
    ("JOBS_REAPER_RETRY_DELAY_MS", "1000", "999"),
];
const PROMOTER_NUMBERS: &[(&str, &str, &str)] = &[
    ("JOBS_INTENT_PROMOTER_POLL_INTERVAL_MS", "1", "0"),
    ("JOBS_INTENT_PROMOTER_BATCH_SIZE", "1", "1001"),
];

fn lookup<'a>(
    setting: &'a str,
    value: &'a str,
) -> impl FnMut(&str) -> Result<String, VarError> + 'a {
    move |name| {
        if name == setting {
            Ok(value.into())
        } else {
            Err(VarError::NotPresent)
        }
    }
}

fn assert_redacted(error: JobsEnvironmentError, setting: &str) {
    assert_eq!(error.setting(), setting);
    assert_eq!(
        error.to_string(),
        format!("invalid jobs environment setting: {setting}")
    );
    assert_eq!(
        format!("{error:?}"),
        format!("JobsEnvironmentError {{ setting: {setting:?} }}")
    );
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn every_worker_number_rejects_bad_values_instead_of_defaulting_or_clamping() {
    for &(setting, minimum, below) in WORKER_NUMBERS {
        for value in [
            "secret-marker",
            "",
            " ",
            "-1",
            "0",
            below,
            "999999999999999999999999999999999999",
        ] {
            let error = JobsConfig::from_lookup(&mut lookup(setting, value)).expect_err(setting);
            assert_redacted(error, setting);
        }
        JobsConfig::from_lookup(&mut lookup(setting, minimum)).expect("valid minimum");
    }
}

#[test]
fn every_promoter_number_rejects_bad_values_in_both_default_modes() {
    for &(setting, minimum, below) in PROMOTER_NUMBERS {
        for (poll, batch) in [
            (Duration::from_millis(500), 16),
            (Duration::from_millis(83), 7),
        ] {
            for value in [
                "secret-marker",
                "",
                " ",
                "-1",
                "0",
                below,
                "999999999999999999999999999999999999",
            ] {
                let error =
                    IntentPromoterConfig::from_lookup(poll, batch, &mut lookup(setting, value))
                        .expect_err(setting);
                assert_redacted(error, setting);
            }
            IntentPromoterConfig::from_lookup(poll, batch, &mut lookup(setting, minimum))
                .expect("valid minimum");
        }
    }
}

#[test]
fn invalid_unicode_is_an_error_for_every_setting() {
    for setting in WORKER_NUMBERS
        .iter()
        .map(|row| row.0)
        .chain(["JOBS_WORKER_ID"])
    {
        let mut lookup = |name: &str| {
            if name == setting {
                Err(VarError::NotUnicode(OsString::from_vec(vec![0xff])))
            } else {
                Err(VarError::NotPresent)
            }
        };
        assert_redacted(
            JobsConfig::from_lookup(&mut lookup).expect_err(setting),
            setting,
        );
    }
    for &(setting, _, _) in PROMOTER_NUMBERS {
        let mut lookup = |name: &str| {
            if name == setting {
                Err(VarError::NotUnicode(OsString::from_vec(vec![0xff])))
            } else {
                Err(VarError::NotPresent)
            }
        };
        assert_redacted(
            IntentPromoterConfig::from_lookup(Duration::from_millis(500), 16, &mut lookup)
                .expect_err(setting),
            setting,
        );
    }
}

#[test]
fn worker_id_defaults_only_when_absent() {
    for value in ["", " \t\n"] {
        assert_redacted(
            JobsConfig::from_lookup(&mut lookup("JOBS_WORKER_ID", value))
                .expect_err("blank worker"),
            "JOBS_WORKER_ID",
        );
    }
    let supplied =
        JobsConfig::from_lookup(&mut lookup("JOBS_WORKER_ID", "worker-id")).expect("explicit ID");
    assert_eq!(supplied.worker_id, "worker-id");
    let defaults = JobsConfig::from_lookup(&mut |_| Err(VarError::NotPresent)).expect("defaults");
    assert!(defaults.worker_id.starts_with("worker-"));
    assert_eq!(defaults.poll_interval, Duration::from_millis(500));
    assert_eq!(defaults.claim_batch_size, 16);
    assert_eq!(defaults.lease_ttl_seconds, 60);
    assert_eq!(defaults.max_global_concurrency, 32);
    assert_eq!(defaults.reaper_interval, Duration::from_secs(15));
    assert_eq!(defaults.schedule_poll_interval, Duration::from_secs(30));
    assert_eq!(defaults.reaper_retry_delay_ms, 30_000);
}

#[test]
fn independent_overrides_preserve_absent_inheritance_and_reject_invalid_inheritance() {
    let config = IntentPromoterConfig::from_lookup(
        Duration::from_millis(83),
        7,
        &mut lookup("JOBS_INTENT_PROMOTER_POLL_INTERVAL_MS", "37"),
    )
    .expect("override");
    assert_eq!(config.poll_interval(), Duration::from_millis(37));
    assert_eq!(config.batch_size(), 7);
    let config = IntentPromoterConfig::from_lookup(
        Duration::from_millis(83),
        7,
        &mut lookup("JOBS_INTENT_PROMOTER_BATCH_SIZE", "9"),
    )
    .expect("override");
    assert_eq!(config.poll_interval(), Duration::from_millis(83));
    assert_eq!(config.batch_size(), 9);
    for (poll, batch, setting) in [
        (Duration::ZERO, 7, "JOBS_INTENT_PROMOTER_POLL_INTERVAL_MS"),
        (
            Duration::from_millis(83),
            0,
            "JOBS_INTENT_PROMOTER_BATCH_SIZE",
        ),
    ] {
        assert_redacted(
            IntentPromoterConfig::from_lookup(poll, batch, &mut |_| Err(VarError::NotPresent))
                .expect_err("invalid inherited value"),
            setting,
        );
    }
}
