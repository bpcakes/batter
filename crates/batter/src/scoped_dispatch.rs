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
use tracing::Dispatch;

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

#[cfg(test)]
mod tests {
    use super::scope;
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
        let dispatch = tracing::Dispatch::new(subscriber);
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
        let fallback = tracing::Dispatch::new(tracing_subscriber::registry());
        tracing::dispatcher::with_default(&fallback, || drop(future));
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            text.matches("unpolled capture dropped").count(),
            1,
            "{text}"
        );
        assert!(!text.contains("operation boundary finished"), "{text}");
    }
}
