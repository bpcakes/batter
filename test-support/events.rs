//! Private synchronous evidence storage. Never expose a guard to assertions.
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct EventLog(Arc<Mutex<Vec<&'static str>>>);

impl EventLog {
    pub fn record(&self, event: &'static str) {
        self.0.lock().unwrap().push(event);
    }

    pub fn snapshot(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }

    pub fn count(&self, event: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|item| **item == event)
            .count()
    }

    pub fn before(&self, first: &str, second: &str) {
        let events = self.snapshot();
        let a = events
            .iter()
            .position(|event| *event == first)
            .expect(first);
        let b = events
            .iter()
            .position(|event| *event == second)
            .expect(second);
        assert!(a < b, "{first} must precede {second}: {events:?}");
    }
}
