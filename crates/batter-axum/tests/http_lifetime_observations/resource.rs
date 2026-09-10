use crate::events::EventLog;
use axum::body::Bytes;
use std::{
    convert::Infallible,
    io::Write,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct Events(Arc<(EventLog, Notify, Mutex<Vec<String>>)>);
impl Events {
    pub fn record(&self, event: &'static str) {
        self.0.0.record(event);
        let _ = writeln!(std::io::stderr().lock(), "lifetime: {event}");
        self.0.1.notify_waiters();
    }
    pub fn count(&self, event: &str) -> usize {
        self.0.0.count(event)
    }
    pub fn snapshot(&self) -> Vec<&'static str> {
        self.0.0.snapshot()
    }
    pub fn diagnostics(&self) -> Vec<String> {
        self.0.2.lock().unwrap().clone()
    }
    pub fn interrupted(&self, detail: String) {
        let diagnostic = format!("interrupted {detail}; events: {:?}", self.snapshot());
        self.0.2.lock().unwrap().push(diagnostic.clone());
        let _ = writeln!(std::io::stderr().lock(), "{diagnostic}");
    }
    pub async fn wait(&self, event: &str) {
        let mut pending = PendingEvent {
            events: self,
            event,
            complete: false,
        };
        let _ = writeln!(std::io::stderr().lock(), "lifetime wait: {event}");
        loop {
            // Subscribe before checking so a concurrent broadcast cannot be lost.
            let changed = self.0.1.notified();
            if self.count(event) > 0 {
                pending.complete = true;
                return;
            }
            changed.await;
        }
    }
    pub fn before(&self, first: &str, second: &str) {
        self.0.0.before(first, second);
    }
}

// The phase owner supplies the deadline; destruction preserves the missing name.
struct PendingEvent<'a> {
    events: &'a Events,
    event: &'a str,
    complete: bool,
}
impl Drop for PendingEvent<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.events.interrupted(format!("event: {}", self.event));
        }
    }
}

pub struct DropEvent(pub Events, pub &'static str);
impl Drop for DropEvent {
    fn drop(&mut self) {
        self.0.record(self.1);
    }
}

// A stored Notify permit releases a handler even if teardown preceded its wait.
// The body has its own release permit, independent of request cancellation.
pub struct StreamBody {
    pub events: Events,
    pub first: bool,
    pub released: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}
impl http_body::Body for StreamBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        if self.first {
            self.first = false;
            return Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                b"first",
            )))));
        }
        match self.released.as_mut().unwrap().as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.events.record("body-complete");
                Poll::Ready(None)
            }
            Poll::Pending => {
                if self.events.count("body-pending") == 0 {
                    self.events.record("body-pending");
                }
                Poll::Pending
            }
        }
    }
}
impl Drop for StreamBody {
    fn drop(&mut self) {
        drop(self.released.take());
        self.events.record("body-drop");
    }
}

pub struct UploadBody {
    pub inner: Option<axum::body::Body>,
    pub events: Events,
}
impl http_body::Body for UploadBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, axum::Error>>> {
        let result = Pin::new(self.inner.as_mut().unwrap()).poll_frame(cx);
        if result.is_pending() && self.events.count("upload-pending") == 0 {
            self.events.record("upload-pending");
        }
        result
    }
}
impl Drop for UploadBody {
    fn drop(&mut self) {
        drop(self.inner.take());
        self.events.record("upload-body-drop");
    }
}

#[test]
fn failed_ordering_preserves_drop_evidence() {
    for missing in [false, true] {
        let events = Events::default();
        events.record("second");
        if !missing {
            events.record("first");
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _resource = DropEvent(events.clone(), "resource-dropped");
                events.before("first", "second");
            }))
            .is_err()
        );
        assert_eq!(events.count("resource-dropped"), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn missing_event_preserves_drop_evidence() {
    let events = Events::default();
    let observed = events.clone();
    let failure = tokio::spawn(async move {
        let _resource = DropEvent(observed.clone(), "resource-dropped");
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            observed.wait("never-emitted"),
        )
        .await
    })
    .await
    .unwrap();
    assert!(failure.is_err());
    assert!(events.diagnostics()[0].contains("event: never-emitted"));
    assert_eq!(events.count("resource-dropped"), 1);
    events.record("after-failure");
    assert_eq!(events.count("after-failure"), 1);
}
