//! The explicit PostgreSQL endpoint boundary: a URL's host or socket, or
//! explicit setters, select the server; `hostaddr` never does.

use std::{ffi::OsString, fmt};

use sqlx::postgres::PgConnectOptions;

const AMBIENT_HOST_ADDRESS: &str = "PGHOSTADDR";

/// Parse a PostgreSQL URL into native options that connect to the URL's host
/// or socket.
///
/// SQLx 0.9.1 treats `hostaddr` differently from SQLx 0.9.0 and from libpq:
/// it reads an ambient `PGHOSTADDR` into every URL parse and connects there
/// instead of the URL host; it checks a Unix socket (a `host=/dir` parameter,
/// or `PGHOST` or local socket discovery for a URL with no host) before an
/// explicit `hostaddr`; and the parsed options expose no way to read or clear
/// the address. This boundary therefore selects servers only by host or socket.
/// It refuses, inside `sqlx::Error::Configuration` and before parsing, with
/// [`AmbientHostAddress`] whenever `PGHOSTADDR` is present, whatever its value,
/// and with [`DeclaredHostAddress`] whenever the URL's query declares
/// `hostaddr`. Any other parse failure is SQLx's own error.
///
/// Use it instead of `str::parse`, `PgConnectOptions::from_str` or the
/// URL-taking `PgPool::connect`, `PgPoolOptions::connect`, `connect_lazy`,
/// `PgConnection::connect` and the `MigrateDatabase` maintenance methods, and
/// pass the result to the pool's or connection's `_with` variant. For a
/// listener, build a pool from these options and pass `&pool` to
/// `PgListener::connect_with` instead of calling `PgListener::connect(url)`. Other PG* variables remain native defaults that explicit
/// URL parts and setters override. The process must not change its environment
/// between this check and connection. Formatting a refusal names no address;
/// the returned options can still expose credentials through `Debug` and URL
/// conversion.
///
/// ```no_run
/// # async fn example(database_url: &str) -> Result<(), sqlx::Error> {
/// use sqlx::postgres::PgPoolOptions;
///
/// let options = batter_sqlx::connect_options(database_url)?;
/// let pool = PgPoolOptions::new().max_connections(1).connect_lazy_with(options);
/// # pool.close().await;
/// # Ok(())
/// # }
/// ```
pub fn connect_options(url: &str) -> Result<PgConnectOptions, sqlx::Error> {
    connect_options_with(url, |name| std::env::var_os(name))
}

/// Fresh native options for explicit setters, built only while no ambient
/// `PGHOSTADDR` would redirect them.
///
/// SQLx starts `PgConnectOptions::new_without_pgpass` from the process
/// environment, and a later `host` setter does not clear an ambient address
/// captured then. Checking the environment and constructing the options in one
/// call means the returned value cannot hold an address captured earlier:
/// build setter-based connecting options from this, never from
/// `new_without_pgpass` or `new` directly. Returns the same
/// [`AmbientHostAddress`] refusal as [`connect_options`]. The passfile is not
/// read; do not call the native `hostaddr` setter (Clippy rejects it here).
///
/// Migration: replaces `refuse_ambient_host_address()` followed by
/// `PgConnectOptions::new_without_pgpass()`, which an earlier revision of this
/// unreleased boundary required as two calls in order.
///
/// ```
/// # fn example() -> Result<(), sqlx::Error> {
/// let options = batter_sqlx::setter_connect_options()?
///     .host("127.0.0.1")
///     .port(5432);
/// # drop(options);
/// # Ok(())
/// # }
/// ```
pub fn setter_connect_options() -> Result<PgConnectOptions, sqlx::Error> {
    refuse_ambient_host_address_with(|name| std::env::var_os(name))?;
    Ok(PgConnectOptions::new_without_pgpass())
}

pub(crate) fn connect_options_with(
    url: &str,
    environment: impl FnOnce(&str) -> Option<OsString>,
) -> Result<PgConnectOptions, sqlx::Error> {
    refuse_ambient_host_address_with(environment)?;
    if declares_host_address(url) {
        return Err(sqlx::Error::Configuration(Box::new(DeclaredHostAddress)));
    }
    url.parse()
}

/// Whether the URL's own query names `hostaddr`, decoded as SQLx decodes it.
/// Text the URL parser rejects selects no address; SQLx then reports the
/// parse failure itself.
fn declares_host_address(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| url.query_pairs().any(|(key, _)| key == "hostaddr"))
}

fn refuse_ambient_host_address_with(
    environment: impl FnOnce(&str) -> Option<OsString>,
) -> Result<(), sqlx::Error> {
    if environment(AMBIENT_HOST_ADDRESS).is_some() {
        return Err(sqlx::Error::Configuration(Box::new(AmbientHostAddress)));
    }
    Ok(())
}

/// The process environment sets `PGHOSTADDR`, which would redirect a
/// connection away from the explicitly selected host.
///
/// Returned inside `sqlx::Error::Configuration` by [`connect_options`],
/// [`setter_connect_options`], fixture resolution and template
/// preparation, before any parse, pool or connection exists. Unset
/// `PGHOSTADDR` and select the server with the URL host. Formatting names no
/// address.
///
/// ```
/// # fn example(error: &sqlx::Error) -> bool {
/// use batter_sqlx::AmbientHostAddress;
/// matches!(error, sqlx::Error::Configuration(source) if source.is::<AmbientHostAddress>())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmbientHostAddress;

impl fmt::Display for AmbientHostAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PGHOSTADDR is set and would redirect the connection from the selected host")
    }
}

impl std::error::Error for AmbientHostAddress {}

/// A PostgreSQL URL declares `hostaddr`, which SQLx 0.9.1 can silently replace
/// with a Unix socket or carry differently from the URL host.
///
/// Returned inside `sqlx::Error::Configuration` by [`connect_options`],
/// fixture resolution and template preparation, before parsing. Remove the
/// parameter and put the address in the URL host instead. Formatting names no
/// address.
///
/// ```
/// # fn example(error: &sqlx::Error) -> bool {
/// use batter_sqlx::DeclaredHostAddress;
/// matches!(error, sqlx::Error::Configuration(source) if source.is::<DeclaredHostAddress>())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclaredHostAddress;

impl fmt::Display for DeclaredHostAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the URL declares hostaddr; select the server with the URL host instead")
    }
}

impl std::error::Error for DeclaredHostAddress {}

#[cfg(test)]
mod tests {
    use super::*;

    fn is<T: std::error::Error + 'static>(error: &sqlx::Error) -> bool {
        matches!(error, sqlx::Error::Configuration(source) if source.is::<T>())
    }

    const URL: &str = "postgres://user:secret-marker@127.0.0.1:5432/database?sslmode=disable";

    #[test]
    fn ambient_host_address_refuses_before_parsing_whatever_its_value() {
        for value in ["192.0.2.1", "", "not-an-address"] {
            let mut asked = Vec::new();
            let error = connect_options_with(URL, |name| {
                asked.push(name.to_owned());
                Some(value.into())
            })
            .unwrap_err();
            assert_eq!(
                asked,
                ["PGHOSTADDR"],
                "only the redirecting variable is read"
            );
            assert!(
                is::<AmbientHostAddress>(&error),
                "refusal is AmbientHostAddress"
            );
            let text = error.to_string();
            assert!(
                !text.contains("192.0.2.1") && !text.contains("secret-marker"),
                "refusal names neither address nor credentials"
            );
        }
        let error = connect_options_with("not a url", |_| Some("192.0.2.1".into())).unwrap_err();
        assert!(is::<AmbientHostAddress>(&error), "refusal precedes parsing");
    }

    #[test]
    fn declared_host_address_is_refused_in_every_spelling() {
        for url in [
            "postgres://user:secret-marker@127.0.0.1/db?hostaddr=192.0.2.1",
            "postgres://user@localhost/db?host=/tmp&hostaddr=127.0.0.1",
            "postgres:///db?hostaddr=127.0.0.1",
            "postgres://user@localhost/db?host%61ddr=127.0.0.1",
            "postgres://user@localhost/db?sslmode=disable&hostaddr=not-an-ip",
        ] {
            let error = connect_options_with(url, |_| None).unwrap_err();
            assert!(is::<DeclaredHostAddress>(&error), "{url}");
            let text = error.to_string();
            assert!(
                !text.contains("192.0.2.1") && !text.contains("secret-marker"),
                "refusal names neither address nor credentials"
            );
        }
    }

    #[test]
    fn host_and_socket_urls_parse() {
        let options = connect_options_with(URL, |_| None).unwrap();
        assert_eq!(options.get_host(), "127.0.0.1", "URL host is kept");
        assert_eq!(
            options.get_database(),
            Some("database"),
            "URL database is kept"
        );
        let socket = connect_options_with("postgres://user@localhost/db?host=/tmp", |_| None);
        assert!(socket.is_ok(), "socket-only URLs are accepted");
        assert!(
            matches!(connect_options_with("not a url", |_| None), Err(error)
                if !is::<AmbientHostAddress>(&error) && !is::<DeclaredHostAddress>(&error)),
            "parse failures stay SQLx's own"
        );
    }
}
