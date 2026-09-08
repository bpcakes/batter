use std::{
    future::Future,
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::instrument::WithSubscriber;

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
            dispatch: tracing::Dispatch::new(subscriber),
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
fn aborted_http_request_destroys_nested_spans_without_cross_registry_panic() {
    use axum::{Router, body::Body, http::Request, middleware, routing::get};
    use batter::lifecycle::ShutdownHandle;
    use batter_axum::{RequestPolicy, request_scope};
    use tower::ServiceExt;

    let scoped = Capture::new();
    let ambient = Capture::new();
    ambient.block_on(async {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = Arc::new(Mutex::new(Some(started_tx)));
        let handle = ShutdownHandle::new();
        handle.mark_ready();
        let router = Router::new()
            .route(
                "/pending",
                get(move || {
                    let started = started.clone();
                    async move {
                        let _resource = DropTrace("http.resource");
                        started.lock().unwrap().take().unwrap().send(()).unwrap();
                        std::future::pending::<&'static str>().await
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(
                RequestPolicy::new(handle, Duration::from_secs(60)).unwrap(),
                request_scope,
            ));
        let task = tokio::spawn(
            router
                .oneshot(
                    Request::builder()
                        .uri("/pending")
                        .body(Body::empty())
                        .unwrap(),
                )
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
    assert_eq!(
        text.matches("HTTP response boundary finished").count(),
        1,
        "{text}"
    );
    assert!(text.contains("batter.http"), "{text}");
    assert!(text.contains("http_outcome=\"dropped\""), "{text}");
    assert!(text.contains("http.resource"), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}
