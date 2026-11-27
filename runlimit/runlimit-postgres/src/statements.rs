//! Every SQL statement the limiters send at run time, and the only way to
//! send one.
//!
//! A [`Statement`]'s text is private to this module and its submodules, so
//! the rest of the crate can execute only the statements defined here, through
//! [`Statement::query`]. Each one calls catalog functions and casts to
//! non-keyword types by their `pg_catalog` names, and keeps to the dialect
//! `crate::catalog_guard` reads; that guard checks every entry of [`ALL`] and
//! that no other module builds SQL. Published migrations are immutable and
//! live with their migrators instead.

use sqlx::{Postgres, postgres::PgArguments, query::Query};

pub(crate) mod attempts;
pub(crate) mod fixed_window;
pub(crate) mod gcra;

/// One operational statement.
#[derive(Clone, Copy)]
pub(crate) struct Statement(&'static str);

impl Statement {
    /// The statement as a query ready for binding.
    pub(crate) fn query<'q>(self) -> Query<'q, Postgres, PgArguments> {
        sqlx::query(self.0)
    }

    /// The statement's text, for the guard and for tests.
    #[cfg(test)]
    pub(crate) const fn sql(self) -> &'static str {
        self.0
    }
}

pub(crate) const SET_LOCAL_TIMEOUTS_SQL: Statement = Statement(
    r"
SELECT
    pg_catalog.set_config('statement_timeout', $1, true),
    pg_catalog.set_config('lock_timeout', $2, true)
",
);

/// Every statement, by its path in this module.
#[cfg(test)]
pub(crate) const ALL: &[(&str, Statement)] = &[
    ("SET_LOCAL_TIMEOUTS_SQL", SET_LOCAL_TIMEOUTS_SQL),
    (
        "fixed_window::BATCH_ADVISORY_LOCK_SQL",
        fixed_window::BATCH_ADVISORY_LOCK_SQL,
    ),
    (
        "fixed_window::BATCH_ROW_LOCK_SQL",
        fixed_window::BATCH_ROW_LOCK_SQL,
    ),
    (
        "fixed_window::BATCH_CAPACITY_LOCK_SQL",
        fixed_window::BATCH_CAPACITY_LOCK_SQL,
    ),
    (
        "fixed_window::BATCH_PREFLIGHT_SQL",
        fixed_window::BATCH_PREFLIGHT_SQL,
    ),
    (
        "fixed_window::BATCH_UPSERT_SQL",
        fixed_window::BATCH_UPSERT_SQL,
    ),
    ("fixed_window::CLEANUP_SQL", fixed_window::CLEANUP_SQL),
    ("gcra::LOCK_SHARDS", gcra::LOCK_SHARDS),
    ("gcra::SAMPLE_TIME", gcra::SAMPLE_TIME),
    ("gcra::READ_COUNTERS", gcra::READ_COUNTERS),
    ("gcra::UPSERT", gcra::UPSERT),
    ("gcra::UPDATE_WATERMARKS", gcra::UPDATE_WATERMARKS),
    ("gcra::CLEANUP_LOCKS", gcra::CLEANUP_LOCKS),
    ("gcra::CLEANUP_ADVANCE_TIME", gcra::CLEANUP_ADVANCE_TIME),
    ("gcra::CLEANUP_DELETE", gcra::CLEANUP_DELETE),
    ("attempts::SHARD_LOCK_SQL", attempts::SHARD_LOCK_SQL),
    ("attempts::CLEANUP_SQL", attempts::CLEANUP_SQL),
    ("attempts::ROW_LOCK_SQL", attempts::ROW_LOCK_SQL),
    ("attempts::SAMPLE_TIME_SQL", attempts::SAMPLE_TIME_SQL),
    ("attempts::OCCUPANCY_SQL", attempts::OCCUPANCY_SQL),
    ("attempts::FREE_SLOT_SQL", attempts::FREE_SLOT_SQL),
    ("attempts::INSERT_SQL", attempts::INSERT_SQL),
    ("attempts::LEASE_SQL", attempts::LEASE_SQL),
    ("attempts::CLAIM_SQL", attempts::CLAIM_SQL),
    ("attempts::COMPLETE_SQL", attempts::COMPLETE_SQL),
];
