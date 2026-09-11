use std::{error::Error, fmt, ops::Deref, sync::Arc};

use super::FixtureError;

/// A retained native acquisition producer's failure, even if its waiter was lost.
pub enum AcquisitionFailure {
    /// The native cause is shared with the body when it also observed the failure.
    Native(Arc<postgres_test_harness::Error>),
    /// The producer task panicked or was cancelled; no native error is fabricated.
    Task(tokio::task::JoinError),
}

impl fmt::Display for AcquisitionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Native(_) => "fixture acquisition failed",
            Self::Task(_) => "fixture acquisition task failed",
        })
    }
}
impl fmt::Debug for AcquisitionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for AcquisitionFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Native(error) => error.as_ref(),
            Self::Task(error) => error,
        })
    }
}

/// Body return and task panic/cancellation remain distinct, inspectable failures.
pub enum BodyFailure<E> {
    /// The application body returned its concrete error.
    Returned(E),
    /// Joining the body failed. The default panic hook may already have printed.
    Task(tokio::task::JoinError),
}

impl<E> fmt::Display for BodyFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Returned(_) => "fixture body returned an error",
            Self::Task(_) => "fixture body task failed",
        })
    }
}
impl<E> fmt::Debug for BodyFailure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<E: Error + 'static> Error for BodyFailure<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Returned(error) => error,
            Self::Task(error) => error,
        })
    }
}

/// One acquired database's awaited cleanup, including partially opened fixtures.
pub struct DatabaseCleanup {
    /// Native disposable database identity for catalog observation.
    pub database_name: String,
    /// Actual cleanup result, after all its registered pools closed.
    pub result: Result<(), FixtureError>,
    /// All failed session observations, even if a later explicit retry succeeded.
    pub observation_failures: Vec<Arc<FixtureError>>,
    /// Native pool acquisition errors retained even when the body handled them.
    pub pool_failures: Vec<Arc<sqlx::Error>>,
}

/// Complete internal report; formatting hides body and native error contents.
///
/// Inspect `body`, every `acquisitions` and `databases` result (including
/// `pool_failures` and `observation_failures`), and `drain` to retain all
/// causes. `Error::source` exposes only the first cause; it cannot represent a
/// branching error report. No native pool-close error is fabricated: close returns unit.
/// A successful driver join is not a successful fixture: inspect the report or
/// call [`Self::into_result`]. Accidental discard warns; an explicit discard is
/// still possible and forfeits failure observation.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_sqlx::test_support::FixtureRun;
/// async fn discarded(run: FixtureRun<(), ()>) -> Result<(), tokio::task::JoinError> {
///     run.into_report().await?;
///     Ok(())
/// }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_sqlx::test_support::FixtureRun;
/// async fn discarded(run: &mut FixtureRun<(), ()>) {
///     run.wait().await.expect("driver joined");
/// }
/// ```
#[must_use = "a joined driver can report failures; inspect the report or call into_result"]
pub struct FixtureReport<T, E> {
    /// Body result or captured task failure.
    pub body: Result<T, BodyFailure<E>>,
    /// Joined lease/template producers in registration order. Any failed producer
    /// makes the report unsuccessful, even when the body handled its delivered error.
    pub acquisitions: Vec<Result<(), AcquisitionFailure>>,
    /// All acquired databases, including partial acquisitions, in registration order.
    pub databases: Vec<DatabaseCleanup>,
    /// Separate barrier after every cleanup producer finished.
    pub drain: Result<(), FixtureError>,
}

/// Borrowed result of [`super::FixtureRun::wait`], with the same discard warning
/// as the owned report. A bare reference does not carry its referent's must-use
/// diagnostic. This view dereferences to the report for field/cause inspection;
/// the run retains ownership so observation can be repeated.
///
/// ```no_run
/// # async fn example(mut run: batter_sqlx::test_support::FixtureRun<(), ()>) {
/// let report = run.wait().await.expect("driver joined");
/// assert!(report.is_ok(), "{report:?}");
/// # }
/// ```
#[must_use = "a joined driver can report failures; inspect the borrowed report"]
pub struct FixtureReportRef<'a, T, E>(pub(super) &'a FixtureReport<T, E>);

impl<T, E> Deref for FixtureReportRef<'_, T, E> {
    type Target = FixtureReport<T, E>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<T, E> fmt::Debug for FixtureReportRef<'_, T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, f)
    }
}

impl<T, E> FixtureReport<T, E> {
    /// True only when the body and every independent cleanup result succeeded.
    pub fn is_ok(&self) -> bool {
        self.body.is_ok()
            && self.acquisitions.iter().all(Result::is_ok)
            && self.databases.iter().all(|database| {
                database.result.is_ok()
                    && database.observation_failures.is_empty()
                    && database.pool_failures.is_empty()
            })
            && self.drain.is_ok()
    }
    /// Extract a successful body value, otherwise retain the entire report.
    pub fn into_result(self) -> Result<T, Box<Self>> {
        if self.is_ok() {
            match self.body {
                Ok(value) => Ok(value),
                Err(_) => unreachable!(),
            }
        } else {
            Err(Box::new(self))
        }
    }
}
impl<T, E> fmt::Display for FixtureReport<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fixture report: body_failed={}, acquisition_failures={}, database_failures={}, pool_failures={}, observation_failures={}, drain_failed={}",
            self.body.is_err(),
            self.acquisitions
                .iter()
                .filter(|result| result.is_err())
                .count(),
            self.databases
                .iter()
                .filter(|database| database.result.is_err())
                .count(),
            self.databases
                .iter()
                .map(|database| database.pool_failures.len())
                .sum::<usize>(),
            self.databases
                .iter()
                .map(|database| database.observation_failures.len())
                .sum::<usize>(),
            self.drain.is_err()
        )
    }
}
impl<T, E> fmt::Debug for FixtureReport<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<T: 'static, E: Error + 'static> Error for FixtureReport<T, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Err(error) = &self.body {
            return Some(error);
        }
        for result in &self.acquisitions {
            if let Err(error) = result {
                return Some(error);
            }
        }
        for database in &self.databases {
            if let Some(error) = database.pool_failures.first() {
                return Some(error.as_ref());
            }
            if let Some(error) = database.observation_failures.first() {
                return Some(error.as_ref());
            }
            if let Err(error) = &database.result {
                return Some(error);
            }
        }
        self.drain.as_ref().err().map(|error| error as _)
    }
}
