//! An operational foundation, not a new effect runtime or web framework.
//!
//! The core uses ordinary futures, typed application errors, and Tokio.
//! The separate `batter-axum` crate provides a thin HTTP boundary.
//! The workspace targets Unix backends. Windows is unsupported and not planned.
//!
//! # Important limits
//!
//! Deadlines drop futures; they do not undo external effects. Cancellation is
//! cooperative, not preemption. Supervision owns directly registered tasks,
//! not tasks secretly spawned by a component. Explicit asynchronous cleanup
//! must be driven to completion by its caller. See `docs/guarantees.md`.
//!
//! ```
//! use batter::operation::OperationContext;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let context = OperationContext::new(Duration::from_secs(2))?;
//! let value = context.run("example.read", |_scope| async {
//!     Ok::<_, std::io::Error>(42)
//! }).await?;
//! assert_eq!(value, 42);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod admission;
pub mod cleanup;
pub mod health;
pub mod lifecycle;
pub mod operation;
pub mod retry;
pub mod startup;
pub mod telemetry;

mod scoped_dispatch;
mod validation;

pub use validation::{ConfigurationError, RegistrationError};

/// Type erasure is confined to process and cleanup boundaries.
/// Domain and operation APIs retain their concrete error types.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

// Private shared test machinery; excluded from the library API and production build.
#[cfg(test)]
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
