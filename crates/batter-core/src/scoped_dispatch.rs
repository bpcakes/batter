//! Keep polling and destruction under the same tracing subscriber.
//!
//! `tracing::instrument::WithDispatch` restores the subscriber only while
//! polling. A task can be destroyed elsewhere, so it does not protect either
//! Drop events or nested span destruction. Wrap the entire owning future,
//! outside its `Instrumented` wrapper, rather than only a borrowed inner body.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tracing::{Dispatch, Span};

pin_project_lite::pin_project! {
    pub(crate) struct ScopedDispatch<F> {
        #[pin]
        inner: Option<F>,
        dispatch: Dispatch,
    }

    impl<F> PinnedDrop for ScopedDispatch<F> {
        fn drop(this: Pin<&mut Self>) {
            let mut this = this.project();
            tracing::dispatcher::with_default(this.dispatch, || {
                // Destroy in place while pinned, under the captured dispatcher.
                // The empty Option prevents a second drop after the guard exits.
                this.inner.set(None);
            });
        }
    }
}

impl<F: Future> Future for ScopedDispatch<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        tracing::dispatcher::with_default(this.dispatch, || {
            this.inner
                .as_pin_mut()
                .expect("future is present until drop")
                .poll(cx)
        })
    }
}

/// Async entry points call this inside their body to capture on first poll.
pub(crate) fn scope<F>(future: F) -> ScopedDispatch<F> {
    with_dispatch(future, tracing::dispatcher::get_default(Clone::clone))
}

/// Queued work passes the dispatcher captured when admission was requested.
pub(crate) fn with_dispatch<F>(future: F, dispatch: Dispatch) -> ScopedDispatch<F> {
    ScopedDispatch {
        inner: Some(future),
        dispatch,
    }
}

/// A task output that keeps its producer's dispatcher and span for destruction.
///
/// Tokio drops an unclaimed `JoinHandle` output in its task harness or wherever
/// the handle is dropped, outside any [`ScopedDispatch`]. Return this from inside
/// the scoped future instead: an output nobody claims is still destroyed under
/// the dispatcher and span that were current when it was produced.
pub(crate) struct ScopedOutput<R> {
    value: Option<R>,
    dispatch: Dispatch,
    span: Span,
}

impl<R> ScopedOutput<R> {
    /// Capture the current dispatcher and span; call inside the scoped future.
    pub(crate) fn capture(value: R) -> Self {
        Self {
            value: Some(value),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            span: Span::current(),
        }
    }

    /// Claim the output; the claimant's own context then owns its destruction.
    /// The producer's span is still released under the producer's dispatcher.
    pub(crate) fn into_inner(mut self) -> R {
        self.value
            .take()
            .expect("output is present until claimed or dropped")
    }
}

impl<R> Drop for ScopedOutput<R> {
    fn drop(&mut self) {
        let value = self.value.take();
        // The span can hold the last reference to a nested span, whose parent
        // the registry closes through the current default dispatcher. Release
        // it under the producer's dispatcher too, whether or not it was claimed.
        let span = std::mem::replace(&mut self.span, Span::none());
        tracing::dispatcher::with_default(&self.dispatch, || {
            if let Some(value) = value {
                let _entered = span.enter();
                drop(value);
            }
            drop(span);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{ScopedOutput, scope};
    use std::{
        future::Future,
        io::{self, Write},
        marker::PhantomPinned,
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
    };

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

    struct NeverPolled {
        _pin: PhantomPinned,
        _child: tracing::Span,
    }

    impl Future for NeverPolled {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            panic!("this future must not be polled")
        }
    }

    impl Drop for NeverPolled {
        fn drop(&mut self) {
            tracing::info!("unpolled capture dropped");
        }
    }

    #[test]
    fn never_polled_pinned_capture_and_its_nested_span_drop_under_saved_dispatch() {
        let output = Buffer(Arc::new(Mutex::new(Vec::new())));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish();
        let dispatch = crate::test_dispatch::new(subscriber);
        let future = tracing::dispatcher::with_default(&dispatch, || {
            let parent = tracing::info_span!("unpolled.parent");
            let child = tracing::info_span!(parent: &parent, "unpolled.child");
            scope(NeverPolled {
                _pin: PhantomPinned,
                _child: child,
            })
        });
        // The inner future is !Unpin and has never been polled. Dropping it
        // also releases its parent span, which belongs to the original registry.
        let fallback = crate::test_dispatch::new(tracing_subscriber::registry());
        tracing::dispatcher::with_default(&fallback, || drop(future));
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            text.matches("unpolled capture dropped").count(),
            1,
            "{text}"
        );
        assert!(!text.contains("operation boundary finished"), "{text}");
    }

    fn capturing() -> (Buffer, tracing::Dispatch) {
        let output = Buffer(Arc::new(Mutex::new(Vec::new())));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish();
        (output, crate::test_dispatch::new(subscriber))
    }

    fn text(output: &Buffer) -> String {
        String::from_utf8(output.0.lock().unwrap().clone()).unwrap()
    }

    const OUTPUT_DROPPED: &str = "scoped output dropped";

    struct Announced;

    impl Drop for Announced {
        fn drop(&mut self) {
            tracing::info!("{OUTPUT_DROPPED}");
        }
    }

    fn produced_under(dispatch: &tracing::Dispatch) -> ScopedOutput<Announced> {
        tracing::dispatcher::with_default(dispatch, || {
            let producer = tracing::info_span!("output.producer");
            let _entered = producer.enter();
            ScopedOutput::capture(Announced)
        })
    }

    fn drop_lines(output: &Buffer) -> Vec<String> {
        text(output)
            .lines()
            .filter(|line| line.contains(OUTPUT_DROPPED))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn unclaimed_output_drops_under_its_producer_dispatch_and_span() {
        let (produced, producer) = capturing();
        let (fallen_back, fallback) = capturing();
        let output = produced_under(&producer);
        tracing::dispatcher::with_default(&fallback, || drop(output));
        let lines = drop_lines(&produced);
        assert_eq!(lines.len(), 1, "{}", text(&produced));
        assert!(lines[0].contains("output.producer"), "{}", lines[0]);
        assert!(
            drop_lines(&fallen_back).is_empty(),
            "{}",
            text(&fallen_back)
        );
    }

    #[test]
    fn claimed_output_drops_under_its_claimant_without_reentering_the_producer() {
        let (produced, producer) = capturing();
        let (claimed, claimant) = capturing();
        let output = produced_under(&producer);
        tracing::dispatcher::with_default(&claimant, || {
            let claiming = tracing::info_span!("output.claimant");
            let _entered = claiming.enter();
            drop(output.into_inner());
        });
        assert!(drop_lines(&produced).is_empty(), "{}", text(&produced));
        let lines = drop_lines(&claimed);
        assert_eq!(lines.len(), 1, "{}", text(&claimed));
        assert!(lines[0].contains("output.claimant"), "{}", lines[0]);
        assert!(!lines[0].contains("output.producer"), "{}", lines[0]);
    }

    /// Records span closes in the producer's registry.
    #[derive(Clone, Default)]
    struct Closed(Arc<Mutex<Vec<&'static str>>>);

    impl<S> tracing_subscriber::Layer<S> for Closed
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        fn on_close(&self, id: tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
            if let Some(span) = ctx.span(&id) {
                self.0.lock().unwrap().push(span.name());
            }
        }
    }

    /// The output holds the last reference to a child span with a parent.
    fn nested_output() -> (Closed, ScopedOutput<()>, tracing::Dispatch) {
        use tracing_subscriber::layer::SubscriberExt;
        let closed = Closed::default();
        let producer =
            crate::test_dispatch::new(tracing_subscriber::registry().with(closed.clone()));
        let output = tracing::dispatcher::with_default(&producer, || {
            let parent = tracing::info_span!("output.parent");
            let child = tracing::info_span!(parent: &parent, "output.child");
            let _entered = child.enter();
            ScopedOutput::capture(())
        });
        assert!(closed.0.lock().unwrap().is_empty());
        (closed, output, producer)
    }

    #[test]
    fn unclaimed_output_releases_its_nested_span_under_the_producer() {
        let (closed, output, _producer) = nested_output();
        // A different registry would panic on, or misroute, the parent close.
        let fallback = crate::test_dispatch::new(tracing_subscriber::registry());
        tracing::dispatcher::with_default(&fallback, || drop(output));
        assert_eq!(*closed.0.lock().unwrap(), ["output.child", "output.parent"]);
    }

    #[test]
    fn claimed_output_releases_its_nested_span_under_the_producer() {
        let (closed, output, _producer) = nested_output();
        let fallback = crate::test_dispatch::new(tracing_subscriber::registry());
        tracing::dispatcher::with_default(&fallback, || output.into_inner());
        assert_eq!(*closed.0.lock().unwrap(), ["output.child", "output.parent"]);
    }
}
