#[path = "../../../../test-support/dispatch.rs"]
pub mod test_dispatch;

use std::{
    future::Future,
    io::{self, Write},
    sync::{Arc, Mutex},
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

pub struct Capture {
    output: Buffer,
    pub dispatch: tracing::Dispatch,
}

impl Capture {
    pub fn new() -> Self {
        Self::with_max_level(tracing::Level::INFO)
    }

    pub fn with_max_level(level: tracing::Level) -> Self {
        Self::with_filter(level.as_str())
    }

    pub fn with_filter(filter: &str) -> Self {
        let output = Buffer(Arc::new(Mutex::new(Vec::new())));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .finish();
        Self {
            output,
            dispatch: test_dispatch::new(subscriber),
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8(self.output.0.lock().unwrap().clone()).unwrap()
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
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
