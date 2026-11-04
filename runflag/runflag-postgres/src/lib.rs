//! Audited `PostgreSQL` control-plane storage for runflag.
//!
//! This package owns the schema, its forward-only migrations and `*_SQL`
//! vendoring constants, the store over one connection with a pool wrapper,
//! audited compare-and-set writes, history, archive and restore, catalog
//! publication, process presence, pruning, the fleet query and native redacted
//! errors. Evaluation stays in `runflag-core`; loading, refreshing and
//! readiness stay with the adapter that registers the loader.
//!
//! Scaffold: the manifest, lints and dependency shape are settled here. The
//! schema and store arrive with the control-plane delivery tasks of the flags
//! epic.
