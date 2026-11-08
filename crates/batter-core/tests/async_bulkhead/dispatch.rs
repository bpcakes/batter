//! Pending admission owns uncalled captures under the first-poll dispatcher.

use super::{Admission, BulkheadTestExt, bulkhead, owner, poll_pending};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer lock is not poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn dispatch(&self) -> tracing::Dispatch {
        let writer = self.clone();
        crate::test_dispatch::new(
            tracing_subscriber::fmt()
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .without_time()
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_max_level(tracing::Level::INFO)
                .finish(),
        )
    }

    fn text(&self) -> String {
        let bytes = self.0.lock().expect("log buffer lock is not poisoned");
        String::from_utf8(bytes.clone()).expect("tracing output is UTF-8")
    }
}

struct Capture {
    name: &'static str,
    _span: tracing::Span,
}

impl Drop for Capture {
    fn drop(&mut self) {
        tracing::info!(capture = self.name, "uncalled capture destroyed");
    }
}

#[tokio::test]
async fn pending_waiter_destroys_uncalled_captures_and_nested_spans_under_first_dispatcher() {
    let bulkhead = bulkhead();
    let owner = owner();
    let _held = bulkhead
        .hold(owner.context(), Admission::Reject)
        .await
        .unwrap();
    let first_logs = Logs::default();
    let later_logs = Logs::default();
    let first = first_logs.dispatch();
    let later = later_logs.dispatch();
    let called = Arc::new(AtomicUsize::new(0));
    let mut waiter = tracing::dispatcher::with_default(&first, || {
        let factory_parent = tracing::info_span!("factory.parent");
        let factory_capture = Capture {
            name: "factory",
            _span: tracing::info_span!(parent: &factory_parent, "factory.child"),
        };
        let resolver_parent = tracing::info_span!("resolver.parent");
        let resolver_capture = Capture {
            name: "resolver",
            _span: tracing::info_span!(parent: &resolver_parent, "resolver.child"),
        };
        let factory_called = called.clone();
        let resolver_called = called.clone();
        Box::pin(bulkhead.run_resolved(
            owner.context(),
            Admission::Wait,
            "pending.dispatch",
            move |_| {
                factory_called.fetch_add(1, Ordering::SeqCst);
                drop(factory_capture);
                async { Ok::<_, ()>(()) }
            },
            move |result| {
                resolver_called.fetch_add(1, Ordering::SeqCst);
                drop(resolver_capture);
                result
            },
        ))
    });
    tracing::dispatcher::with_default(&first, || poll_pending(waiter.as_mut()));
    // Each child retains its parent; only the uncalled callback owns the child.
    assert!(
        first_logs.text().is_empty(),
        "captures must remain in the waiter"
    );
    tracing::dispatcher::with_default(&later, || drop(waiter));

    assert_eq!(
        called.load(Ordering::SeqCst),
        0,
        "callbacks must stay uncalled"
    );
    let text = first_logs.text();
    for capture in ["factory", "resolver"] {
        assert_eq!(
            text.lines()
                .filter(|line| line.contains(&format!("capture=\"{capture}\"")))
                .count(),
            1,
            "{capture} must be destroyed exactly once under its first dispatcher: {text}"
        );
        for suffix in ["child", "parent"] {
            let own = format!("{capture}.{suffix}: ");
            assert_eq!(
                text.lines()
                    .filter(|line| line.contains(&own) && line.trim_end().ends_with(" close"))
                    .count(),
                1,
                "{capture}.{suffix} must close under its first dispatcher: {text}"
            );
        }
    }
    assert!(later_logs.text().is_empty(), "{}", later_logs.text());
}
