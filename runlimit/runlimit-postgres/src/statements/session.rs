//! The statement every limiter transaction runs first.

statements! {
    /// Sets the transaction's statement and lock timeouts.
    SET_LOCAL_TIMEOUTS_SQL = r"
SELECT
    pg_catalog.set_config('statement_timeout', $1, true),
    pg_catalog.set_config('lock_timeout', $2, true)
";
}
