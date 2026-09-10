#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;

use batter::{operation::OperationContext, telemetry::with_current_dispatch};
use std::{
    cell::Cell,
    future::Future,
    io::{self, Write},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::{Instrument, instrument::WithSubscriber};

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Capture {
    output: Buffer,
    dispatch: tracing::Dispatch,
}

impl Capture {
    fn new() -> Self {
        let output = Buffer(Arc::new(Mutex::new(Vec::new())));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish();
        Self {
            output,
            dispatch: crate::test_dispatch::new(subscriber),
        }
    }

    fn text(&self) -> String {
        String::from_utf8(self.output.0.lock().unwrap().clone()).unwrap()
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        // Keep a different, ordinary INFO registry active during task destruction.
        // Unlike a global installation, this is isolated from other test threads.
        tracing::dispatcher::with_default(&self.dispatch, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(future)
        })
    }
}

struct DropTrace(&'static str);

impl Drop for DropTrace {
    fn drop(&mut self) {
        tracing::info!(resource = self.0, "captured resource dropped");
    }
}

#[test]
fn public_dispatch_wrapper_captures_before_poll_and_preserves_aborted_span_destruction() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let future = tracing::dispatcher::with_default(&scoped.dispatch, || {
        let parent = tracing::info_span!("adapter.parent");
        with_current_dispatch(
            async move {
                let _resource = DropTrace("adapter.resource");
                tracing::info!("adapter future polled");
                started_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            }
            .instrument(tracing::info_span!(parent: &parent, "adapter.child")),
        )
    });
    // The first poll and task destruction both happen under another subscriber.
    // Spawning also proves the public opaque future retains Send when possible.
    ambient.block_on(async {
        let task = tokio::spawn(future);
        started_rx.await.unwrap();
        task.abort();
        let error = task.await.unwrap_err();
        assert!(error.is_cancelled(), "{error}");
        assert!(!error.is_panic(), "{error}");
    });
    let text = scoped.text();
    let polled = text
        .lines()
        .find(|line| line.contains("adapter future polled"))
        .unwrap_or_else(|| panic!("first poll lost the captured subscriber: {text}"));
    assert!(polled.contains("adapter.parent"), "{polled}");
    assert!(polled.contains("adapter.child"), "{polled}");
    assert_eq!(
        text.matches("captured resource dropped").count(),
        1,
        "{text}"
    );
    assert!(text.contains("adapter.resource"), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}

#[test]
fn public_dispatch_wrapper_destroys_unpolled_captures_under_originating_subscriber() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    let future = tracing::dispatcher::with_default(&scoped.dispatch, || {
        let resource = DropTrace("adapter.unpolled");
        let parent = tracing::info_span!("unpolled.parent");
        with_current_dispatch(
            async move {
                let _resource = resource;
                panic!("this future must never be polled");
            }
            .instrument(tracing::info_span!(parent: &parent, "unpolled.child")),
        )
    });
    tracing::dispatcher::with_default(&ambient.dispatch, || drop(future));
    let text = scoped.text();
    assert_eq!(
        text.matches("captured resource dropped").count(),
        1,
        "{text}"
    );
    assert!(text.contains("adapter.unpolled"), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}

#[tokio::test]
async fn public_dispatch_wrapper_accepts_borrowed_non_send_work() {
    let mut value = String::from("borrowed");
    let count = Rc::new(Cell::new(0));
    let result = with_current_dispatch(async {
        tokio::task::yield_now().await;
        count.set(count.get() + 1);
        value.push_str(" value");
        value.as_str()
    })
    .await;
    assert_eq!(result, "borrowed value");
    assert_eq!(count.get(), 1);
}

#[test]
fn aborted_operation_drops_work_and_observation_in_its_scoped_dispatch() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    ambient.block_on(async {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(
            async move {
                OperationContext::new(Duration::from_secs(60))
                    .unwrap()
                    .run("operation.abort", |_| async move {
                        let _resource = DropTrace("operation.resource");
                        started_tx.send(()).unwrap();
                        std::future::pending::<Result<(), io::Error>>().await
                    })
                    .await
            }
            .with_subscriber(scoped.dispatch.clone()),
        );
        started_rx.await.unwrap();
        task.abort();
        let error = task.await.unwrap_err();
        assert!(error.is_cancelled(), "{error}");
        assert!(!error.is_panic(), "{error}");
    });
    let text = scoped.text();
    assert_eq!(
        text.matches("operation boundary finished").count(),
        1,
        "{text}"
    );
    assert!(text.contains("operation.abort"), "{text}");
    assert!(text.contains("outcome=\"dropped\""), "{text}");
    assert!(text.contains("operation.resource"), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}

#[tokio::test]
async fn operation_still_accepts_borrowed_non_send_work() {
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let mut value = String::from("borrowed");
    let count = Rc::new(Cell::new(0));
    let result = context
        .run("operation.borrowed", |_| async {
            tokio::task::yield_now().await;
            count.set(count.get() + 1);
            value.push_str(" value");
            Ok::<_, io::Error>(value.as_str())
        })
        .await
        .unwrap();
    assert_eq!(result, "borrowed value");
    assert_eq!(count.get(), 1);
}
