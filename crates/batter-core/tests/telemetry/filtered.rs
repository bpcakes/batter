use super::{Buffer, OperationContext};
use std::{
    future::{Future, poll_fn},
    io,
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};

struct Capture {
    output: Arc<Mutex<Vec<u8>>>,
    dispatch: tracing::Dispatch,
}

impl Capture {
    fn new() -> Self {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = Buffer(output.clone());
        let dispatch = crate::test_dispatch::new(
            tracing_subscriber::fmt()
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .without_time()
                .with_env_filter("info,batter=warn")
                .finish(),
        );
        Self { output, dispatch }
    }

    fn text(&self) -> String {
        String::from_utf8(self.output.lock().unwrap().clone()).unwrap()
    }
}

#[tokio::test(start_paused = true)]
async fn filtered_operation_retains_first_parent_for_failure_deadline_and_drop() {
    let owner = Capture::new();
    let other = Capture::new();
    for outcome in ["failed", "deadline_exceeded", "dropped"] {
        let parent = tracing::dispatcher::with_default(&owner.dispatch, || {
            tracing::info_span!(
                "application.request",
                request_id = outcome,
                outcome = "application-owned",
                elapsed_ms = 777
            )
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let context = OperationContext::new(Duration::from_secs(1)).unwrap();
        let mut run = Box::pin(context.run("filtered.operation", |_| async move {
            let _ = rx.await;
            tracing::info!("application resumed");
            Err::<(), _>(io::Error::other("secret-cause"))
        }));
        assert!(
            poll_fn(
                |cx| Poll::Ready(tracing::dispatcher::with_default(&owner.dispatch, || {
                    parent.in_scope(|| run.as_mut().poll(cx))
                }))
            )
            .await
            .is_pending()
        );
        if outcome == "failed" {
            tx.send(()).unwrap();
        }
        if outcome == "deadline_exceeded" {
            tokio::time::advance(Duration::from_secs(2)).await;
        }
        let stranger = tracing::dispatcher::with_default(&other.dispatch, || {
            tracing::info_span!("unrelated.request", request_id = "stranger")
        });
        if outcome != "dropped" {
            assert!(
                poll_fn(|cx| Poll::Ready(tracing::dispatcher::with_default(
                    &other.dispatch,
                    || { stranger.in_scope(|| run.as_mut().poll(cx)) }
                )))
                .await
                .is_ready()
            );
        }
        tracing::dispatcher::with_default(&other.dispatch, || stranger.in_scope(|| drop(run)));
        // Observation must record only its own span, never these inherited fields.
        tracing::dispatcher::with_default(&owner.dispatch, || {
            parent.in_scope(|| tracing::info!("parent unchanged"))
        });
    }
    let text = owner.text();
    let completions: Vec<_> = text
        .lines()
        .filter(|line| line.contains("operation boundary finished"))
        .collect();
    assert_eq!(completions.len(), 3, "{text}");
    for (line, outcome) in completions
        .iter()
        .zip(["failed", "deadline_exceeded", "dropped"])
    {
        assert!(
            line.contains(&format!("request_id=\"{outcome}\"")),
            "{line}"
        );
        assert!(
            line.split_once("operation boundary finished")
                .unwrap()
                .1
                .contains(&format!("outcome=\"{outcome}\"")),
            "{line}"
        );
        assert!(line.contains("WARN"), "{line}");
    }
    for line in text
        .lines()
        .filter(|line| line.contains("parent unchanged"))
    {
        assert!(
            line.contains("outcome=\"application-owned\" elapsed_ms=777"),
            "{line}"
        );
    }
    let resumed = text
        .lines()
        .find(|line| line.contains("application resumed"))
        .unwrap();
    assert!(resumed.contains("request_id=\"failed\""), "{resumed}");
    assert!(
        !text.contains("stranger")
            && !text.contains("secret-cause")
            && !text.contains("batter.operation"),
        "{text}"
    );
    assert!(other.text().is_empty(), "{}", other.text());
}

#[tokio::test]
async fn filtered_operation_without_initial_parent_does_not_adopt_drop_context() {
    let capture = Capture::new();
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let mut run = Box::pin(context.run("unparented", |_| {
        std::future::pending::<Result<(), io::Error>>()
    }));
    assert!(
        poll_fn(
            |cx| Poll::Ready(tracing::dispatcher::with_default(&capture.dispatch, || run
                .as_mut()
                .poll(cx)))
        )
        .await
        .is_pending()
    );
    tracing::dispatcher::with_default(&capture.dispatch, || {
        tracing::info_span!("late.request", request_id = "late").in_scope(|| drop(run));
    });
    let text = capture.text();
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("operation boundary finished"))
            .count(),
        1,
        "{text}"
    );
    assert!(text.contains("dropped") && !text.contains("late"), "{text}");
}

#[test]
fn unused_capture_releases_subscriber_storage() {
    let capture = Capture::new();
    let output = Arc::downgrade(&capture.output);
    drop(capture);
    // Concurrent interest-cache rebuilds can briefly borrow a registered
    // dispatch. Require release after those borrowers finish, not instant drop.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while output.strong_count() != 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        output.upgrade().is_none(),
        "capture subscriber leaked its output buffer"
    );
}
