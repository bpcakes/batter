//! The process-wide name table is bounded; this binary owns it exclusively.
#![cfg(feature = "metrics")]

#[path = "metrics/capture.rs"]
mod capture;

use batter_core::{operation::OperationOwner, telemetry::metrics::*};
use capture::Capture;
use std::{collections::BTreeSet, convert::Infallible, time::Duration};

#[tokio::test]
async fn distinct_names_beyond_capacity_coalesce_without_eviction() {
    const NAMES: usize = NAME_CAPACITY * 3;
    let capture = Capture::unbounded();
    let _recorder = metrics::set_default_local_recorder(&capture);
    let context = OperationOwner::new(Duration::from_secs(5))
        .unwrap()
        .into_context();
    // Leaked runtime strings model the worst case: names that fit the
    // vocabulary but are not developer constants.
    let names: Vec<&'static str> = (0..NAMES)
        .map(|index| &*String::leak(format!("generated.n{index}")))
        .collect();
    for _ in 0..2 {
        for name in &names {
            let result = context
                .run(name, |_| async { Ok::<_, Infallible>(()) })
                .await;
            assert!(result.is_ok());
        }
    }
    let operations: BTreeSet<String> = capture
        .samples(OPERATION_COMPLETIONS)
        .into_iter()
        .flat_map(|sample| sample.labels)
        .filter(|(key, _)| key == "operation")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(operations.len(), NAME_CAPACITY + 1);
    assert!(operations.contains(OVERFLOW_NAME));
    // The first admitted names keep their slots on every later observation.
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("operation", names[0])]),
        2.0
    );
    let overflow = (NAMES - NAME_CAPACITY) * 2;
    assert_eq!(
        capture.count(OPERATION_COMPLETIONS, &[("operation", OVERFLOW_NAME)]),
        overflow as f64
    );
    assert_eq!(
        capture.count(
            LABELS_COALESCED,
            &[("domain", "operation"), ("reason", "capacity")]
        ),
        overflow as f64
    );
    assert!(capture.series().len() <= MAX_SERIES);
}
