//! The protected registration of the audited `PostgreSQL` control plane.
//!
//! Registration owns the loader that publishes versioned snapshots before
//! readiness, the refresh schedule, process presence, pruning, the last-good
//! file and the readiness condition, so an application cannot serve a stale or
//! absent snapshot by composing the native store itself.
//!
//! Scaffold: `register_in` arrives with the control-plane loading task.
