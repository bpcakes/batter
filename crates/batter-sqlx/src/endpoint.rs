//! The explicit PostgreSQL endpoint boundary: a canonical URL's authority, or
//! explicit setters, select the server for every PostgreSQL client.

use std::{collections::BTreeSet, ffi::OsString, fmt};

use sqlx::postgres::PgConnectOptions;

const AMBIENT_HOST_ADDRESS: &str = "PGHOSTADDR";

/// Parse a canonical PostgreSQL URL into native options that connect to the
/// server its authority names.
///
/// SQLx 0.9.1, tokio-postgres and libpq read the same URL differently: SQLx
/// reads an ambient `PGHOSTADDR` into every URL parse and checks a Unix socket
/// before a declared `hostaddr`; a query `host` or `port` replaces the
/// authority in SQLx but extends tokio-postgres's host or port list; libpq
/// accepts keyword/value strings, `service` files and query text after `#`.
/// This boundary admits only the subset all three read alike, so a URL that
/// passes selects one server for every client. Inside
/// `sqlx::Error::Configuration` and before parsing, it refuses with
/// [`AmbientHostAddress`] whenever `PGHOSTADDR` is present, whatever its value,
/// and with [`NoncanonicalUrl`] unless the URL:
///
/// - is a `postgres://` or `postgresql://` URI without `#`, a raw `+` in the
///   query, or a raw `@` outside the userinfo delimiter;
/// - names exactly one host and an explicit port in its authority, with a Unix
///   socket directory percent-encoded there (`postgres://u@%2Ftmp:5432/db`);
/// - names its database in the path or a `dbname` parameter;
/// - declares only `sslmode`, `sslrootcert`, `sslcert`, `sslkey`,
///   `application_name`, `options`, `user`, `password` and `dbname`, each once.
///
/// Every endpoint part is then explicit, so ambient `PGHOST`, `PGPORT` and
/// `PGDATABASE` cannot select it either. Any remaining parse failure is SQLx's
/// own error.
///
/// Use it instead of `str::parse`, `PgConnectOptions::from_str` or the
/// URL-taking `PgPool::connect`, `PgPoolOptions::connect`, `connect_lazy`,
/// `PgConnection::connect` and the `MigrateDatabase` maintenance methods, and
/// pass the result to the pool's or connection's `_with` variant. For a
/// listener, build a pool from these options and pass `&pool` to
/// `PgListener::connect_with` instead of calling `PgListener::connect(url)`.
/// Other PG* variables remain native defaults that explicit URL parts and
/// setters override. The process must not change its environment between this
/// check and connection. Formatting a refusal names no URL part; the returned
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
    canonical(url)
        .map_err(|reason| sqlx::Error::Configuration(Box::new(NoncanonicalUrl { reason })))?;
    url.parse()
}

/// Query parameters SQLx, tokio-postgres and libpq all honour with the same
/// meaning; any other key is refused rather than silently ignored by one.
const PARAMETERS: [&str; 9] = [
    "sslmode",
    "sslrootcert",
    "sslcert",
    "sslkey",
    "application_name",
    "options",
    "user",
    "password",
    "dbname",
];

/// Check the URL subset SQLx, tokio-postgres and libpq read alike.
fn canonical(url: &str) -> Result<(), NoncanonicalReason> {
    use NoncanonicalReason::*;
    if url.contains('#') {
        return Err(Fragment);
    }
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or(NotUri)?;
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    // tokio-postgres takes the last `@` anywhere as the userinfo delimiter and
    // SQLx form-decodes a query `+` as a space where the others keep it, so
    // both must be percent-encoded there.
    let query = tail.split_once('?').map_or("", |(_, query)| query);
    if query.contains('+') || tail.contains('@') || authority.matches('@').count() > 1 {
        return Err(Encoding);
    }
    // libpq and tokio-postgres split the decoded host list at commas.
    let hosts = authority.rsplit('@').next().unwrap_or_default();
    if hosts.contains(',') || hosts.to_ascii_lowercase().contains("%2c") {
        return Err(MultipleHosts);
    }
    let parsed = url::Url::parse(url).map_err(|_| NotUri)?;
    if parsed.host_str().unwrap_or_default().is_empty() {
        return Err(MissingHost);
    }
    if parsed.port().is_none() {
        return Err(MissingPort);
    }
    let mut seen = BTreeSet::new();
    for (key, _) in parsed.query_pairs() {
        if !PARAMETERS.contains(&&*key) {
            return Err(UnsupportedParameter);
        }
        if !seen.insert(key.into_owned()) {
            return Err(DuplicateParameter);
        }
    }
    if parsed.path().trim_start_matches('/').is_empty() && !seen.contains("dbname") {
        return Err(MissingDatabase);
    }
    Ok(())
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
/// [`setter_connect_options`], fixture resolution and template preparation,
/// before any parse, pool or connection exists. Unset `PGHOSTADDR` and select
/// the server with the URL host. Formatting names no address.
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

/// A PostgreSQL URL outside the subset every PostgreSQL client reads alike.
///
/// Returned inside `sqlx::Error::Configuration` by [`connect_options`],
/// fixture resolution and template preparation, before parsing. Its
/// [`reason`](Self::reason) names the rule; formatting names no URL part.
///
/// Migration: replaces `DeclaredHostAddress`, which an earlier revision of
/// this unreleased boundary returned for a declared `hostaddr`; that case is
/// now [`NoncanonicalReason::UnsupportedParameter`].
///
/// ```
/// # fn example(error: &sqlx::Error) -> bool {
/// use batter_sqlx::{NoncanonicalReason, NoncanonicalUrl};
/// matches!(error, sqlx::Error::Configuration(source)
///     if source.downcast_ref::<NoncanonicalUrl>().map(NoncanonicalUrl::reason)
///         == Some(NoncanonicalReason::UnsupportedParameter))
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoncanonicalUrl {
    reason: NoncanonicalReason,
}

impl NoncanonicalUrl {
    /// The rule the URL broke.
    #[must_use]
    pub const fn reason(&self) -> NoncanonicalReason {
        self.reason
    }
}

/// Which canonical-URL rule a refused PostgreSQL URL broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoncanonicalReason {
    /// Not a `postgres://` or `postgresql://` URI, such as a libpq
    /// keyword/value string.
    NotUri,
    /// Contains `#`, which libpq reads as query text and the others do not.
    /// Encode a literal `#` as `%23`.
    Fragment,
    /// Contains a raw `+` in the query, or a raw `@` outside the userinfo
    /// delimiter, which the clients decode differently. Encode them as `%2B`
    /// and `%40`.
    Encoding,
    /// The authority names several comma-separated hosts.
    MultipleHosts,
    /// The authority names no host. A Unix socket directory goes in the
    /// authority percent-encoded, such as `postgres://user@%2Ftmp:5432/db`.
    MissingHost,
    /// The authority names no port, so an ambient `PGPORT` would select it
    /// for SQLx and libpq but not for tokio-postgres.
    MissingPort,
    /// Neither the path nor a `dbname` parameter names a database, so an
    /// ambient `PGDATABASE` or the login name would select it.
    MissingDatabase,
    /// Declares a parameter outside `sslmode`, `sslrootcert`, `sslcert`,
    /// `sslkey`, `application_name`, `options`, `user`, `password` and
    /// `dbname`, such as `hostaddr`, `host`, `port`, `service` or `ssl`, which
    /// at least one client ignores or reads differently.
    UnsupportedParameter,
    /// Declares one parameter twice, which the clients resolve differently.
    DuplicateParameter,
}

impl fmt::Display for NoncanonicalUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.reason {
            NoncanonicalReason::NotUri => "the PostgreSQL URL is not a postgres:// URI",
            NoncanonicalReason::Fragment => "the PostgreSQL URL contains '#'; encode it as %23",
            NoncanonicalReason::Encoding => {
                "the PostgreSQL URL contains a raw '+' in its query or a misplaced '@'; percent-encode them"
            }
            NoncanonicalReason::MultipleHosts => "the PostgreSQL URL names several hosts",
            NoncanonicalReason::MissingHost => "the PostgreSQL URL names no host in its authority",
            NoncanonicalReason::MissingPort => "the PostgreSQL URL names no port in its authority",
            NoncanonicalReason::MissingDatabase => "the PostgreSQL URL names no database",
            NoncanonicalReason::UnsupportedParameter => {
                "the PostgreSQL URL declares a parameter not every client reads alike"
            }
            NoncanonicalReason::DuplicateParameter => {
                "the PostgreSQL URL declares a parameter twice"
            }
        })
    }
}

impl std::error::Error for NoncanonicalUrl {}

#[cfg(test)]
mod tests {
    use super::*;

    fn is<T: std::error::Error + 'static>(error: &sqlx::Error) -> bool {
        matches!(error, sqlx::Error::Configuration(source) if source.is::<T>())
    }

    fn reason(url: &str) -> Option<NoncanonicalReason> {
        match connect_options_with(url, |_| None) {
            Ok(_) => None,
            Err(sqlx::Error::Configuration(source)) => {
                Some(source.downcast_ref::<NoncanonicalUrl>().unwrap().reason())
            }
            Err(error) => panic!("unexpected error {error}"),
        }
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
    fn noncanonical_urls_are_refused_with_their_rule() {
        use NoncanonicalReason::*;
        for (url, expected) in [
            ("host=db-a hostaddr=192.0.2.1 dbname=postgres", NotUri),
            ("mysql://user@127.0.0.1:5432/db", NotUri),
            (
                "postgres://u@127.0.0.1:5432/db?application_name=a#b&hostaddr=192.0.2.1",
                Fragment,
            ),
            ("postgres://u@127.0.0.1:5432/db#tag", Fragment),
            (
                "postgres://db-a:5432/jobs?application_name=worker@db-b",
                Encoding,
            ),
            ("postgres://u@db-a:5432/jobs?options=a+b", Encoding),
            ("postgres://u@db-a,db-b:5432/db", MultipleHosts),
            ("postgres://u@db-a:5432,db-b:5433/db", MultipleHosts),
            ("postgres://u@db-a%2Cdb-b:5432/db", MultipleHosts),
            ("postgres:///db?host=/tmp", MissingHost),
            ("postgres://u@db/jobs", MissingPort),
            ("postgres://u@db:5432", MissingDatabase),
            ("postgres://u@db:5432/", MissingDatabase),
            (
                "postgres://u:secret-marker@127.0.0.1:5432/db?hostaddr=192.0.2.1",
                UnsupportedParameter,
            ),
            (
                "postgres://u@localhost:5432/db?host%61ddr=127.0.0.1",
                UnsupportedParameter,
            ),
            (
                "postgres://u@db-a:5432/postgres?host=db-b",
                UnsupportedParameter,
            ),
            (
                "postgres://u@127.0.0.1:5432/postgres?port=5432",
                UnsupportedParameter,
            ),
            (
                "postgres://u@127.0.0.1:5432/db?service=other",
                UnsupportedParameter,
            ),
            (
                "postgres://u@127.0.0.1:5432/db?ssl=true",
                UnsupportedParameter,
            ),
            (
                "postgres://u@127.0.0.1:5432/db?statement-cache-capacity=0",
                UnsupportedParameter,
            ),
            (
                "postgres://u@127.0.0.1:5432/db?sslmode=disable&sslmode=require",
                DuplicateParameter,
            ),
        ] {
            assert_eq!(reason(url), Some(expected), "{url}");
        }
        let error = connect_options_with(
            "postgres://user:secret-marker@127.0.0.1:5432/db?hostaddr=192.0.2.1",
            |_| None,
        )
        .unwrap_err();
        let text = error.to_string();
        assert!(
            !text.contains("192.0.2.1") && !text.contains("secret-marker"),
            "{text}"
        );
    }

    #[test]
    fn canonical_host_and_socket_urls_parse() {
        let options = connect_options_with(URL, |_| None).unwrap();
        assert_eq!(options.get_host(), "127.0.0.1", "URL host is kept");
        assert_eq!(options.get_port(), 5432, "URL port is kept");
        assert_eq!(
            options.get_database(),
            Some("database"),
            "URL database is kept"
        );
        for url in [
            "postgresql://user@localhost:5432/db?sslmode=verify-full&sslrootcert=%2Fetc%2Fca.pem",
            "postgres://user@[::1]:5432/db?application_name=a%23b&options=-c%20x%3Dy",
            "postgres://user%40example:p%40ss+word%2C1@db.invalid:5432/db",
            "postgres://user@%2Ftmp%2Fpg+socket:5432/jobs",
            "postgres://user@%2Ftmp:5432/db",
            "postgres://user@127.0.0.1:5432/?dbname=other&user=u&password=p",
        ] {
            assert_eq!(reason(url), None, "{url}");
        }
        let socket = connect_options_with("postgres://user@%2Ftmp:5432/db", |_| None).unwrap();
        assert!(
            socket.get_socket().is_some(),
            "an encoded authority socket stays a socket"
        );
    }
}
