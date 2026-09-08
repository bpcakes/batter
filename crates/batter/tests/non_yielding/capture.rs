use super::evidence::Output;
use std::{
    io::{self, Read},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

pub struct Capture {
    output: Arc<Mutex<Output>>,
    reader: Option<JoinHandle<io::Result<()>>>,
}

impl Capture {
    pub fn start(mut pipe: impl Read + Send + 'static) -> io::Result<Self> {
        let output = Arc::new(Mutex::new(Output::default()));
        let captured = output.clone();
        let reader = thread::Builder::new()
            .name("fixture-output".into())
            .spawn(move || {
                let mut buffer = [0; 4096];
                loop {
                    let count = match pipe.read(&mut buffer) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        result => result?,
                    };
                    if count == 0 {
                        return Ok(());
                    }
                    let mut output = captured.lock().unwrap();
                    output.record(&buffer[..count], Instant::now());
                    // Continue draining after the bound. Stopping the reader
                    // could block a fixture in output instead of in its task.
                }
            })?;
        Ok(Self {
            output,
            reader: Some(reader),
        })
    }

    pub fn snapshot(&self) -> Output {
        self.output.lock().unwrap().clone()
    }

    pub fn inspect<T>(&self, inspect: impl FnOnce(&Output, Instant) -> T) -> T {
        // Sample evidence and time under the same lock used to timestamp
        // records. A snapshot followed by a later clock read can go stale at
        // the deadline while the reader publishes an on-time event.
        let output = self.output.lock().unwrap();
        inspect(&output, Instant::now())
    }

    pub fn finish(&mut self) -> io::Result<Output> {
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| io::Error::other("fixture output reader panicked"))??;
        }
        Ok(self.snapshot())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // The child guard kills/reaps before this field is dropped. No writer
        // remains in Command, so EOF also lets us join on failure/unwinding.
        let _ = self.finish();
    }
}

#[path = "capture_tests.rs"]
mod tests;
