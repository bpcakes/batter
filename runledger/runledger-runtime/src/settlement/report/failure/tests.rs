use std::{error::Error, num::NonZeroUsize, sync::Arc};

use crate::RuntimeShutdownFailure;
use crate::config::JobsConfigValidationError;
use crate::shutdown_signal::RuntimeShutdownSignalError;
use crate::test_logs::{PANIC_SENTINEL, SentinelPayload, panicked_join_error};

/// What a chain-walking reporter such as `anyhow` or `eyre` would print.
fn rendered_chain(error: &(dyn Error + 'static)) -> Vec<String> {
    let mut rendered = Vec::new();
    let mut next = Some(error);
    while let Some(error) = next {
        rendered.push(format!("{error} / {error:?}"));
        next = error.source();
    }
    rendered
}

fn explicit_join_source(failure: &RuntimeShutdownFailure) -> &tokio::task::JoinError {
    match failure {
        RuntimeShutdownFailure::LoopJoin { source, .. }
        | RuntimeShutdownFailure::DescendantJoin { source, .. } => source,
        other => panic!("expected a join failure, got {other:?}"),
    }
}

#[tokio::test]
async fn join_failures_keep_panic_payloads_out_of_the_source_chain() {
    for payload in SentinelPayload::ALL {
        let source = Arc::new(panicked_join_error(payload).await);
        for failure in [
            RuntimeShutdownFailure::LoopJoin {
                task: "worker",
                source: Arc::clone(&source),
            },
            RuntimeShutdownFailure::DescendantJoin {
                task: "running_observer",
                source: Arc::clone(&source),
            },
        ] {
            let boxed: Box<dyn Error + Send + Sync> = Box::new(failure.clone());
            for rendered in rendered_chain(&failure)
                .into_iter()
                .chain(rendered_chain(boxed.as_ref()))
            {
                assert!(
                    !rendered.contains(PANIC_SENTINEL),
                    "{payload:?} payload reached a reporter: {rendered}"
                );
            }
            assert!(failure.source().is_none());
            assert!(
                explicit_join_source(&failure)
                    .to_string()
                    .contains(PANIC_SENTINEL),
                "explicit variant matching still exposes the retained payload"
            );
        }
    }
}

#[test]
fn non_join_sources_remain_exposed() {
    let signal = RuntimeShutdownFailure::Signal {
        source: RuntimeShutdownSignalError::new(std::io::Error::other("signal setup failed")),
    };
    let signal_source = signal.source().expect("signal failures keep their source");
    assert_eq!(signal_source.to_string(), "shutdown signal failed");
    assert_eq!(
        signal_source.source().map(ToString::to_string).as_deref(),
        Some("signal setup failed")
    );

    let invalid = RuntimeShutdownFailure::LoopInvalidConfig {
        task: "worker",
        source: JobsConfigValidationError::EmptyWorkerId,
    };
    assert_eq!(
        invalid.source().map(ToString::to_string),
        Some(JobsConfigValidationError::EmptyWorkerId.to_string())
    );
}

#[tokio::test]
async fn display_messages_are_unchanged() {
    let join = Arc::new(panicked_join_error(SentinelPayload::Owned).await);
    let unjoined = NonZeroUsize::new(2).expect("nonzero");
    for (failure, expected) in [
        (
            RuntimeShutdownFailure::Signal {
                source: RuntimeShutdownSignalError::new(std::io::Error::other("detail")),
            },
            "jobs runtime shutdown signal failed",
        ),
        (
            RuntimeShutdownFailure::SignalPanicked,
            "jobs runtime shutdown signal panicked",
        ),
        (
            RuntimeShutdownFailure::LoopExitedUnexpectedly { task: "worker" },
            "jobs runtime loop `worker` completed unexpectedly",
        ),
        (
            RuntimeShutdownFailure::LoopInvalidConfig {
                task: "worker",
                source: JobsConfigValidationError::EmptyWorkerId,
            },
            "jobs runtime loop `worker` rejected invalid configuration",
        ),
        (
            RuntimeShutdownFailure::LoopJoin {
                task: "worker",
                source: Arc::clone(&join),
            },
            "failed joining jobs runtime loop `worker`",
        ),
        (
            RuntimeShutdownFailure::DescendantJoin {
                task: "running_observer",
                source: Arc::clone(&join),
            },
            "failed joining jobs runtime descendant `running_observer`",
        ),
        (
            RuntimeShutdownFailure::CallbackInterrupted {
                callback: "reaped_observer",
            },
            "jobs runtime callback `reaped_observer` was interrupted while stopping",
        ),
        (
            RuntimeShutdownFailure::EarlierCallbackInterruptions { count: 3 },
            "jobs runtime observed 3 callback interruption(s) before stopping began",
        ),
        (
            RuntimeShutdownFailure::GracefulTimeout,
            "jobs runtime native work did not settle within the graceful allowance",
        ),
        (
            RuntimeShutdownFailure::AbortTimeout { unjoined },
            "jobs runtime tasks did not settle within the abort allowance (2 task(s) unjoined)",
        ),
        (
            RuntimeShutdownFailure::UnrepresentableDeadline,
            "jobs runtime shutdown budget could not be represented as a deadline",
        ),
        (
            RuntimeShutdownFailure::SettlementInterrupted,
            "jobs runtime settlement owner ended before completing observation",
        ),
    ] {
        assert_eq!(failure.to_string(), expected);
        let keeps_source = matches!(
            failure,
            RuntimeShutdownFailure::Signal { .. }
                | RuntimeShutdownFailure::LoopInvalidConfig { .. }
        );
        assert_eq!(failure.source().is_some(), keeps_source, "{failure:?}");
    }
}
