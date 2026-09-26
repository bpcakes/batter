//! Batter's public facade.
//!
//! The operational implementation lives in [`batter_core`]. This crate keeps
//! the established `batter::...` paths source-compatible and exposes optional
//! adapter namespaces without changing native ownership.
//!
//! The default feature set is empty. Enable only the namespaces an application
//! uses: `at-rest`, `axum`, `metrics`, `otlp`, `sqlx`, `runledger`, `runlimit`,
//! `test-support`, or the narrower bridge features `runlimit-memory`, `runlimit-postgres`,
//! `runlimit-axum`, and `sqlx-test-support`.
//!
//! # Important limits
//!
//! Deadlines drop futures; they do not undo external effects. Cancellation is
//! cooperative, not preemption. Supervision owns directly registered tasks,
//! not tasks secretly spawned by a component. Owned startup, command and service
//! drivers retain registered cleanup independently of borrowed waiters. Direct
//! `CleanupStack::close` driving remains caller-owned. See `docs/guarantees.md`.
//!
//! An owned service root awaits checked completion after startup handoff. The
//! success witness retains the report; `?` propagates the original failed
//! report or coordinator error through the application's error boundary.
//!
//! ```no_run
//! #![deny(unused_must_use)]
//! use batter::{BoxError, lifecycle::RunningSupervisor};
//!
//! async fn finish(running: &RunningSupervisor) -> Result<(), BoxError> {
//!     running.wait_checked().await?;
//!     Ok(())
//! }
//! ```
//!
//! ```
//! use batter::operation::OperationContext;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let context = batter::operation::OperationOwner::new(Duration::from_secs(2))?.into_context();
//! let value = context.run("example.read", |_scope| async {
//!     Ok::<_, std::io::Error>(42)
//! }).await?;
//! assert_eq!(value, 42);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub use batter_core::*;

/// Synchronous envelope encryption and stable MAC keys.
///
/// Enabled by the `at-rest` feature. The implementation remains the standalone
/// [`batter_at_rest`] package, and every item has the same type identity through
/// the direct and facade paths.
///
/// ```
/// let _: Option<batter::at_rest::Keyring> = None;
/// ```
#[cfg(feature = "at-rest")]
pub mod at_rest {
    pub use batter_at_rest::*;
}

/// Axum request, browser, readiness, and serving boundaries.
///
/// Enabled by the `axum` feature. Native Axum and Tower types remain native;
/// this module re-exports Batter's adapter surface only.
///
/// ```
/// let _: Option<batter::axum::RequestPolicy> = None;
/// ```
#[cfg(feature = "axum")]
pub mod axum {
    pub use batter_axum::*;
}

/// Native SQLx PostgreSQL transaction, snapshot, verification, and connection
/// disposition APIs.
///
/// Enabled by the `sqlx` feature. The nested `test_support` module additionally
/// requires `sqlx-test-support` and remains owned by the SQLx adapter.
///
/// ```
/// let _: Option<batter::sqlx::PgLease> = None;
/// ```
#[cfg(feature = "sqlx")]
pub mod sqlx {
    pub use batter_sqlx::*;

    /// External-harness fixture ownership for SQLx tests.
    #[cfg(feature = "sqlx-test-support")]
    pub mod test_support {
        pub use batter_sqlx::test_support::*;
    }
}

/// Runledger initialization and native settlement translation.
///
/// Enabled by the `runledger` feature, which also exposes [`crate::sqlx`] for
/// owned PostgreSQL scopes and [`crate::runledger::run_atomic`]. Durable policy and native
/// supervision remain owned by Runledger.
///
/// ```
/// let _: Option<batter::runledger::NativeReport> = None;
/// let _: Option<batter::sqlx::PgSession<'_>> = None;
/// ```
#[cfg(feature = "runledger")]
pub mod runledger {
    pub use batter_runledger::*;
}

/// Runlimit quota admission and its selected HTTP/error bridges.
///
/// Enabled by `runlimit` or one of its bridge features. The `http` module is
/// present only with `runlimit-axum`; memory and PostgreSQL bridge features do
/// not select a storage backend in the facade.
///
/// ```
/// let _: Option<batter::runlimit::EmptyChecks> = None;
/// ```
#[cfg(feature = "runlimit")]
pub mod runlimit {
    pub use batter_runlimit::*;
}

/// Generic scripted outcomes and body/cleanup result combination for tests.
///
/// Enabled by the `test-support` feature. This does not select SQLx or the
/// external PostgreSQL harness.
///
/// ```
/// let _: Option<batter::test_support::Script<(), ()>> = None;
/// ```
#[cfg(feature = "test-support")]
pub mod test_support {
    pub use batter_test_support::*;
}

/// Optional bounded OTLP metrics adapter, driven by protected service completion.
#[cfg(feature = "otlp")]
pub use batter_otlp as otlp;

#[cfg(test)]
mod tests {
    use super::{BoxError, ConfigurationError, RegistrationError, lifecycle, operation};

    #[test]
    fn legacy_root_types_are_available_through_the_facade() {
        let _: fn(ConfigurationError) -> BoxError = BoxError::from;
        let _: fn(RegistrationError) -> BoxError = BoxError::from;
        let _: Option<operation::Interruption> = None;
        let _: Option<lifecycle::Readiness> = None;
    }
}
