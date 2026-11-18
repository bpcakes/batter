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
        "the live Runlimit test URL must be a postgres:// URI without '#' that names one host \
         in its authority (or a host=/socket parameter with an empty authority) and declares \
         no hostaddr, port, service or repeated parameter"
    );
    url.parse().expect("valid PostgreSQL test URL")
}

fn canonical(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default();
    if url.contains('#')
        || !matches!(parsed.scheme(), "postgres" | "postgresql")
        || host.contains(',')
    {
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
