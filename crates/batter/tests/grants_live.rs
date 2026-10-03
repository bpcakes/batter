//! Explicit PostgreSQL 18 acceptance for cross-adapter exact-role grant
//! composition.
//!
//! These cases provision disposable non-owner, non-superuser logins from the
//! native grant fragments plus explicitly declared application policy, then run
//! the real native operations and the real forbidden operations under them.
//! Compiled-but-ignored cases are not evidence; run them explicitly:
//!
//! ```text
//! BATTER_SQLX_ADMIN_URL=postgres://... \
//!   cargo test -p batter --features runledger,runlimit-postgres \
//!   --test grants_live -- --ignored --test-threads=1
//! ```

#[path = "grants_live/policy.rs"]
mod policy;
#[path = "grants_live/quota_cases.rs"]
mod quota_cases;
#[path = "grants_live/runledger_cases.rs"]
mod runledger_cases;
#[path = "grants_live/schema.rs"]
mod schema;
#[path = "grants_live/support.rs"]
mod support;
#[path = "grants_live/worker_cases.rs"]
mod worker_cases;
