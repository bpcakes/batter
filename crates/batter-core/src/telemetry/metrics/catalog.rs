//! Shared metric definitions used by description publication and exporters.
//!
//! ```
//! use batter_core::telemetry::metrics::{catalog, OPERATION_COMPLETIONS};
//! let metric = catalog::shape(OPERATION_COMPLETIONS).unwrap();
//! assert_eq!(metric.name(), OPERATION_COMPLETIONS);
//! ```
use super::facade::{Key, Unit};
use crate::telemetry::metrics as catalog;

#[derive(Clone, Copy, PartialEq, Eq)]
/// Instrument kinds emitted by Batter.
pub enum MetricKind {
    /// Monotonic counter.
    Counter,
    /// Distribution of nonnegative seconds.
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

/// Library-owned metric schema; construction is private.
pub struct MetricShape {
    name: &'static str,
    description: &'static str,
    kind: MetricKind,
    unit: Unit,
    labels: &'static [Label],
}

const OPERATION: &[Label] = &[
    Label::Name("operation"),
    Label::Closed("outcome", &catalog::OUTCOMES),
];

/// Every foundation metric, in description publication order.
pub const CATALOG: [MetricShape; 10] = [
    MetricShape {
        name: catalog::OPERATION_COMPLETIONS,
        description: "Completed Batter operation boundaries by outcome",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: OPERATION,
    },
    MetricShape {
        name: catalog::OPERATION_DURATION,
        description: "Elapsed Batter operation boundary time",
        kind: MetricKind::Histogram,
        unit: Unit::Seconds,
        labels: OPERATION,
    },
    MetricShape {
        name: catalog::RETRY_ATTEMPTS,
        description: "Finished Batter retry attempts by outcome",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: OPERATION,
    },
    MetricShape {
        name: catalog::RETRY_EXECUTIONS,
        description: "Terminal Batter retry execution results",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Name("operation"),
            Label::Closed("result", catalog::RETRY_RESULTS),
        ],
    },
    MetricShape {
        name: catalog::ADMISSION_DECISIONS,
        description: "Batter admission decisions",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("admission", catalog::ADMISSIONS),
            Label::Decision,
        ],
    },
    MetricShape {
        name: catalog::TASK_EXITS,
        description: "Observed Batter task exits",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("kind", catalog::TASK_KINDS),
            Label::Name("task"),
            Label::Closed("outcome", catalog::TASK_OUTCOMES),
        ],
    },
    MetricShape {
        name: catalog::CLEANUP_HOOKS,
        description: "Batter cleanup hook outcomes",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: &[Label::Closed("outcome", catalog::CLEANUP_OUTCOMES)],
    },
    MetricShape {
        name: catalog::SHUTDOWNS,
        description: "Batter supervisor drive results",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("cause", catalog::SHUTDOWN_CAUSES),
            Label::Closed("result", catalog::SHUTDOWN_RESULTS),
        ],
    },
    MetricShape {
        name: catalog::SHUTDOWN_DURATION,
        description: "Batter stop instant to shutdown-report time",
        kind: MetricKind::Histogram,
        unit: Unit::Seconds,
        labels: &[Label::Closed("result", catalog::SHUTDOWN_RESULTS)],
    },
    MetricShape {
        name: catalog::LABELS_COALESCED,
        description: "Metric names replaced by a bounded placeholder",
        kind: MetricKind::Counter,
        unit: Unit::Count,
        labels: &[
            Label::Closed("domain", catalog::COALESCE_DOMAINS),
            Label::Closed("reason", catalog::COALESCE_REASONS),
        ],
    },
];

/// Histogram names that receive the fixed bucket boundaries.
pub const HISTOGRAMS: [&str; 2] = [catalog::OPERATION_DURATION, catalog::SHUTDOWN_DURATION];

/// Find a foundation metric schema by its exact name.
pub fn shape(name: &str) -> Option<&'static MetricShape> {
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

fn valid_labels(shape: &MetricShape, key: &Key) -> bool {
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

impl MetricShape {
    /// Stable metric name.
    pub fn name(&self) -> &'static str {
        self.name
    }
    /// Metric instrument kind.
    pub fn kind(&self) -> MetricKind {
        self.kind
    }
    /// Metric unit.
    pub fn unit(&self) -> Unit {
        self.unit
    }
    /// Static catalog description.
    pub fn description(&self) -> &'static str {
        self.description
    }
    /// Check the complete key, including name and ordered label vocabulary.
    /// This validates shape, not provenance or the global series-capacity bound.
    pub fn accepts(&self, key: &Key) -> bool {
        key.name() == self.name && valid_labels(self, key)
    }
}
