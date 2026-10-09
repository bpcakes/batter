//! Typed settings read from explicit sources. Nothing here acquires a resource.

use batter::axum::ResponseConstructionBudget;
use batter::runledger::native::runtime::config::JobsConfig;
use batter::settings::{SecretString, SettingsError, SettingsSource, milliseconds};
use sqlx::postgres::PgConnectOptions;
use std::{ffi::OsString, net::SocketAddr, str::FromStr, time::Duration};

/// Every environment name this service reads. Unknown `RECORDS_` names are rejected.
const NAMES: &[&str] = &[
    "RECORDS_BIND",
    "DATABASE_URL",
    "RECORDS_REQUEST_TIMEOUT_MS",
    "RECORDS_WORKER_ID",
    "RUST_LOG",
];

const RESERVED_PREFIXES: &[&str] = &["RECORDS_"];

/// Longest request budget this service accepts.
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Validated service settings. Debug and Display never print values.
pub struct Settings {
    pub bind: SocketAddr,
    pub database: DatabaseSettings,
    pub request_budget: ResponseConstructionBudget,
    pub jobs: JobsConfig,
    pub log_filter: Option<String>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Settings([REDACTED])")
    }
}

/// The parsed connection target. The URL is a secret; the login role is not.
pub struct DatabaseSettings {
    url: SecretString,
    login: String,
}

impl DatabaseSettings {
    /// Re-parse the validated URL into native connect options for a trusted consumer.
    pub fn connect_options(&self) -> PgConnectOptions {
        PgConnectOptions::from_str(self.url.expose_secret())
            .expect("DATABASE_URL was validated when settings were loaded")
    }

    /// The authenticated PostgreSQL login role, used as the declared session profile.
    pub fn login(&self) -> &str {
        &self.login
    }
}

impl std::fmt::Debug for DatabaseSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DatabaseSettings([REDACTED])")
    }
}

impl Settings {
    /// Read the process environment. Invalid values fail here, before any pool exists.
    pub fn load() -> Result<Self, SettingsError> {
        let environment = SettingsSource::from_pairs(std::env::vars_os())?;
        Self::from_environment(environment)
    }

    /// Validate one explicit source. Defaults live here; the source may override them.
    pub fn from_environment(environment: SettingsSource) -> Result<Self, SettingsError> {
        let mut values = SettingsSource::from_pairs([
            (OsString::from("RECORDS_BIND"), "127.0.0.1:3000".into()),
            ("RECORDS_REQUEST_TIMEOUT_MS".into(), "2000".into()),
        ])?;
        values.overlay(environment.select(NAMES, RESERVED_PREFIXES, true)?);

        let bind = values.required("RECORDS_BIND")?.parse().map_err(|e| {
            SettingsError::new("RECORDS_BIND", "invalid socket address").with_cause(e)
        })?;

        let request_budget = milliseconds(
            values.required("RECORDS_REQUEST_TIMEOUT_MS")?,
            "RECORDS_REQUEST_TIMEOUT_MS",
            MAX_REQUEST_TIMEOUT,
        )?;
        let request_budget = ResponseConstructionBudget::new(request_budget).map_err(|e| {
            SettingsError::new("RECORDS_REQUEST_TIMEOUT_MS", "invalid duration").with_cause(e)
        })?;

        let database = DatabaseSettings::parse(values.required("DATABASE_URL")?)?;

        let worker_id = values.required("RECORDS_WORKER_ID")?.trim();
        if worker_id.is_empty() || worker_id.len() > 64 {
            return Err(SettingsError::new(
                "RECORDS_WORKER_ID",
                "expected 1 to 64 bytes of non-blank text",
            ));
        }
        let jobs = JobsConfig {
            worker_id: worker_id.to_owned(),
            poll_interval: Duration::from_millis(500),
            claim_batch_size: 16,
            lease_ttl_seconds: 60,
            max_global_concurrency: 4,
            reaper_interval: Duration::from_secs(15),
            schedule_poll_interval: Duration::from_secs(30),
            reaper_retry_delay_ms: 30_000,
        };
        jobs.validate().map_err(|e| {
            SettingsError::new("RECORDS_WORKER_ID", "invalid worker configuration").with_cause(e)
        })?;

        Ok(Self {
            bind,
            database,
            request_budget,
            jobs,
            log_filter: values.text("RUST_LOG")?.map(str::to_owned),
        })
    }
}

impl DatabaseSettings {
    fn parse(url: &str) -> Result<Self, SettingsError> {
        let options = PgConnectOptions::from_str(url).map_err(|e| {
            SettingsError::new("DATABASE_URL", "invalid PostgreSQL URL").with_cause(e)
        })?;
        let login = options.get_username().to_owned();
        if login.is_empty() {
            return Err(SettingsError::new("DATABASE_URL", "missing login role"));
        }
        Ok(Self {
            url: SecretString::new(url),
            login,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(pairs: &[(&str, &str)]) -> SettingsSource {
        SettingsSource::from_pairs(
            pairs
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v))),
        )
        .expect("test pairs are unique")
    }

    const URL: &str = "postgres://records:secret@localhost:5432/records";

    #[test]
    fn loads_defaults_and_required_values() {
        let settings = Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", " worker-a "),
        ]))
        .expect("valid settings");
        assert_eq!(
            settings.bind,
            "127.0.0.1:3000".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(settings.request_budget.get(), Duration::from_millis(2000));
        assert_eq!(settings.jobs.worker_id, "worker-a");
        assert_eq!(settings.database.login(), "records");
        assert_eq!(
            settings.database.connect_options().get_username(),
            "records"
        );
        assert!(settings.log_filter.is_none());
    }

    #[test]
    fn overrides_replace_defaults() {
        let settings = Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", "w"),
            ("RECORDS_BIND", "0.0.0.0:8080"),
            ("RECORDS_REQUEST_TIMEOUT_MS", "250"),
            ("RUST_LOG", "debug"),
        ]))
        .expect("valid settings");
        assert_eq!(settings.bind.port(), 8080);
        assert_eq!(settings.request_budget.get(), Duration::from_millis(250));
        assert_eq!(settings.log_filter.as_deref(), Some("debug"));
    }

    #[test]
    fn rejects_missing_database_url() {
        let error = Settings::from_environment(source(&[("RECORDS_WORKER_ID", "w")]))
            .expect_err("missing secret");
        assert_eq!(error.field(), "DATABASE_URL");
    }

    #[test]
    fn rejects_malformed_database_url_without_echoing_it() {
        let error = Settings::from_environment(source(&[
            ("DATABASE_URL", "not a url with password hunter2"),
            ("RECORDS_WORKER_ID", "w"),
        ]))
        .expect_err("malformed url");
        assert_eq!(error.field(), "DATABASE_URL");
        assert!(!error.to_string().contains("hunter2"));
    }

    #[test]
    fn rejects_zero_and_oversized_timeouts() {
        for value in ["0", "-5", "abc", "999999999"] {
            let error = Settings::from_environment(source(&[
                ("DATABASE_URL", URL),
                ("RECORDS_WORKER_ID", "w"),
                ("RECORDS_REQUEST_TIMEOUT_MS", value),
            ]))
            .expect_err("invalid timeout");
            assert_eq!(error.field(), "RECORDS_REQUEST_TIMEOUT_MS", "value {value}");
        }
    }

    #[test]
    fn rejects_blank_worker_id() {
        let error = Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", "   "),
        ]))
        .expect_err("blank worker id");
        assert_eq!(error.field(), "RECORDS_WORKER_ID");
    }

    #[test]
    fn rejects_invalid_bind_address() {
        let error = Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", "w"),
            ("RECORDS_BIND", "localhost"),
        ]))
        .expect_err("hostnames are not socket addresses");
        assert_eq!(error.field(), "RECORDS_BIND");
    }

    #[test]
    fn rejects_unknown_reserved_names() {
        let error = Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", "w"),
            ("RECORDS_TYPO", "1"),
        ]))
        .expect_err("unknown reserved name");
        assert_eq!(error.field(), "source");
    }

    #[test]
    fn unrelated_environment_names_are_ignored() {
        Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", "w"),
            ("HOME", "/nowhere"),
        ]))
        .expect("unrelated names are ignored");
    }

    #[test]
    fn debug_output_is_redacted() {
        let settings = Settings::from_environment(source(&[
            ("DATABASE_URL", URL),
            ("RECORDS_WORKER_ID", "w"),
        ]))
        .expect("valid settings");
        let rendered = format!("{settings:?} {:?}", settings.database);
        assert!(!rendered.contains("secret"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
