//! Retain access to the accepted recorder without requiring it to be `Send`.

use metrics::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit};
use std::sync::{Arc, OnceLock};

pub(super) fn install<R: Recorder + Sync + 'static>(
    recorder: R,
) -> Result<&'static R, metrics::SetRecorderError<R>> {
    let recorder = Box::new(recorder);
    let slot = Arc::new(OnceLock::new());
    if metrics::set_global_recorder(Installed(slot.clone())).is_err() {
        return Err(metrics::SetRecorderError(*recorder));
    }
    // Upstream also retains an accepted global recorder for the process lifetime.
    // Allocate before publication, and fill the slot before calling recorder code.
    let recorder: &'static R = Box::leak(recorder);
    let _ = slot.set(recorder);
    Ok(recorder)
}

// A shared reference needs only R: Sync; Arc<R> would also require R: Send.
// A concurrent caller in the brief publication-to-fill interval waits for setup.
struct Installed<R: 'static>(Arc<OnceLock<&'static R>>);

impl<R: Recorder> Recorder for Installed<R> {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.0.wait().describe_counter(key, unit, description);
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.0.wait().describe_gauge(key, unit, description);
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.0.wait().describe_histogram(key, unit, description);
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        self.0.wait().register_counter(key, metadata)
    }

    fn register_gauge(&self, key: &Key, metadata: &Metadata<'_>) -> Gauge {
        self.0.wait().register_gauge(key, metadata)
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        self.0.wait().register_histogram(key, metadata)
    }
}
