use super::*;
use std::{
    cell::RefCell,
    io::{self, Write},
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn dispatch(&self) -> tracing::Dispatch {
        let writer = self.clone();
        capture::test_dispatch::new(
            tracing_subscriber::fmt()
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .without_time()
                .with_span_events(FmtSpan::CLOSE)
                .finish(),
        )
    }
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

struct DropTrace {
    name: &'static str,
    _span: tracing::Span,
}
impl Drop for DropTrace {
    fn drop(&mut self) {
        tracing::info!(capture = self.name, "resolver capture dropped");
    }
}

#[test]
fn abandonment_destroys_work_resolver_and_spans_under_first_poll_dispatch() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let first_logs = Logs::default();
    let later_logs = Logs::default();
    let first = first_logs.dispatch();
    let later = later_logs.dispatch();
    let owner = OperationOwner::new(Duration::from_secs(60)).unwrap();
    let escaped = RefCell::new(None);
    let resolver_called = Cell::new(false);
    let captured = tracing::dispatcher::with_default(&first, || DropTrace {
        name: "resolver",
        _span: tracing::info_span!("resolver.capture"),
    });
    let mut future = Box::pin(run_resolved_value(
        owner.context(),
        "adapter.abandoned",
        |scope| async {
            *escaped.borrow_mut() = Some(scope);
            let _work = DropTrace {
                name: "work",
                _span: tracing::info_span!("work.capture"),
            };
            std::future::pending::<Result<(), Infallible>>().await
        },
        |result| {
            resolver_called.set(true);
            drop(captured);
            result.unwrap();
            ResolvedOutcome::Succeeded(())
        },
    ));
    tracing::dispatcher::with_default(&first, || {
        assert!(matches!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    });
    tracing::dispatcher::with_default(&later, || drop(future));
    assert!(!resolver_called.get());
    assert_eq!(
        escaped.borrow().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
    assert!(owner.context().check().is_ok());
    let text = first_logs.text();
    assert_eq!(
        text.matches("resolver capture dropped").count(),
        2,
        "{text}"
    );
    for name in ["resolver.capture", "work.capture"] {
        assert!(
            text.lines()
                .any(|line| line.contains(name) && line.contains("close")),
            "{text}"
        );
    }
    assert_eq!(
        text.matches("operation boundary finished").count(),
        1,
        "{text}"
    );
    assert!(text.contains("outcome=\"dropped\""), "{text}");
    assert!(later_logs.text().is_empty(), "{}", later_logs.text());
}
