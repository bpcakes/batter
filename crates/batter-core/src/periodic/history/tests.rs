use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, thiserror::Error)]
#[error("retained application failure")]
struct Failure(u32);

/// Records whether the publication lock was held while it was destroyed.
#[derive(thiserror::Error)]
#[error("displaced application failure")]
struct DropWitness {
    history: Arc<History>,
    observed: Arc<AtomicUsize>,
}

impl fmt::Debug for DropWitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DropWitness")
    }
}

impl Drop for DropWitness {
    fn drop(&mut self) {
        let observed = if self.history.is_publishing() {
            UNDER_LOCK
        } else {
            OUTSIDE_LOCK
        };
        self.observed.store(observed, Ordering::SeqCst);
    }
}

const NOT_DESTROYED: usize = 0;
const OUTSIDE_LOCK: usize = 1;
const UNDER_LOCK: usize = 2;

fn cause(value: u32) -> Cause {
    Arc::new(Failure(value))
}

#[test]
fn an_unpublished_snapshot_is_explicitly_incomplete() {
    let summary = History::new().snapshot();
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert!(!summary.is_complete());
    assert!(!summary.has_failures());
    assert!(!summary.acknowledged);
}

#[test]
fn bounded_samples_retain_the_first_and_last_concrete_causes() {
    let history = History::new();
    for value in 1..=4 {
        history.admitted();
        history.recoverable(u64::from(value), cause(value));
    }
    let summary = history.snapshot();
    assert_eq!(summary.recoverable_failures, 4);
    assert_eq!(summary.unsampled_failures, 2);
    let first = summary.first_failure.expect("first cause is retained");
    let last = summary.last_failure.expect("last cause is retained");
    assert_eq!(first.invocation, 1);
    assert_eq!(last.invocation, 4);
    // Identity survives without an `E: Clone` bound or text conversion.
    assert_eq!(
        first
            .error
            .downcast_ref::<Failure>()
            .expect("concrete cause remains inspectable")
            .0,
        1,
    );
    assert_eq!(last.error.downcast_ref::<Failure>().unwrap().0, 4);
    // One retained failure is never counted twice.
    assert_eq!(summary.recoverable_failures, summary.unsampled_failures + 2);
}

#[test]
fn a_single_failure_occupies_only_the_first_slot() {
    let history = History::new();
    history.recoverable(1, cause(7));
    let summary = history.snapshot();
    assert_eq!(summary.recoverable_failures, 1);
    assert_eq!(summary.unsampled_failures, 0);
    assert_eq!(summary.first_failure.unwrap().invocation, 1);
    assert!(summary.last_failure.is_none());
}

#[test]
fn later_success_does_not_erase_earlier_failures() {
    let history = History::new();
    history.recoverable(1, cause(1));
    for _ in 0..3 {
        history.admitted();
        history.succeeded();
    }
    let summary = history.snapshot();
    assert_eq!(summary.succeeded, 3);
    assert_eq!(summary.recoverable_failures, 1);
    assert_eq!(summary.first_failure.unwrap().invocation, 1);
}

#[test]
fn deadline_and_stop_interruptions_are_counted_separately() {
    let history = History::new();
    history.deadline_exceeded();
    history.deadline_exceeded();
    history.stop_interrupted();
    let summary = history.snapshot();
    assert_eq!(summary.deadline_exceeded, 2);
    assert_eq!(summary.stop_interrupted, 1);
    assert_eq!(summary.recoverable_failures, 0);
    assert!(summary.has_failures());
}

#[test]
fn counters_saturate_and_report_saturation_instead_of_wrapping() {
    let history = History::new();
    {
        let mut state = history.lock();
        state.invocations = u64::MAX;
        state.succeeded = u64::MAX;
    }
    history.admitted();
    history.succeeded();
    let summary = history.snapshot();
    assert_eq!(summary.invocations, u64::MAX);
    assert_eq!(summary.succeeded, u64::MAX);
    assert!(summary.saturated);
    // Saturation of one counter does not stop the others from advancing.
    history.deadline_exceeded();
    assert_eq!(history.snapshot().deadline_exceeded, 1);
}

#[test]
fn unsampled_accounting_saturates_without_losing_the_bounded_samples() {
    let history = History::new();
    history.recoverable(1, cause(1));
    history.recoverable(2, cause(2));
    {
        let mut state = history.lock();
        state.unsampled_failures = u64::MAX;
    }
    history.recoverable(3, cause(3));
    let summary = history.snapshot();
    assert_eq!(summary.unsampled_failures, u64::MAX);
    assert!(summary.saturated);
    assert_eq!(summary.first_failure.unwrap().invocation, 1);
    assert_eq!(summary.last_failure.unwrap().invocation, 3);
}

#[test]
fn a_displaced_cause_is_destroyed_outside_the_publication_lock() {
    let history = History::new();
    let observed = Arc::new(AtomicUsize::new(NOT_DESTROYED));
    history.recoverable(1, cause(1));
    history.recoverable(
        2,
        Arc::new(DropWitness {
            history: history.clone(),
            observed: observed.clone(),
        }),
    );
    assert_eq!(observed.load(Ordering::SeqCst), NOT_DESTROYED);
    // This replacement displaces the witness and must destroy it afterwards.
    history.recoverable(3, cause(3));
    assert_eq!(observed.load(Ordering::SeqCst), OUTSIDE_LOCK);
    assert_eq!(history.snapshot().last_failure.unwrap().invocation, 3);
}

#[test]
fn a_published_completion_marks_the_snapshot_complete() {
    let history = History::new();
    history.acknowledged();
    history.finished(PeriodicCompletion::Stopped);
    let summary = history.snapshot();
    assert!(summary.acknowledged);
    assert!(summary.is_complete());
    assert_eq!(summary.completion, PeriodicCompletion::Stopped);
}

#[test]
fn a_reader_observes_the_same_retained_evidence_as_the_supervisor() {
    let history = History::new();
    let reader = history.reader("storage.pruning");
    let retained = RetainedHistory::new("storage.pruning", history.clone());
    history.admitted();
    history.succeeded();
    history.finished(PeriodicCompletion::Stopped);
    assert_eq!(reader.name(), "storage.pruning");
    let record = retained.into_record(&[]);
    assert_eq!(record.name, "storage.pruning");
    assert_eq!(record.summary.succeeded, reader.snapshot().succeeded);
    assert_eq!(record.summary.completion, PeriodicCompletion::Stopped);
}

#[test]
fn an_unjoined_runner_cannot_keep_its_published_termination_marker() {
    let history = History::new();
    history.admitted();
    history.succeeded();
    history.recoverable(2, cause(2));
    history.acknowledged();
    history.finished(PeriodicCompletion::Stopped);
    // The coordinator never observed this runner's task, so its own claim
    // about having stopped cannot stand in the report.
    let record = RetainedHistory::new("lease.renewal", history.clone())
        .into_record(&["dependency.health", "lease.renewal"]);
    assert_eq!(record.summary.completion, PeriodicCompletion::Pending);
    assert!(!record.summary.is_complete());
    // Earlier evidence survives that reconciliation.
    assert_eq!(record.summary.succeeded, 1);
    assert_eq!(record.summary.recoverable_failures, 1);
    assert_eq!(record.summary.first_failure.unwrap().invocation, 2);
    assert!(record.summary.acknowledged);
    // A joined runner keeps the marker it published.
    let joined = RetainedHistory::new("lease.renewal", history).into_record(&["other.component"]);
    assert_eq!(joined.summary.completion, PeriodicCompletion::Stopped);
}

#[test]
fn retained_terminal_evidence_survives_a_reset_completion_marker() {
    let history = History::new();
    history.admitted();
    history.terminal(1, cause(5));
    history.finished(PeriodicCompletion::Fatal);
    // The coordinator could not join this runner, so its marker is reset, but
    // the terminal cause it already retained must still be reported.
    let record = RetainedHistory::new("storage.pruning", history).into_record(&["storage.pruning"]);
    let summary = record.summary;
    assert_eq!(summary.completion, PeriodicCompletion::Pending);
    assert!(!summary.is_complete());
    assert_eq!(summary.recoverable_failures, 0);
    assert!(summary.has_failures());
    assert_eq!(
        summary
            .terminal_failure
            .unwrap()
            .error
            .downcast_ref::<Failure>()
            .unwrap()
            .0,
        5,
    );
}

#[test]
fn terminal_completions_are_reported_as_failures() {
    for (completion, expected) in [
        (PeriodicCompletion::Fatal, true),
        (PeriodicCompletion::InitializationExpired, true),
        (PeriodicCompletion::AbandonedDuringStartup, false),
        (PeriodicCompletion::Stopped, false),
        (PeriodicCompletion::Pending, false),
    ] {
        let history = History::new();
        // One admitted run that advanced no classified counter at all.
        history.admitted();
        history.finished(completion);
        let summary = history.snapshot();
        assert_eq!(
            summary.has_failures(),
            expected,
            "{completion:?} must report has_failures() == {expected}",
        );
        assert_eq!(summary.recoverable_failures, 0);
    }
}
