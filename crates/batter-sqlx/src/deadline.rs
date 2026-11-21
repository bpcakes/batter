use std::{error::Error, fmt, future::Future};
use tokio::time::Instant;

/// A bounded native operation's failure, without inventing database evidence.
/// `F` is the ordinary boundary failure; `E` is an already observed application
/// rejection retained when its recovery cannot acknowledge before the deadline.
/// Every deadline path retires its owner. Retirement proves no remote rollback.
pub enum PgDeadlineError<F, E> {
    /// Deadline before a complete application rejection was observed, or while
    /// validating an otherwise successful provisional result.
    Elapsed,
    /// The native operation returned its ordinary failure before the deadline.
    Failed(F),
    /// Application rejected, then recovery timed out. The original error remains
    /// inspectable; no successful cleanup or reusable owner is implied.
    RecoveryElapsed(E),
}
impl<F, E> fmt::Debug for PgDeadlineError<F, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<F, E> fmt::Display for PgDeadlineError<F, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Elapsed => "PostgreSQL operation deadline elapsed",
            Self::Failed(_) => "PostgreSQL bounded operation failed",
            Self::RecoveryElapsed(_) => "PostgreSQL recovery deadline elapsed after rejection",
        })
    }
}
impl<F: Error + 'static, E: Error + 'static> Error for PgDeadlineError<F, E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Elapsed => None,
            Self::Failed(error) => Some(error),
            Self::RecoveryElapsed(error) => Some(error),
        }
    }
}
pub(crate) struct Elapsed;
pub(crate) async fn until<T>(
    deadline: Instant,
    work: impl Future<Output = T>,
) -> Result<T, Elapsed> {
    let mut work = std::pin::pin!(work);
    let mut timer = std::pin::pin!(tokio::time::sleep_until(deadline));
    std::future::poll_fn(|cx| {
        if timer.as_mut().poll(cx).is_ready() || Instant::now() >= deadline {
            return std::task::Poll::Ready(Err(Elapsed));
        }
        work.as_mut().poll(cx).map(Ok)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn elapsed_clock_prevents_first_poll_and_reentry() {
        use std::{future::poll_fn, task::Poll, time::Duration};
        let deadline = Instant::now();
        assert!(
            until(deadline, async { panic!("polled after deadline") })
                .await
                .is_err(),
            "work must receive no poll at its deadline"
        );
        let deadline = Instant::now() + Duration::from_micros(100);
        let mut polls = 0;
        let mut bounded = std::pin::pin!(until(
            deadline,
            poll_fn(|_| {
                polls += 1;
                if polls == 1 {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
        ));
        poll_fn(|cx| {
            assert!(bounded.as_mut().poll(cx).is_pending(), "initial work waits");
            Poll::Ready(())
        })
        .await;
        tokio::time::advance(Duration::from_micros(100)).await;
        assert!(
            bounded.await.is_err(),
            "an external wake cannot poll work at expiry"
        );
    }

    #[test]
    fn bounded_failures_retain_causes_without_formatting_them() {
        let marker = "private-deadline-cause";
        let failures: [PgDeadlineError<std::io::Error, std::io::Error>; 2] = [
            PgDeadlineError::Failed(std::io::Error::other(marker)),
            PgDeadlineError::RecoveryElapsed(std::io::Error::other(marker)),
        ];
        for failure in failures {
            for text in [
                format!("{failure}"),
                format!("{failure:?}"),
                format!("{failure:#?}"),
            ] {
                assert!(
                    !text.contains(marker),
                    "automatic formatting must redact retained causes"
                );
            }
            assert_eq!(
                failure.source().map(ToString::to_string).as_deref(),
                Some(marker)
            );
        }
    }
}
