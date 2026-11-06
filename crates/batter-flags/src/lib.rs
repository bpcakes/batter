//! Typed flags and kill switches for Batter applications.
//!
//! Applications declare every flag and kill switch once, in one catalog, and
//! evaluate them only through typed handles: one view per HTTP request, job
//! invocation or periodic run. Two sources feed that one model. Declared mode
//! resolves values once at startup from the application's existing settings
//! source and changes them by restart. The control plane, selected with
//! `postgres`, loads and refreshes an audited `PostgreSQL` snapshot that a
//! registered component keeps fresh.
//!
//! There are no default features. `postgres` re-exports the store as
//! `postgres` and exposes `control_plane`, the protected registration; `axum`
//! exposes `http` for route-group gates on the Batter Axum adapter; `runledger`
//! exposes `jobs` for job-handler gates and pinned decisions. [`core`] is
//! always the catalog crate itself.
//!
//! Experimental. Declared mode, the startup report and the test flags are
//! implemented, as are HTTP gates with `axum`, job gates with `runledger` and
//! [`Flags::gate_periodic_in`] for protected periodic work. Control-plane
//! registration arrives with a later delivery task of the flags epic.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod declared;
mod flags;
mod gate_error;
pub mod low_level;
pub mod periodic;
mod reader;
mod report;
pub mod test;

pub use flags::Flags;
pub use gate_error::GateError;
pub use report::{ReportEntry, StartupReport, ValueSource};

#[cfg(feature = "postgres")]
pub mod control_plane;
#[cfg(feature = "axum")]
pub mod http;
#[cfg(feature = "runledger")]
pub mod jobs;

/// The catalog, snapshot and evaluation crate itself.
///
/// This is `batter-flags-core` under a shorter name, so handles, decisions and
/// views keep one type identity through the direct and facade paths.
pub use batter_flags_core as core;

/// The `PostgreSQL` control-plane store itself, selected with `postgres`.
///
/// It is the `batter-flags-postgres` crate, never a wrapper module. Its store
/// is the low-level form that nothing loads or refreshes; a supervised service
/// reaches the control plane through `control_plane` instead.
#[cfg(feature = "postgres")]
pub use batter_flags_postgres as postgres;
