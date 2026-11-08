//! Application settings: names, defaults and validation live at this root.
//!
//! | Name | Mode | Meaning |
//! | --- | --- | --- |
//! | `DATABASE_URL` | serve | Serving login: application tables, Runledger and the flags store. |
//! | `CHECKOUT_MIGRATION_DATABASE_URL` | migrate | Schema-owner login for `checkout-service migrate`. |
//! | `CHECKOUT_FLAGS_LAST_GOOD_DIR` | serve | Existing directory for the flags last-good file. |
//! | `CHECKOUT_BIND` | serve | Listener address, default `127.0.0.1:8080`. |
//! | `CHECKOUT_REQUEST_TIMEOUT_MS` | serve | Response-construction budget, default 2000. |
//! | `CHECKOUT_WORKER_ID` | serve | Runledger worker identity, default `checkout-service`. |
//!
//! `FLAG_*` names belong to the flags catalog (`src/flags.rs`). Ambient `PG*`
//! variables are refused so connection options come only from the URL.
//!
//! No TLS provider is compiled in (the offline lockfile resolves none), so
//! `sslmode` must be `disable` or `prefer` against a local server. A deployment
//! adds a TLS feature to the `sqlx` dependency and selects `verify-full`.

use std::{fmt, net::SocketAddr, path::PathBuf, time::Duration};

use batter::{
    axum::ResponseConstructionBudget,
    settings::{SettingsError, SettingsSource, milliseconds},
};
use sqlx::postgres::PgConnectOptions;

const NAMES: &[&str] = &[
    "DATABASE_URL",
    "CHECKOUT_MIGRATION_DATABASE_URL",
    "CHECKOUT_FLAGS_LAST_GOOD_DIR",
    "CHECKOUT_BIND",
    "CHECKOUT_REQUEST_TIMEOUT_MS",
    "CHECKOUT_WORKER_ID",
];

/// A PostgreSQL login: its parsed options and the role name the session
/// profile declares.
#[derive(Clone)]
pub struct Login {
    pub options: PgConnectOptions,
    pub role: String,
}

impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Login([REDACTED])")
    }
}

/// Everything the serving process reads.
pub struct ServeConfig {
    pub database: Login,
    pub last_good_dir: PathBuf,
    pub bind: SocketAddr,
    pub request_budget: ResponseConstructionBudget,
    pub worker_id: String,
}

impl fmt::Debug for ServeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServeConfig([REDACTED])")
    }
}

/// Everything `migrate` reads.
pub struct MigrateConfig {
    pub database: Login,
}

impl fmt::Debug for MigrateConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MigrateConfig([REDACTED])")
    }
}

fn narrowed(captured: &SettingsSource) -> Result<SettingsSource, SettingsError> {
    // `select_environment` rejects unknown names under a reserved prefix; with
    // no allowed names it rejects every ambient `PG*` variable.
    captured
        .select_environment(&[], &["PG"])
        .map_err(|_| SettingsError::new("PG*", "ambient libpq variables are not accepted"))?;
    captured.select_environment(NAMES, &["CHECKOUT_"])
}

fn login(values: &SettingsSource, name: &'static str) -> Result<Login, SettingsError> {
    let options: PgConnectOptions = values
        .required(name)?
        .parse()
        .map_err(|error| SettingsError::new(name, "invalid PostgreSQL URL").with_cause(error))?;
    let role = options.get_username().to_owned();
    if role.is_empty() {
        return Err(SettingsError::new(name, "the URL must name a login"));
    }
    Ok(Login { options, role })
}

impl ServeConfig {
    pub fn from_settings(captured: &SettingsSource) -> Result<Self, SettingsError> {
        let mut values = SettingsSource::from_pairs([
            ("CHECKOUT_BIND".into(), "127.0.0.1:8080".into()),
            ("CHECKOUT_REQUEST_TIMEOUT_MS".into(), "2000".into()),
            ("CHECKOUT_WORKER_ID".into(), "checkout-service".into()),
        ])?;
        values.overlay(narrowed(captured)?);
        let bind = values
            .required("CHECKOUT_BIND")?
            .parse()
            .map_err(|e| SettingsError::new("CHECKOUT_BIND", "invalid address").with_cause(e))?;
        let budget = milliseconds(
            values.required("CHECKOUT_REQUEST_TIMEOUT_MS")?,
            "CHECKOUT_REQUEST_TIMEOUT_MS",
            Duration::from_secs(60),
        )?;
        let request_budget = ResponseConstructionBudget::new(budget).map_err(|e| {
            SettingsError::new("CHECKOUT_REQUEST_TIMEOUT_MS", "invalid duration").with_cause(e)
        })?;
        let worker_id = values.required("CHECKOUT_WORKER_ID")?;
        if worker_id.is_empty() || worker_id.len() > 128 {
            return Err(SettingsError::new(
                "CHECKOUT_WORKER_ID",
                "must be 1 to 128 bytes",
            ));
        }
        let last_good_dir = PathBuf::from(values.required("CHECKOUT_FLAGS_LAST_GOOD_DIR")?);
        if !last_good_dir.is_absolute() {
            return Err(SettingsError::new(
                "CHECKOUT_FLAGS_LAST_GOOD_DIR",
                "must be an absolute path",
            ));
        }
        Ok(Self {
            database: login(&values, "DATABASE_URL")?,
            last_good_dir,
            bind,
            request_budget,
            worker_id: worker_id.to_owned(),
        })
    }
}

impl MigrateConfig {
    pub fn from_settings(captured: &SettingsSource) -> Result<Self, SettingsError> {
        let values = narrowed(captured)?;
        Ok(Self {
            database: login(&values, "CHECKOUT_MIGRATION_DATABASE_URL")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(pairs: &[(&str, &str)]) -> SettingsSource {
        SettingsSource::from_pairs(pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())))
            .expect("distinct names")
    }

    #[test]
    fn serve_settings_apply_defaults_and_require_the_database_and_last_good_dir() {
        let config = ServeConfig::from_settings(&source(&[
            ("DATABASE_URL", "postgres://checkout@127.0.0.1/checkout"),
            ("CHECKOUT_FLAGS_LAST_GOOD_DIR", "/var/lib/checkout"),
            ("PATH", "/bin"),
        ]))
        .expect("valid settings");
        assert_eq!(config.bind, "127.0.0.1:8080".parse().unwrap());
        assert_eq!(config.database.role, "checkout");
        assert_eq!(config.worker_id, "checkout-service");

        let missing = ServeConfig::from_settings(&source(&[(
            "CHECKOUT_FLAGS_LAST_GOOD_DIR",
            "/var/lib/checkout",
        )]))
        .unwrap_err();
        assert_eq!(missing.field(), "DATABASE_URL");
    }

    #[test]
    fn ambient_libpq_variables_and_unknown_checkout_names_are_refused() {
        let base = [
            ("DATABASE_URL", "postgres://checkout@127.0.0.1/checkout"),
            ("CHECKOUT_FLAGS_LAST_GOOD_DIR", "/var/lib/checkout"),
        ];
        let mut with_pg = base.to_vec();
        with_pg.push(("PGHOST", "elsewhere"));
        assert_eq!(
            ServeConfig::from_settings(&source(&with_pg))
                .unwrap_err()
                .field(),
            "PG*"
        );
        let mut typo = base.to_vec();
        typo.push(("CHECKOUT_BINDD", "0.0.0.0:80"));
        assert!(ServeConfig::from_settings(&source(&typo)).is_err());
    }

    #[test]
    fn diagnostics_never_show_the_url() {
        let config = MigrateConfig::from_settings(&source(&[(
            "CHECKOUT_MIGRATION_DATABASE_URL",
            "postgres://owner:secret-password@127.0.0.1/checkout",
        )]))
        .expect("valid settings");
        assert!(!format!("{config:?}").contains("secret-password"));
        let invalid = MigrateConfig::from_settings(&source(&[(
            "CHECKOUT_MIGRATION_DATABASE_URL",
            "not a url with secret-password",
        )]))
        .unwrap_err();
        assert!(!invalid.to_string().contains("secret-password"));
    }
}
