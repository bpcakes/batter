//! Live test URLs select their server by host, never by `hostaddr`.
//!
//! `SQLx` 0.9.1 connects to an ambient `PGHOSTADDR` instead of a URL's host and
//! checks a Unix socket before a URL's `hostaddr`. These tests migrate and
//! mutate the disposable database the operator selected, so they refuse an
//! ambient `PGHOSTADDR` and a URL that declares `hostaddr`, whatever the value.

use sqlx::postgres::PgConnectOptions;

/// Native options for a live test URL; panics on an ambient or declared
/// `hostaddr`.
pub(crate) fn test_connect_options(url: &str) -> PgConnectOptions {
    assert!(
        std::env::var_os("PGHOSTADDR").is_none(),
        "unset PGHOSTADDR for live Runlimit PostgreSQL tests: SQLx would connect to it instead \
         of the test URL's host"
    );
    let declares_host_address =
        url::Url::parse(url).is_ok_and(|url| url.query_pairs().any(|(key, _)| key == "hostaddr"));
    assert!(
        !declares_host_address,
        "remove hostaddr from the live Runlimit test URL and select the server with its host"
    );
    url.parse().expect("valid PostgreSQL test URL")
}
