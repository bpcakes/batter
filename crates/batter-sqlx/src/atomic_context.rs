use crate::{PgAtomicError, PgAtomicScope, run_atomic};
use batter_core::operation::{OperationContext, OperationError};
use sqlx::PgPool;
use std::{convert::Infallible, future::Future, sync::Mutex};

/// Execute an owned atomic workflow within an existing operation budget.
///
/// This is the context-aware counterpart of [`run_atomic`]. It retains the
/// complete native outcome in the same poll that acknowledges disposition,
/// before deadline/cancellation resolution and operation telemetry. Confirmed
/// output, acknowledged rejection, and explicit commit uncertainty cannot be
/// downgraded to an interruption by local outcome publication. It adds no retry.
///
/// An interruption without retained evidence remains an interruption; dropping
/// the future during commit does not prove rollback. Dropping this outer future
/// loses its result and retires the connection, as with `run_atomic`. Callbacks
/// must not export provisional outputs or assume detached external effects are
/// rolled back. The child scope uses the parent's deadline without resetting it.
///
/// ```no_run
/// use batter_core::operation::OperationContext;
/// # async fn example(pool: &sqlx::PgPool, context: &OperationContext) {
/// let result = batter_sqlx::run_atomic_in(pool, context, "audit.append", async |scope| {
///     scope.application(async |sql| {
///         sqlx::query_scalar::<_, i64>("INSERT INTO audit_events DEFAULT VALUES RETURNING id")
///             .fetch_one(sql.executor()).await
///     }).await
/// }).await;
/// // Only Ok(id) proves COMMIT acknowledgement; errors retain native disposition.
/// # let _ = result;
/// # }
/// ```
pub async fn run_atomic_in<T, E>(
    pool: &PgPool,
    context: &OperationContext,
    operation: &'static str,
    work: impl AsyncFnOnce(&mut PgAtomicScope) -> Result<T, E>,
) -> Result<T, OperationError<PgAtomicError<T, E>>> {
    retain(context, operation, run_atomic(pool, work)).await
}

async fn retain<T, E>(
    context: &OperationContext,
    operation: &'static str,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, OperationError<E>> {
    let retained = Mutex::new(None);
    let slot = &retained;
    context
        .run_resolved(
            operation,
            move |_| async move {
                let result = work.await;
                *slot.lock().expect("private atomic outcome slot") = Some(result);
                Ok::<(), Infallible>(())
            },
            |boundary| match retained.lock().expect("private atomic outcome slot").take() {
                Some(result) => result.map_err(OperationError::Failed),
                None => match boundary {
                    Err(OperationError::Interrupted(reason)) => {
                        Err(OperationError::Interrupted(reason))
                    }
                    Err(OperationError::Failed(never)) => match never {},
                    Ok(()) => unreachable!("completed atomic future must retain its result"),
                },
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter_core::operation::Interruption;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[tokio::test]
    async fn acknowledged_output_and_native_error_survive_completion_poll_cancellation() {
        for outcome in [Ok(42), Err("commit uncertain")] {
            let context = OperationContext::new(Duration::from_secs(1)).unwrap();
            let result = retain(&context, "test.atomic", async {
                context.cancel();
                outcome
            })
            .await;
            match outcome {
                Ok(value) => assert_eq!(result.unwrap(), value),
                Err(error) => assert!(
                    matches!(result, Err(OperationError::Failed(actual)) if actual == error)
                ),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_before_acknowledgement_remains_interrupted() {
        let context = OperationContext::new(Duration::from_millis(10)).unwrap();
        let observed = AtomicUsize::new(0);
        let result = retain(&context, "test.atomic", async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            observed.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(42)
        })
        .await;
        assert!(matches!(
            result,
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
        ));
        assert_eq!(
            observed.load(Ordering::SeqCst),
            0,
            "an unobserved simultaneous completion must not be fabricated"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn acknowledged_outcome_survives_deadline_crossed_in_the_completion_poll() {
        for outcome in [Ok(42), Err("unconfirmed commit")] {
            let context = OperationContext::new(Duration::from_millis(10)).unwrap();
            let result = retain(
                &context,
                "test.atomic",
                std::future::poll_fn(|cx| {
                    // Advance the paused clock within this poll, without yielding
                    // between the simulated acknowledgement and its return value.
                    // Polling advance applies its clock change before its yield;
                    // the assertion verifies that prerequisite directly.
                    let before = tokio::time::Instant::now();
                    let mut advance =
                        std::pin::pin!(tokio::time::advance(Duration::from_millis(11)));
                    let _ = advance.as_mut().poll(cx);
                    assert_eq!(
                        tokio::time::Instant::now() - before,
                        Duration::from_millis(11)
                    );
                    assert!(matches!(
                        context.check(),
                        Err(Interruption::DeadlineExceeded)
                    ));
                    std::task::Poll::Ready(outcome)
                }),
            )
            .await;
            match outcome {
                Ok(value) => assert_eq!(result.unwrap(), value),
                Err(error) => assert!(
                    matches!(result, Err(OperationError::Failed(actual)) if actual == error)
                ),
            }
        }
    }

    #[tokio::test]
    async fn unpolled_and_cancelled_atomic_work_never_acquires_or_invokes_callback() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let context = OperationContext::new(Duration::from_secs(1)).unwrap();
        let called = AtomicUsize::new(0);
        drop(run_atomic_in(&pool, &context, "test.atomic", async |_| {
            called.fetch_add(1, Ordering::SeqCst);
            Ok::<(), ()>(())
        }));
        context.cancel();
        let result = run_atomic_in(&pool, &context, "test.atomic", async |_| {
            called.fetch_add(1, Ordering::SeqCst);
            Ok::<(), ()>(())
        })
        .await;
        assert!(matches!(
            result,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        ));
        assert_eq!(called.load(Ordering::SeqCst), 0);
        assert_eq!(pool.size(), 0);
    }
}
