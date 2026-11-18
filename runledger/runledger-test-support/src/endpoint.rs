//! Fixture URLs select their server through a canonical authority that SQLx,
//! tokio-postgres and libpq read alike.

use std::{collections::BTreeSet, ffi::OsString, fmt};

use sqlx::postgres::PgConnectOptions;

pub(crate) const AMBIENT_HOST_ADDRESS_ENV: &str = "PGHOSTADDR";

/// Parse a canonical fixture URL into options that connect to the server its
/// authority names.
///
/// SQLx 0.9.1 reads an ambient `PGHOSTADDR` into every URL parse and connects
/// there instead of the URL host, checks a Unix socket before a declared
/// `hostaddr`, and treats a query `host` or `port` as a replacement where
/// tokio-postgres treats it as another candidate. A fixture URL names the
/// server this crate owns or was given, so this refuses inside
/// `sqlx::Error::Configuration`, before parsing, with [`AmbientHostAddress`]
/// whenever `PGHOSTADDR` is present and with [`NoncanonicalUrl`] unless the URL
/// is a `postgres://` or `postgresql://` URI without `#`, names one host (or an
/// empty host with a `host=/socket/dir` parameter), and declares no
/// `hostaddr`, `port` or `service`, no `host` beside an authority host and no
/// parameter twice. Every fixture connection in this crate uses it; use it, or
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
    if !canonical(url) {
        return Err(sqlx::Error::Configuration(Box::new(NoncanonicalUrl)));
    }
    url.parse()
}

/// The URL subset SQLx, tokio-postgres and libpq read alike.
fn canonical(url: &str) -> bool {
    if url.contains('#') {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default();
    if !matches!(parsed.scheme(), "postgres" | "postgresql") || host.contains(',') {
        return false;
    }
    let mut seen = BTreeSet::new();
    for (key, value) in parsed.query_pairs() {
        let endpoint = match &*key {
            "hostaddr" | "port" | "service" => true,
            "host" => !host.is_empty() || !value.starts_with('/'),
            _ => false,
        };
        if endpoint || !seen.insert(key.into_owned()) {
            return false;
        }
    }
    !host.is_empty() || seen.contains("host")
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

/// A fixture URL outside the subset SQLx, tokio-postgres and libpq read alike;
/// see [`connect_options`]. Put the endpoint in the URL authority. Formatting
/// names no URL part.
///
/// Migration: replaces `DeclaredHostAddress`, which an earlier revision of
/// this unreleased boundary returned only for a declared `hostaddr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NoncanonicalUrl;

impl fmt::Display for NoncanonicalUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "the fixture URL must be a postgres:// URI that selects its server in the authority",
        )
    }
}

impl std::error::Error for NoncanonicalUrl {}

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
    fn noncanonical_fixture_urls_are_refused() {
        for url in [
            "host=db-a hostaddr=192.0.2.1 dbname=postgres",
            "postgres://u@127.0.0.1/db#tag",
            "postgres://u@db-a,db-b/db",
            "postgres:///db",
            "postgres://u@127.0.0.1/db?hostaddr=192.0.2.1",
            "postgres://u@localhost/db?host=/tmp",
            "postgres://u@127.0.0.1:5432/postgres?port=5432",
            "postgres:///db?host%61ddr=127.0.0.1&host=/tmp",
            "postgres://u@127.0.0.1/db?sslmode=disable&sslmode=require",
        ] {
            let error = connect_options_with(url, |_| None).expect_err(url);
            assert!(is::<NoncanonicalUrl>(&error), "{url}");
        }
    }

    #[test]
    fn canonical_host_and_socket_fixture_urls_parse() {
        let options = connect_options_with(URL, |_| None).expect("valid URL");
        assert_eq!(options.get_host(), "127.0.0.1", "URL host is kept");
        for url in [
            "postgres:///db?host=/tmp&user=u",
            "postgres://u@%2Ftmp/db",
            "postgres://u@127.0.0.1/db?sslmode=disable&dbname=db",
        ] {
            connect_options_with(url, |_| None).expect(url);
        }
    }
}
