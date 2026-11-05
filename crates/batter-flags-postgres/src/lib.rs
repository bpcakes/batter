//! Audited `PostgreSQL` control-plane storage for `batter-flags`.
//!
//! This crate owns the schema, its forward-only migrations and `*_SQL`
//! vendoring constants, the store over one connection with a pool wrapper,
//! audited compare-and-set writes, history, archive and restore, catalog
//! publication, process presence, pruning, the fleet query and its redacted
//! errors. Evaluation stays in `batter-flags-core`; loading, refreshing and
//! readiness stay in `batter-flags`, which re-exports this crate as
//! `batter_flags::postgres`.
//!
//! Scaffold: the manifest is settled here. The schema and store arrive with
//! the control-plane delivery tasks of the flags epic.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
