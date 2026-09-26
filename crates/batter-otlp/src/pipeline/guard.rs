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
use batter_core::telemetry::metrics::{
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

pub(crate) use batter_core::telemetry::metrics::catalog::HISTOGRAMS;
use batter_core::telemetry::metrics::catalog::{MetricKind as Kind, shape};

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
        if shape.kind() != kind {
            Self::reject(&self.unsupported_kinds);
            return false;
        }
        if !shape.accepts(key) {
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
        if shape.kind() != kind || unit != Some(shape.unit()) || text.len() > DESCRIPTION_MAX_BYTES
        {
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
