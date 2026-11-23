//! Test-only tracing capture and panic-payload redaction checks.
//!
//! [`capture_logs`] installs a thread-local subscriber, so capturing tests run
//! on a current-thread runtime or call the logging code directly.

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

/// A private payload that must never reach runtime diagnostics.
pub(crate) const PANIC_SENTINEL: &str = "runledger-private-panic-payload-6d1f";

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl LogBuffer {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("log buffer").clone()).expect("UTF-8 tracing")
    }

    /// Asserts that `message` was logged and that no panic payload was.
    pub(crate) fn assert_redacted_event(&self, message: &str) {
        let text = self.text();
        assert!(text.contains(message), "missing {message:?} in {text}");
        assert!(
            !text.contains(PANIC_SENTINEL),
            "panic payload reached diagnostics: {text}"
        );
    }
}

pub(crate) fn capture_logs(log: &LogBuffer) -> tracing::subscriber::DefaultGuard {
    let writer = log.clone();
    tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish(),
    )
}

/// The two payload types that panic formatting can print.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SentinelPayload {
    Owned,
    Static,
}

impl SentinelPayload {
    pub(crate) const ALL: [Self; 2] = [Self::Owned, Self::Static];

    pub(crate) fn raise(self) {
        match self {
            Self::Owned => std::panic::panic_any(PANIC_SENTINEL.to_owned()),
            Self::Static => std::panic::panic_any(PANIC_SENTINEL),
        }
    }
}

/// A real panicked join whose `Display` and `Debug` print the sentinel.
pub(crate) async fn panicked_join_error(payload: SentinelPayload) -> tokio::task::JoinError {
    let error = tokio::spawn(async move { payload.raise() })
        .await
        .expect_err("the task panics");
    assert!(error.is_panic());
    assert!(
        error.to_string().contains(PANIC_SENTINEL) && format!("{error:?}").contains(PANIC_SENTINEL),
        "the fixture must carry a formattable payload"
    );
    error
}
