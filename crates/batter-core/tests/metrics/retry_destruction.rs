//! Execution completion includes destruction of retained state and callbacks.

use super::{capture::Capture, owner};
use batter_core::{
    retry::{self, ReplaySafety, RetryDecision, RetryOptions, RetryPolicy},
    telemetry::metrics::{RETRY_EXECUTIONS, facade},
};
use std::{
    future::{Future, pending, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    task::Poll,
    time::Duration,
};

#[derive(Debug)]
struct DropProbe(bool);

impl DropProbe {
    fn touch(&self) {
        std::hint::black_box(self);
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        assert!(!self.0, "execution-owned state panicked during destruction");
    }
}

fn policy() -> RetryPolicy {
    let delay = Duration::from_millis(1);
    RetryPolicy::new(2, delay, delay).unwrap()
}

fn assert_execution(capture: &Capture, result: &str) {
    assert_eq!(capture.samples(RETRY_EXECUTIONS).len(), 1);
    assert_eq!(capture.count(RETRY_EXECUTIONS, &[("result", result)]), 1.0);
}

#[tokio::test(start_paused = true)]
async fn retained_error_is_destroyed_before_success_or_final_failure_is_recorded() {
    for succeeds in [true, false] {
        for panics in [false, true] {
            let capture = Capture::unbounded();
            let _recorder = facade::set_default_local_recorder(&capture);
            let task = tokio::spawn(async move {
                retry::execute(
                    owner().context(),
                    "provider.read",
                    ReplaySafety::Idempotent,
                    &policy(),
                    move |attempt| async move {
                        if attempt.number == 1 {
                            Err(DropProbe(panics))
                        } else if succeeds {
                            Ok(7)
                        } else {
                            Err(DropProbe(false))
                        }
                    },
                    |_| RetryDecision::Retry,
                )
                .await
            });
            let outcome = task.await;
            let expected = if panics {
                assert!(outcome.unwrap_err().is_panic());
                "panicked"
            } else {
                assert_eq!(outcome.unwrap().is_ok(), succeeds);
                if succeeds {
                    "succeeded"
                } else {
                    "attempts_exhausted"
                }
            };
            assert_execution(&capture, expected);
        }
    }
}

#[derive(Clone, Copy)]
enum Callback {
    Factory,
    Classifier,
    Jitter,
    Options,
}

async fn callback_destruction(callback: Callback) {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let task = tokio::spawn(async move {
        let factory_capture = DropProbe(matches!(callback, Callback::Factory));
        let classifier_capture = DropProbe(matches!(callback, Callback::Classifier));
        let sampler_capture = DropProbe(matches!(callback, Callback::Jitter | Callback::Options));
        let factory = move |_| {
            factory_capture.touch();
            async { Ok::<_, ()>(7) }
        };
        let classify = move |_: &()| {
            classifier_capture.touch();
            RetryDecision::Retry
        };
        let sample = move || {
            sampler_capture.touch();
            0
        };
        let owner = owner();
        let policy = policy();
        match callback {
            Callback::Factory | Callback::Classifier => {
                retry::execute(
                    owner.context(),
                    "provider.read",
                    ReplaySafety::Idempotent,
                    &policy,
                    factory,
                    classify,
                )
                .await
                .unwrap();
            }
            Callback::Jitter => {
                retry::execute_with_jitter(
                    owner.context(),
                    "provider.read",
                    ReplaySafety::Idempotent,
                    &policy,
                    sample,
                    factory,
                    classify,
                )
                .await
                .unwrap();
            }
            Callback::Options => {
                retry::execute_with_options(
                    owner.context(),
                    "provider.read",
                    ReplaySafety::Idempotent,
                    &policy,
                    RetryOptions::new().with_jitter(sample),
                    factory,
                    classify,
                )
                .await
                .unwrap();
            }
        }
    });
    assert!(task.await.unwrap_err().is_panic());
    assert_execution(&capture, "panicked");
}

#[tokio::test]
async fn factory_capture_is_destroyed_before_completion() {
    callback_destruction(Callback::Factory).await;
}

#[tokio::test]
async fn classifier_capture_is_destroyed_before_completion() {
    callback_destruction(Callback::Classifier).await;
}

#[tokio::test]
async fn jitter_sampler_is_destroyed_before_completion() {
    callback_destruction(Callback::Jitter).await;
}

#[tokio::test]
async fn options_sampler_is_destroyed_before_completion() {
    callback_destruction(Callback::Options).await;
}

#[tokio::test]
async fn pending_retry_destroys_captures_before_recording_abandonment() {
    for panics in [false, true] {
        let capture = Capture::unbounded();
        let _recorder = facade::set_default_local_recorder(&capture);
        let captured = DropProbe(panics);
        let owner = owner();
        let policy = policy();
        let mut retry = Box::pin(retry::execute(
            owner.context(),
            "provider.read",
            ReplaySafety::Idempotent,
            &policy,
            move |_| {
                captured.touch();
                pending::<Result<(), ()>>()
            },
            |_| RetryDecision::Retry,
        ));
        poll_fn(|cx| {
            assert!(retry.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(
            catch_unwind(AssertUnwindSafe(|| drop(retry))).is_err(),
            panics
        );
        assert_execution(&capture, if panics { "panicked" } else { "dropped" });
    }
}

#[tokio::test]
async fn returned_error_destruction_remains_owned_by_the_caller() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let result = retry::execute(
        owner().context(),
        "provider.read",
        ReplaySafety::Idempotent,
        &policy(),
        |_| async { Err::<(), _>(DropProbe(true)) },
        |_| RetryDecision::Stop,
    )
    .await;
    assert!(result.is_err());
    assert_execution(&capture, "not_retryable");
    assert!(catch_unwind(AssertUnwindSafe(|| drop(result))).is_err());
    assert_execution(&capture, "not_retryable");
}
