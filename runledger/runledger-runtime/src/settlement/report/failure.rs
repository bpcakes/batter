use std::{fmt, num::NonZeroUsize, sync::Arc};

use tokio::task::JoinError;

use crate::RuntimeShutdownSignalError;
use crate::config::JobsConfigValidationError;

/// The primary retained reason a [`RuntimeShutdownReport`] is not a success.
///
/// This is classification for logs, alerts and process exit codes. It is not an
/// authorization: [`RuntimeShutdownReport::classify`] can allow cleanup
/// while this is present, and only that typed decision decides whether dependency
/// cleanup may run. The originating report retains every other
/// observed reason; this names one.
///
/// Automatic Debug and Display formatting retain the classification and safe
/// metadata but never format join sources. A [`JoinError`] formats its panic
/// payload, so [`Error::source`](std::error::Error::source) does not expose it
/// either: it returns `None` for [`Self::LoopJoin`] and [`Self::DescendantJoin`],
/// and reporters that walk the source chain stop there. Those payloads remain
/// available only through explicit variant matching on their `source` field.
/// [`Self::Signal`] and [`Self::LoopInvalidConfig`] still expose their sources.
///
/// [`RuntimeShutdownReport`]: crate::RuntimeShutdownReport
/// [`RuntimeShutdownReport::classify`]: crate::RuntimeShutdownReport::classify
#[derive(Clone)]
pub enum RuntimeShutdownFailure {
    /// The external stop input returned an error retained by the report.
    Signal { source: RuntimeShutdownSignalError },
    /// The external stop input panicked while it was polled or destroyed.
    /// Detailed, redacted-by-default evidence remains on the report.
    SignalPanicked,
    /// A supervised loop returned `Completed` instead of the expected shutdown exit.
    LoopExitedUnexpectedly { task: &'static str },
    /// A supervised loop rejected its configuration after build validation.
    LoopInvalidConfig {
        task: &'static str,
        source: JobsConfigValidationError,
    },
    /// A supervised loop panicked or failed to join. `source` is reachable only
    /// by matching this variant, not through `Error::source`.
    LoopJoin {
        task: &'static str,
        source: Arc<JoinError>,
    },
    /// A native-owned descendant panicked or was cancelled unexpectedly.
    /// `source` is reachable only by matching this variant, not through
    /// `Error::source`.
    DescendantJoin {
        task: &'static str,
        source: Arc<JoinError>,
    },
    /// A handler, observer or hook was interrupted while stopping, so
    /// descendants it may have created cannot be accounted for.
    CallbackInterrupted { callback: &'static str },
    /// Callbacks were interrupted before stopping began. This count never
    /// expires and permanently disqualifies cooperative cleanup.
    EarlierCallbackInterruptions { count: u64 },
    /// Native work had not settled when the graceful allowance elapsed.
    GracefulTimeout,
    /// Tasks did not settle within the total allowance after aborting.
    AbortTimeout { unjoined: NonZeroUsize },
    /// The budget could not be represented as a deadline, so no settlement time
    /// was allowed. [`RuntimeShutdownReport::deadline_error`] retains the
    /// rejected allowance. Validation at construction cannot prove that the
    /// same duration fits a stop instant reached much later.
    ///
    /// [`RuntimeShutdownReport::deadline_error`]: crate::RuntimeShutdownReport::deadline_error
    UnrepresentableDeadline,
    /// The settlement owner ended before establishing a final join boundary.
    /// Retained evidence is partial and never authorizes dependency cleanup.
    SettlementInterrupted,
}

impl fmt::Display for RuntimeShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signal { .. } => f.write_str("jobs runtime shutdown signal failed"),
            Self::SignalPanicked => f.write_str("jobs runtime shutdown signal panicked"),
            Self::LoopExitedUnexpectedly { task } => {
                write!(f, "jobs runtime loop `{task}` completed unexpectedly")
            }
            Self::LoopInvalidConfig { task, .. } => {
                write!(
                    f,
                    "jobs runtime loop `{task}` rejected invalid configuration"
                )
            }
            Self::LoopJoin { task, .. } => write!(f, "failed joining jobs runtime loop `{task}`"),
            Self::DescendantJoin { task, .. } => {
                write!(f, "failed joining jobs runtime descendant `{task}`")
            }
            Self::CallbackInterrupted { callback } => write!(
                f,
                "jobs runtime callback `{callback}` was interrupted while stopping"
            ),
            Self::EarlierCallbackInterruptions { count } => write!(
                f,
                "jobs runtime observed {count} callback interruption(s) before stopping began"
            ),
            Self::GracefulTimeout => {
                f.write_str("jobs runtime native work did not settle within the graceful allowance")
            }
            Self::AbortTimeout { unjoined } => write!(
                f,
                "jobs runtime tasks did not settle within the abort allowance ({unjoined} task(s) unjoined)"
            ),
            Self::UnrepresentableDeadline => {
                f.write_str("jobs runtime shutdown budget could not be represented as a deadline")
            }
            Self::SettlementInterrupted => {
                f.write_str("jobs runtime settlement owner ended before completing observation")
            }
        }
    }
}

impl std::error::Error for RuntimeShutdownFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Signal { source } => Some(source),
            Self::LoopInvalidConfig { source, .. } => Some(source),
            // A join error's Display and Debug print its panic payload.
            Self::LoopJoin { .. } | Self::DescendantJoin { .. } => None,
            Self::SignalPanicked
            | Self::LoopExitedUnexpectedly { .. }
            | Self::CallbackInterrupted { .. }
            | Self::EarlierCallbackInterruptions { .. }
            | Self::GracefulTimeout
            | Self::AbortTimeout { .. }
            | Self::UnrepresentableDeadline
            | Self::SettlementInterrupted => None,
        }
    }
}

impl fmt::Debug for RuntimeShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signal { source } => f.debug_struct("Signal").field("source", source).finish(),
            Self::SignalPanicked => f.write_str("SignalPanicked"),
            Self::LoopExitedUnexpectedly { task } => f
                .debug_struct("LoopExitedUnexpectedly")
                .field("task", task)
                .finish(),
            Self::LoopInvalidConfig { task, source } => f
                .debug_struct("LoopInvalidConfig")
                .field("task", task)
                .field("source", source)
                .finish(),
            Self::LoopJoin { task, source } => f
                .debug_struct("LoopJoin")
                .field("task", task)
                .field("cancelled", &source.is_cancelled())
                .field("panicked", &source.is_panic())
                .finish(),
            Self::DescendantJoin { task, source } => f
                .debug_struct("DescendantJoin")
                .field("task", task)
                .field("cancelled", &source.is_cancelled())
                .field("panicked", &source.is_panic())
                .finish(),
            Self::CallbackInterrupted { callback } => f
                .debug_struct("CallbackInterrupted")
                .field("callback", callback)
                .finish(),
            Self::EarlierCallbackInterruptions { count } => f
                .debug_struct("EarlierCallbackInterruptions")
                .field("count", count)
                .finish(),
            Self::GracefulTimeout => f.write_str("GracefulTimeout"),
            Self::AbortTimeout { unjoined } => f
                .debug_struct("AbortTimeout")
                .field("unjoined", unjoined)
                .finish(),
            Self::UnrepresentableDeadline => f.write_str("UnrepresentableDeadline"),
            Self::SettlementInterrupted => f.write_str("SettlementInterrupted"),
        }
    }
}

#[cfg(test)]
mod tests;
