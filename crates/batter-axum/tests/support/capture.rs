use std::{
    future::Future,
    io::{self, Write},
    sync::{Arc, Mutex, OnceLock},
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
            dispatch: retain_dispatch(tracing::Dispatch::new(subscriber)),
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

// Keep registries alive across cases: tracing caches per-subscriber callsite
// interest. Fixture destruction must not become the subject of adapter tests.
// This is test-only ownership, not a global default subscriber installation.
pub fn retain_dispatch(dispatch: tracing::Dispatch) -> tracing::Dispatch {
    static DISPATCHES: OnceLock<Mutex<Vec<tracing::Dispatch>>> = OnceLock::new();
    DISPATCHES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .push(dispatch.clone());
    dispatch
}
