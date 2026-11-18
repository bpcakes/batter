//! Live test URLs select their server, never an ambient `PGHOSTADDR`.
//!
//! `SQLx` 0.9.1, like libpq, connects to an ambient `PGHOSTADDR` instead of a
//! URL's host when the URL has no `hostaddr` parameter. These tests migrate and
//! mutate the disposable database the operator selected, so they refuse to
//! parse a test URL while the variable is present, whatever its value.

use sqlx::postgres::PgConnectOptions;

/// Native options for a live test URL; panics while `PGHOSTADDR` is set.
pub(crate) fn test_connect_options(url: &str) -> PgConnectOptions {
    assert!(
        std::env::var_os("PGHOSTADDR").is_none(),
        "unset PGHOSTADDR for live Runlimit PostgreSQL tests: SQLx would connect to it instead \
         of the test URL's host"
    );
    url.parse().expect("valid PostgreSQL test URL")
}
