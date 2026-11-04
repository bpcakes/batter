//! Kill-switch gates for registered periodic components.
//!
//! A gated component evaluates its switch through a view captured for each
//! run; the library owns the skip-or-run outcome and its evidence, while the
//! operation itself stays with the application.
//!
//! Scaffold: the gate arrives with the periodic-gating task.
