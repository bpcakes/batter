//! Live test URLs select their server through a canonical authority.
//!
//! `SQLx` 0.9.1 connects to an ambient `PGHOSTADDR` instead of a URL's host,
//! checks a Unix socket before a URL's `hostaddr`, and reads a query `host` or
//! `port` as a replacement where tokio-postgres and libpq read another
//! candidate. These tests migrate and mutate the disposable database the
//! operator selected, so they refuse an ambient `PGHOSTADDR` and any URL
//! outside the subset every PostgreSQL client reads alike.

use std::collections::BTreeSet;

use sqlx::postgres::PgConnectOptions;

/// Query parameters `SQLx`, tokio-postgres and libpq all honour alike.
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

/// Native options for a live test URL; panics on an ambient `PGHOSTADDR` or a
/// URL that does not select its server in a canonical authority.
pub(crate) fn test_connect_options(url: &str) -> PgConnectOptions {
    assert!(
        std::env::var_os("PGHOSTADDR").is_none(),
        "unset PGHOSTADDR for live Runlimit PostgreSQL tests: SQLx would connect to it instead \
         of the test URL's host"
    );
    assert!(
        canonical(url),
        "the live Runlimit test URL must be a postgres:// URI that names one host, an explicit \
         port and a database, percent-encodes query '+' and stray '@', and declares only sslmode, \
         ssl file, application_name, options, user, password or dbname parameters once"
    );
    url.parse().expect("valid PostgreSQL test URL")
}

/// The URL subset `SQLx`, tokio-postgres and libpq read alike: a `postgres://`
/// URI without `#`, a raw query `+` or a raw `@` outside the userinfo; exactly one
/// host and an explicit port in the authority; a database in the path or
/// `dbname`; and only allowlisted parameters, each once.
fn canonical(url: &str) -> bool {
    if url.contains('#') {
        return false;
    }
    let Some(rest) = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
    else {
        return false;
    };
    let (authority, tail) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
    let hosts = authority.rsplit('@').next().unwrap_or_default();
    // Only the query form-decodes `+` (as a space, in SQLx alone).
    if tail
        .split_once('?')
        .is_some_and(|(_, query)| query.contains('+'))
        || tail.contains('@')
        || authority.matches('@').count() > 1
        || hosts.contains(',')
        || hosts.to_ascii_lowercase().contains("%2c")
    {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.host_str().unwrap_or_default().is_empty() || parsed.port().is_none() {
        return false;
    }
    let mut seen = BTreeSet::new();
    for (key, _) in parsed.query_pairs() {
        if !PARAMETERS.contains(&&*key) || !seen.insert(key.into_owned()) {
            return false;
        }
    }
    !parsed.path().trim_start_matches('/').is_empty() || seen.contains("dbname")
}
