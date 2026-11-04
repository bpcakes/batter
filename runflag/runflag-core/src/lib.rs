//! Framework-neutral typed flags and kill switches.
//!
//! This package owns the flag key and catalog grammar, declaration kinds,
//! validated snapshots and their views, evaluation that reads no clock,
//! pinned decisions and the disclosed `low_level` publication path. It
//! deliberately contains no async runtime, transport framework or persistence
//! integration: the `PostgreSQL` store lives in `runflag-postgres`, and
//! lifecycle ownership stays with the adapter that registers a loader.
//!
//! The `serde` feature derives serialization for the manifest and snapshot
//! types that the store and the operator command exchange; without it the
//! normal dependency closure holds no serialization library.
//!
//! Scaffold: the manifest, lints and feature shape are settled here. The items
//! arrive with the catalog and snapshot delivery tasks of the flags epic.
