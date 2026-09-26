//! A thread-local test recorder that retains every sample it accepts.
//!
//! `limit` models a saturated or unavailable collector: samples beyond it are
//! discarded by the recorder and counted, never reported back to Batter.
#![allow(dead_code)]

use metrics::{
    Counter, CounterFn, Gauge, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

#[derive(Default)]
struct Store {
    samples: Vec<Sample>,
    discarded: usize,
    registrations: usize,
}

#[derive(Clone)]
pub struct Capture {
    limit: usize,
    store: Arc<Mutex<Store>>,
}

impl Capture {
    pub fn unbounded() -> Self {
        Self::saturating_after(usize::MAX)
    }

    pub fn saturating_after(limit: usize) -> Self {
        Self {
            limit,
            store: Arc::default(),
        }
    }

    pub fn samples(&self, name: &str) -> Vec<Sample> {
        let store = self.store.lock().unwrap();
        store
            .samples
            .iter()
            .filter(|s| s.name == name)
            .cloned()
            .collect()
    }

    /// Sum of counter increments whose labels include every `(key, value)`.
    pub fn count(&self, name: &str, labels: &[(&str, &str)]) -> f64 {
        self.samples(name)
            .into_iter()
            .filter(|sample| {
                labels
                    .iter()
                    .all(|(k, v)| sample.labels.iter().any(|(sk, sv)| sk == k && sv == v))
            })
            .map(|sample| sample.value)
            .sum()
    }

    /// Position of the first accepted sample of `name` with every label, in
    /// recording order across all metrics.
    pub fn first(&self, name: &str, labels: &[(&str, &str)]) -> Option<usize> {
        let store = self.store.lock().unwrap();
        store.samples.iter().position(|sample| {
            sample.name == name
                && labels
                    .iter()
                    .all(|(k, v)| sample.labels.iter().any(|(sk, sv)| sk == k && sv == v))
        })
    }

    /// Distinct `(name, labels)` series the recorder was asked to register.
    pub fn series(&self) -> BTreeSet<(String, Vec<(String, String)>)> {
        let store = self.store.lock().unwrap();
        store
            .samples
            .iter()
            .map(|s| (s.name.clone(), s.labels.clone()))
            .collect()
    }

    pub fn discarded(&self) -> usize {
        self.store.lock().unwrap().discarded
    }

    /// Every registration, including those whose sample was discarded.
    pub fn registrations(&self) -> usize {
        self.store.lock().unwrap().registrations
    }

    fn handle(&self, key: &Key) -> Arc<Handle> {
        self.store.lock().unwrap().registrations += 1;
        let mut labels: Vec<_> = key
            .labels()
            .map(|label| (label.key().to_owned(), label.value().to_owned()))
            .collect();
        labels.sort();
        Arc::new(Handle {
            name: key.name().to_owned(),
            labels,
            capture: self.clone(),
        })
    }
}

struct Handle {
    name: String,
    labels: Vec<(String, String)>,
    capture: Capture,
}

impl Handle {
    fn push(&self, value: f64) {
        let mut store = self.capture.store.lock().unwrap();
        if store.samples.len() >= self.capture.limit {
            store.discarded += 1;
            return;
        }
        store.samples.push(Sample {
            name: self.name.clone(),
            labels: self.labels.clone(),
            value,
        });
    }
}

impl CounterFn for Handle {
    fn increment(&self, value: u64) {
        self.push(value as f64);
    }

    fn absolute(&self, value: u64) {
        self.push(value as f64);
    }
}

impl HistogramFn for Handle {
    fn record(&self, value: f64) {
        self.push(value);
    }
}

impl Recorder for Capture {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

    fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
        Counter::from_arc(self.handle(key))
    }

    fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
        Gauge::noop()
    }

    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(self.handle(key))
    }
}
