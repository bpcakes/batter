//! A report nobody claims is destroyed under the dispatcher that started it.

use super::{budget, parent};
use crate::{BoxError, command::Command};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capturing() -> (Buffer, tracing::Dispatch) {
    let buffer = Buffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    (buffer, crate::test_dispatch::new(subscriber))
}

const DESTROYED: &str = "unclaimed work value destroyed";

/// A work value that reports its destruction, and where it happened.
struct Announced {
    destroyed: Option<oneshot::Sender<()>>,
}

impl Drop for Announced {
    fn drop(&mut self) {
        tracing::info!("{DESTROYED}");
        if let Some(destroyed) = self.destroyed.take() {
            // The receiver outlives the command; a lost report fails the test.
            let _ = destroyed.send(());
        }
    }
}

/// Whether the coordinator or the dropped owner frees the output depends on
/// scheduling; either way it must reach the dispatcher that started the command.
#[tokio::test]
async fn unclaimed_owner_report_is_destroyed_under_the_starting_dispatcher() {
    let (started_output, starting) = capturing();
    let (fallback_output, fallback) = capturing();
    let _fallback = tracing::dispatcher::set_default(&fallback);
    let (destroyed, destruction) = oneshot::channel();
    let (finalized, finalization) = oneshot::channel();
    let command = Command::new(parent().into_context(), budget(), move |scope| {
        Box::pin(async move {
            scope
                .reserve_cleanup("value.observer")?
                .register(move || async move {
                    finalized
                        .send(())
                        .map_err(|()| "cleanup observer dropped")?;
                    Ok::<(), BoxError>(())
                });
            Ok::<_, BoxError>(Announced {
                destroyed: Some(destroyed),
            })
        })
    });
    let owner = tracing::dispatcher::with_default(&starting, || command.start_without_signals());
    // Cleanup runs only after work returned its value; nobody then claims it.
    finalization.await.expect("cleanup ran");
    drop(owner);
    destruction
        .await
        .expect("the unclaimed value was destroyed");
    let text = started_output.text();
    assert_eq!(text.matches(DESTROYED).count(), 1, "{text}");
    let fallback_text = fallback_output.text();
    assert!(!fallback_text.contains(DESTROYED), "{fallback_text}");
}
