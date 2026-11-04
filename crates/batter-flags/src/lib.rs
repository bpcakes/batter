//! Typed flags and kill switches for Batter applications.
//!
//! Applications declare every flag and kill switch once, in one native
//! catalog, and evaluate them only through typed handles: one view per HTTP
//! request, job invocation or periodic run. Two sources feed that one model.
//! Declared mode resolves values once at startup from the application's
//! existing settings source and changes them by restart. The control plane,
//! selected with `postgres`, loads and refreshes an audited `PostgreSQL`
//! snapshot that a registered component keeps fresh.
//!
//! There are no default features. `postgres` re-exports the native store as
//! `postgres` and exposes `control_plane`, the protected registration; `axum`
//! exposes `http` for route-group gates on the Batter Axum adapter; `runledger`
//! exposes `jobs` for job-handler gates and pinned decisions. [`native`] is
//! always the native catalog package itself.
//!
//! Experimental: this crate is the scaffold of the flags delivery. Its module
//! and feature shape is settled; the items arrive with the declared-mode and
//! control-plane tasks.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod declared;
pub mod low_level;
pub mod periodic;
pub mod test;

#[cfg(feature = "postgres")]
pub mod control_plane;
#[cfg(feature = "axum")]
pub mod http;
#[cfg(feature = "runledger")]
pub mod jobs;

/// The native catalog, snapshot and evaluation package itself.
///
/// This is `runflag-core` under its own name, so handles, decisions and views
/// keep one type identity through the direct and facade paths.
pub use runflag_core as native;

/// The native `PostgreSQL` control-plane store itself, selected with `postgres`.
///
/// It is the native package, never a wrapper module. Its store is the
/// low-level form that nothing loads or refreshes; a supervised service reaches
/// the control plane through `control_plane` instead.
#[cfg(feature = "postgres")]
pub use runflag_postgres as postgres;
