//! The explicit PostgreSQL endpoint boundary: a URL or explicit setters select
//! the server, never an ambient `PGHOSTADDR`.

use std::{ffi::OsString, fmt};

use sqlx::postgres::PgConnectOptions;

const AMBIENT_HOST_ADDRESS: &str = "PGHOSTADDR";

/// Parse a PostgreSQL URL into native options that connect to the URL's host.
///
/// SQLx 0.9.1, like libpq, starts every URL parse from the process
/// environment and connects to an ambient `PGHOSTADDR` instead of the URL host
/// whenever the URL has no `hostaddr` parameter. The parsed options expose no
/// way to read or clear that address, so a URL naming one server can log in to
/// another. This function refuses with [`AmbientHostAddress`] inside
/// `sqlx::Error::Configuration` whenever `PGHOSTADDR` is present, whatever its
/// value, before parsing; any other parse failure is SQLx's own error.
///
/// Use it instead of `str::parse`, `PgConnectOptions::from_str`, or the URL
/// forms `PgPool::connect`, `PgPoolOptions::connect`, `connect_lazy` and
/// `PgConnection::connect`; pass the result to their `_with` variants. Other
/// PG* variables remain native defaults that explicit URL parts and setters
/// override. The process must not change its environment between this check
/// and connection. Formatting the refusal names no address; the returned
/// options can still expose credentials through `Debug` and URL conversion.
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

/// Refuse to build options while an ambient `PGHOSTADDR` would redirect them.
///
/// For native options built from `PgConnectOptions::new_without_pgpass` and
/// explicit setters, which SQLx also starts from the environment: a later
/// `host` setter does not clear the ambient address. Returns the same
/// [`AmbientHostAddress`] refusal as [`connect_options`].
///
/// ```
/// # fn example() -> Result<(), sqlx::Error> {
/// batter_sqlx::refuse_ambient_host_address()?;
/// let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
///     .host("127.0.0.1")
///     .port(5432);
/// # drop(options);
/// # Ok(())
/// # }
/// ```
pub fn refuse_ambient_host_address() -> Result<(), sqlx::Error> {
    refuse_ambient_host_address_with(|name| std::env::var_os(name))
}

pub(crate) fn connect_options_with(
    url: &str,
    environment: impl FnOnce(&str) -> Option<OsString>,
) -> Result<PgConnectOptions, sqlx::Error> {
    refuse_ambient_host_address_with(environment)?;
    url.parse()
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
/// [`refuse_ambient_host_address`] and fixture resolution, before any parse,
/// pool or connection exists. Unset `PGHOSTADDR`; select an address explicitly
/// with a URL `hostaddr` parameter or the native setter instead. Formatting
/// names no address.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(error: &sqlx::Error) -> bool {
        matches!(error, sqlx::Error::Configuration(source) if source.is::<AmbientHostAddress>())
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
            assert!(refused(&error), "refusal is AmbientHostAddress");
            let text = error.to_string();
            assert!(
                !text.contains("192.0.2.1") && !text.contains("secret-marker"),
                "refusal names neither address nor credentials"
            );
        }
        let error = connect_options_with("not a url", |_| Some("192.0.2.1".into())).unwrap_err();
        assert!(refused(&error), "refusal precedes parsing");
    }

    #[test]
    fn absent_host_address_parses_the_url() {
        let options = connect_options_with(URL, |_| None).unwrap();
        assert_eq!(options.get_host(), "127.0.0.1", "URL host is kept");
        assert_eq!(
            options.get_database(),
            Some("database"),
            "URL database is kept"
        );
        assert!(
            matches!(connect_options_with("not a url", |_| None), Err(error) if !refused(&error)),
            "parse failures stay SQLx's own"
        );
    }
}
