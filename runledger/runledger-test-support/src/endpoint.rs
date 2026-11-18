//! Fixture URLs select their server, never an ambient `PGHOSTADDR`.

use std::{ffi::OsString, fmt};

use sqlx::postgres::PgConnectOptions;

pub(crate) const AMBIENT_HOST_ADDRESS_ENV: &str = "PGHOSTADDR";

/// Parse a fixture URL into options that connect to the URL's host.
///
/// SQLx 0.9.1, like libpq, starts every URL parse from the process environment
/// and connects to an ambient `PGHOSTADDR` instead of the URL host when the URL
/// has no `hostaddr` parameter. A fixture URL names the server this crate owns
/// or was given, so this refuses with [`AmbientHostAddress`] inside
/// `sqlx::Error::Configuration` whenever the variable is present, whatever its
/// value, before parsing. Every fixture connection in this crate uses it; use it,
/// or [`EphemeralDatabase::connect_options`](crate::EphemeralDatabase::connect_options),
/// instead of passing fixture URLs to SQLx's URL-taking `connect` forms. The
/// process must not change its environment between this check and connection.
///
/// ```
/// # fn example(url: &str) -> Result<(), sqlx::Error> {
/// let options = runledger_test_support::connect_options(url)?;
/// let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(options);
/// # drop(pool);
/// # Ok(())
/// # }
/// ```
pub fn connect_options(url: &str) -> Result<PgConnectOptions, sqlx::Error> {
    connect_options_with(url, |name| std::env::var_os(name))
}

pub(crate) fn connect_options_with(
    url: &str,
    environment: impl FnOnce(&str) -> Option<OsString>,
) -> Result<PgConnectOptions, sqlx::Error> {
    if environment(AMBIENT_HOST_ADDRESS_ENV).is_some() {
        return Err(sqlx::Error::Configuration(Box::new(AmbientHostAddress)));
    }
    url.parse()
}

/// The process environment sets `PGHOSTADDR`, which would redirect a fixture
/// connection away from the fixture URL's host.
///
/// Returned inside `sqlx::Error::Configuration` by [`connect_options`] and by
/// every fixture connection in this crate. Unset `PGHOSTADDR` for native test
/// runs. Formatting names no address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmbientHostAddress;

impl fmt::Display for AmbientHostAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PGHOSTADDR is set and would redirect the fixture connection from its host")
    }
}

impl std::error::Error for AmbientHostAddress {}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://runledger:secret-marker@127.0.0.1:5432/postgres";

    fn refused(error: &sqlx::Error) -> bool {
        matches!(error, sqlx::Error::Configuration(source) if source.is::<AmbientHostAddress>())
    }

    #[test]
    fn ambient_host_address_refuses_before_parsing_whatever_its_value() {
        for value in ["192.0.2.1", "", "not-an-address"] {
            let error = connect_options_with(URL, |name| {
                assert_eq!(name, AMBIENT_HOST_ADDRESS_ENV, "only PGHOSTADDR is read");
                Some(value.into())
            })
            .expect_err("PGHOSTADDR present");
            assert!(refused(&error), "refusal is AmbientHostAddress");
            assert!(
                !error.to_string().contains("secret-marker"),
                "refusal names no credentials"
            );
        }
        let error = connect_options_with("not a url", |_| Some("192.0.2.1".into()))
            .expect_err("PGHOSTADDR present");
        assert!(refused(&error), "refusal precedes parsing");
    }

    #[test]
    fn absent_host_address_parses_the_fixture_url() {
        let options = connect_options_with(URL, |_| None).expect("valid URL");
        assert_eq!(options.get_host(), "127.0.0.1", "URL host is kept");
    }
}
