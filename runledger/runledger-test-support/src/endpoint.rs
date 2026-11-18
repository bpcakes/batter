//! Fixture URLs select their server by host or socket, never by `hostaddr`.

use std::{ffi::OsString, fmt};

use sqlx::postgres::PgConnectOptions;

pub(crate) const AMBIENT_HOST_ADDRESS_ENV: &str = "PGHOSTADDR";

/// Parse a fixture URL into options that connect to the URL's host or socket.
///
/// SQLx 0.9.1 reads an ambient `PGHOSTADDR` into every URL parse and connects
/// there instead of the URL host, and checks a Unix socket before a URL's
/// `hostaddr`. A fixture URL names the server this crate owns or was given, so
/// this refuses inside `sqlx::Error::Configuration`, before parsing, with
/// [`AmbientHostAddress`] whenever `PGHOSTADDR` is present and with
/// [`DeclaredHostAddress`] whenever the URL declares `hostaddr`, whatever the
/// value. Every fixture connection in this crate uses it; use it, or
/// [`EphemeralDatabase::connect_options`](crate::EphemeralDatabase::connect_options),
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
    if declares_host_address(url) {
        return Err(sqlx::Error::Configuration(Box::new(DeclaredHostAddress)));
    }
    url.parse()
}

/// Whether the URL's own query names `hostaddr`, decoded as SQLx decodes it.
pub(crate) fn declares_host_address(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| url.query_pairs().any(|(key, _)| key == "hostaddr"))
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

/// A fixture URL declares `hostaddr`, which SQLx 0.9.1 can silently replace
/// with a Unix socket or carry differently from the URL host. Remove it and
/// select the server with the URL host. Formatting names no address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclaredHostAddress;

impl fmt::Display for DeclaredHostAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the fixture URL declares hostaddr; select the server with its host instead")
    }
}

impl std::error::Error for DeclaredHostAddress {}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://runledger:secret-marker@127.0.0.1:5432/postgres";

    fn is<T: std::error::Error + 'static>(error: &sqlx::Error) -> bool {
        matches!(error, sqlx::Error::Configuration(source) if source.is::<T>())
    }

    #[test]
    fn ambient_host_address_refuses_before_parsing_whatever_its_value() {
        for value in ["192.0.2.1", "", "not-an-address"] {
            let error = connect_options_with(URL, |name| {
                assert_eq!(name, AMBIENT_HOST_ADDRESS_ENV, "only PGHOSTADDR is read");
                Some(value.into())
            })
            .expect_err("PGHOSTADDR present");
            assert!(
                is::<AmbientHostAddress>(&error),
                "refusal is AmbientHostAddress"
            );
            assert!(
                !error.to_string().contains("secret-marker"),
                "refusal names no credentials"
            );
        }
        let error = connect_options_with("not a url", |_| Some("192.0.2.1".into()))
            .expect_err("PGHOSTADDR present");
        assert!(is::<AmbientHostAddress>(&error), "refusal precedes parsing");
    }

    #[test]
    fn declared_host_address_is_refused_in_every_spelling() {
        for url in [
            "postgres://u@127.0.0.1/db?hostaddr=192.0.2.1",
            "postgres://u@localhost/db?host=/tmp&hostaddr=127.0.0.1",
            "postgres:///db?host%61ddr=127.0.0.1",
        ] {
            let error = connect_options_with(url, |_| None).expect_err("hostaddr declared");
            assert!(is::<DeclaredHostAddress>(&error), "{url}");
        }
    }

    #[test]
    fn host_and_socket_fixture_urls_parse() {
        let options = connect_options_with(URL, |_| None).expect("valid URL");
        assert_eq!(options.get_host(), "127.0.0.1", "URL host is kept");
        connect_options_with("postgres://u@localhost/db?host=/tmp", |_| None).expect("socket only");
    }
}
