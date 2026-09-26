//! Catalog guard in front of the OpenTelemetry metrics bridge.
//!
//! The bridge retains one registry entry, one SDK callback and one attribute
//! vector per complete key, and one metadata entry per described name. This
//! guard admits only the foundation catalog with its exact label shapes and
//! bounded vocabularies, and at most [`KEY_CAPACITY`] complete keys for the
//! process lifetime. Every rejection is decided before delegation, so rejected
//! observations allocate nothing in the bridge. Recorder calls never block on
//! I/O, never panic on a poisoned lock and never log.

use crate::diagnostics::GuardRejections;
use batter::telemetry::metrics::{
    self as catalog,
    facade::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit},
};
use std::{
    collections::HashSet,
    sync::{
        PoisonError, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// Longest accepted catalog description in bytes.
pub(crate) const DESCRIPTION_MAX_BYTES: usize = 128;
/// Complete keys admitted for the process lifetime: the foundation's own
/// series bound, so every series Batter can create fits.
pub(crate) const KEY_CAPACITY: usize = catalog::MAX_SERIES;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Counter,
    Histogram,
}

#[derive(Clone, Copy)]
enum Label {
    /// An operation or task name, or one of the foundation placeholders.
    Name(&'static str),
    /// A closed foundation vocabulary.
    Closed(&'static str, &'static [&'static str]),
    /// The decision vocabulary selected by the preceding admission label.
    Decision,
}

struct Shape {
    name: &'static str,
    kind: Kind,
    unit: Unit,
    labels: &'static [Label],
}

const OPERATION: &[Label] = &[
    Label::Name("operation"),
    Label::Closed("outcome", &catalog::OUTCOMES),
];

const CATALOG: [Shape; 10] = [
    Shape {
        name: catalog::OPERATION_COMPLETIONS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: OPERATION,
    },
    Shape {
        name: catalog::OPERATION_DURATION,
        kind: Kind::Histogram,
        unit: Unit::Seconds,
        labels: OPERATION,
    },
    Shape {
        name: catalog::RETRY_ATTEMPTS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: OPERATION,
    },
    Shape {
        name: catalog::RETRY_EXECUTIONS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Name("operation"),
            Label::Closed("result", catalog::RETRY_RESULTS),
        ],
    },
    Shape {
        name: catalog::ADMISSION_DECISIONS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("admission", catalog::ADMISSIONS),
            Label::Decision,
        ],
    },
    Shape {
        name: catalog::TASK_EXITS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("kind", catalog::TASK_KINDS),
            Label::Name("task"),
            Label::Closed("outcome", catalog::TASK_OUTCOMES),
        ],
    },
    Shape {
        name: catalog::CLEANUP_HOOKS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: &[Label::Closed("outcome", catalog::CLEANUP_OUTCOMES)],
    },
    Shape {
        name: catalog::SHUTDOWNS,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("cause", catalog::SHUTDOWN_CAUSES),
            Label::Closed("result", catalog::SHUTDOWN_RESULTS),
        ],
    },
    Shape {
        name: catalog::SHUTDOWN_DURATION,
        kind: Kind::Histogram,
        unit: Unit::Seconds,
        labels: &[Label::Closed("result", catalog::SHUTDOWN_RESULTS)],
    },
    Shape {
        name: catalog::LABELS_COALESCED,
        kind: Kind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("domain", catalog::COALESCE_DOMAINS),
            Label::Closed("reason", catalog::COALESCE_REASONS),
        ],
    },
];

/// Histogram names that receive the fixed bucket boundaries.
pub(crate) const HISTOGRAMS: [&str; 2] = [catalog::OPERATION_DURATION, catalog::SHUTDOWN_DURATION];

fn shape(name: &str) -> Option<&'static Shape> {
    CATALOG.iter().find(|shape| shape.name == name)
}

fn valid_name(value: &str) -> bool {
    value == catalog::INVALID_NAME
        || value == catalog::OVERFLOW_NAME
        || (!value.is_empty()
            && value.len() <= catalog::MAX_NAME_LEN
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
}

fn decisions(admission: &str) -> Option<&'static [&'static str]> {
    match admission {
        "bulkhead" => Some(catalog::BULKHEAD_DECISIONS),
        "process" => Some(catalog::PROCESS_DECISIONS),
        "root" => Some(catalog::ROOT_DECISIONS),
        _ => None,
    }
}

fn valid_labels(shape: &Shape, key: &Key) -> bool {
    let mut labels = key.labels();
    // Decision vocabularies depend on the admission value that precedes them.
    let mut previous = None;
    for expected in shape.labels {
        let Some(label) = labels.next() else {
            return false;
        };
        let value = label.value();
        let valid = match *expected {
            Label::Name(name) => label.key() == name && valid_name(value),
            Label::Closed(name, values) => label.key() == name && values.contains(&value),
            Label::Decision => {
                label.key() == "decision"
                    && previous
                        .and_then(decisions)
                        .is_some_and(|values| values.contains(&value))
            }
        };
        previous = Some(value);
        if !valid {
            return false;
        }
    }
    labels.next().is_none()
}

/// Shared guard evidence retained by the export owner after installation.
#[derive(Default)]
pub(crate) struct GuardState {
    keys: RwLock<HashSet<Key>>,
    closed: AtomicBool,
    unknown_names: AtomicU64,
    invalid_labels: AtomicU64,
    invalid_descriptions: AtomicU64,
    unsupported_kinds: AtomicU64,
    capacity: AtomicU64,
}

impl GuardState {
    pub(crate) fn rejections(&self) -> GuardRejections {
        GuardRejections {
            unknown_names: self.unknown_names.load(Ordering::Relaxed),
            invalid_labels: self.invalid_labels.load(Ordering::Relaxed),
            invalid_descriptions: self.invalid_descriptions.load(Ordering::Relaxed),
            unsupported_kinds: self.unsupported_kinds.load(Ordering::Relaxed),
            capacity: self.capacity.load(Ordering::Relaxed),
        }
    }

    /// Admitted complete keys; never exceeds [`KEY_CAPACITY`].
    #[cfg(test)]
    pub(crate) fn admitted(&self) -> usize {
        self.keys
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Stop delegating after the final snapshot. Later observations cannot be
    /// exported and must not allocate in the bridge.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn reject(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Validate a registration, then admit its key within the fixed capacity.
    fn admit(&self, key: &Key, kind: Kind) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        let Some(shape) = shape(key.name()) else {
            Self::reject(&self.unknown_names);
            return false;
        };
        if shape.kind != kind {
            Self::reject(&self.unsupported_kinds);
            return false;
        }
        if !valid_labels(shape, key) {
            Self::reject(&self.invalid_labels);
            return false;
        }
        if self
            .keys
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key)
        {
            return true;
        }
        let mut keys = self.keys.write().unwrap_or_else(PoisonError::into_inner);
        if keys.contains(key) {
            return true;
        }
        if keys.len() >= KEY_CAPACITY {
            drop(keys);
            Self::reject(&self.capacity);
            return false;
        }
        keys.insert(key.clone());
        true
    }

    fn describe(&self, name: &KeyName, kind: Kind, unit: Option<Unit>, text: &str) -> bool {
        let Some(shape) = shape(name.as_str()) else {
            Self::reject(&self.unknown_names);
            return false;
        };
        if shape.kind != kind || unit != Some(shape.unit) || text.len() > DESCRIPTION_MAX_BYTES {
            Self::reject(&self.invalid_descriptions);
            return false;
        }
        true
    }
}

/// A recorder admitting only the bounded foundation catalog.
pub(crate) struct CatalogRecorder<R> {
    inner: R,
    state: std::sync::Arc<GuardState>,
}

impl<R: Recorder> CatalogRecorder<R> {
    pub(crate) fn new(inner: R, state: std::sync::Arc<GuardState>) -> Self {
        Self { inner, state }
    }

    #[cfg(test)]
    pub(crate) fn inner(&self) -> &R {
        &self.inner
    }
}

impl<R: Recorder> Recorder for CatalogRecorder<R> {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if self.state.describe(&key, Kind::Counter, unit, &description) {
            self.inner.describe_counter(key, unit, description);
        }
    }

    fn describe_gauge(&self, key: KeyName, _: Option<Unit>, _: SharedString) {
        if shape(key.as_str()).is_some() {
            GuardState::reject(&self.state.invalid_descriptions);
        } else {
            GuardState::reject(&self.state.unknown_names);
        }
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if self
            .state
            .describe(&key, Kind::Histogram, unit, &description)
        {
            self.inner.describe_histogram(key, unit, description);
        }
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        if self.state.admit(key, Kind::Counter) {
            self.inner.register_counter(key, metadata)
        } else {
            Counter::noop()
        }
    }

    fn register_gauge(&self, key: &Key, _: &Metadata<'_>) -> Gauge {
        if shape(key.name()).is_some() {
            GuardState::reject(&self.state.unsupported_kinds);
        } else {
            GuardState::reject(&self.state.unknown_names);
        }
        Gauge::noop()
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        if self.state.admit(key, Kind::Histogram) {
            self.inner.register_histogram(key, metadata)
        } else {
            Histogram::noop()
        }
    }
}
