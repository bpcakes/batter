use super::*;
use std::time::Duration;

#[test]
fn startup_and_final_validation_require_the_same_complete_exact_record() {
    let at = Instant::now();
    let mut output = Output::default();
    output.record(
        b"diagnostic: drain-requested\nprefix batter-fixture:drain-requested\n\
          batter-fixture:drain-requested-extra\nbatter-fixture:drain-requested",
        at,
    );
    assert!(output.validate(&["drain-requested"], &[]).is_err());
    assert!(
        output
            .event_by("drain-requested", at, at)
            .unwrap_err()
            .contains("did not emit")
    );
    let completed = at + Duration::from_secs(1);
    output.record(b"\r\n", completed);
    output.validate(&["drain-requested"], &[]).unwrap();
    assert_eq!(
        output
            .event_by("drain-requested", completed, completed)
            .unwrap(),
        Some(completed)
    );
    assert!(output.validate(&[], &["drain-requested"]).is_err());
}

#[test]
fn fragmented_record_is_observed_only_when_its_newline_arrives() {
    let at = Instant::now();
    let deadline = at + Duration::from_secs(5);
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-", at);
    output.record(b"requested", deadline);
    assert_eq!(output.observed("drain-requested"), None);
    output.record(b"\n", deadline + Duration::from_nanos(1));
    assert!(
        output
            .event_by("drain-requested", deadline, deadline)
            .unwrap_err()
            .contains("after its deadline")
    );
}

#[test]
fn duplicate_records_preserve_the_first_observation() {
    let at = Instant::now();
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-requested\n", at);
    output.record(
        b"batter-fixture:drain-requested\n",
        at + Duration::from_secs(10),
    );
    assert_eq!(
        output
            .event_by("drain-requested", at, at + Duration::from_secs(20))
            .unwrap(),
        Some(at)
    );
}

#[test]
fn invalid_utf8_and_unterminated_records_are_not_startup_evidence() {
    let at = Instant::now();
    let mut output = Output::default();
    output.record(
        b"batter-fixture:drain-requested\xff\nbatter-fixture:drain-requested",
        at,
    );
    assert!(output.validate(&["drain-requested"], &[]).is_err());
    assert!(output.event_by("drain-requested", at, at).is_err());
}

#[test]
fn an_unfinished_record_cannot_hide_forbidden_final_evidence() {
    let at = Instant::now();
    for tail in [b"batter-fixture:cleanup-invoked".as_slice(), b"batter-fixt"] {
        let mut output = Output::default();
        output.record(b"batter-fixture:evidence-ready\n", at);
        output.record(tail, at);
        let error = output
            .validate(&["evidence-ready"], &["cleanup-invoked"])
            .unwrap_err();
        assert!(error.contains("incomplete fixture record"), "{error}");
    }
}

#[test]
fn byte_overflow_invalidates_even_previously_observed_startup() {
    let at = Instant::now();
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-requested\n", at);
    output.record(b"thread 'fixture' panicked at deliberate failure\n", at);
    output.record(&vec![b'x'; OUTPUT_LIMIT], at);
    assert_eq!(output.bytes.len(), OUTPUT_LIMIT);
    assert!(
        output
            .event_by("drain-requested", at, at)
            .unwrap_err()
            .contains("capture limit")
    );
    assert!(output.validate(&["drain-requested"], &[]).is_err());
}

#[test]
fn distinct_event_metadata_is_bounded_and_overflow_fails_closed() {
    let at = Instant::now();
    let mut output = Output::default();
    for id in 0..=EVENT_LIMIT {
        output.record(format!("batter-fixture:event-{id}\n").as_bytes(), at);
    }
    assert_eq!(output.events.len(), EVENT_LIMIT);
    assert!(output.bytes.len() < OUTPUT_LIMIT);
    let error = output.event_by("event-0", at, at).unwrap_err();
    assert!(error.contains("64 distinct events"), "{error}");
    assert!(output.validate(&["event-0"], &[]).is_err());
    output.record(&vec![b'x'; OUTPUT_LIMIT], at);
    assert!(output.check().unwrap_err().contains("64 distinct events"));
}

#[test]
fn later_panic_keeps_captured_startup_but_invalidates_final_evidence() {
    let at = Instant::now();
    let mut output = Output::default();
    output.record(b"batter-fixture:drain-requested\n", at);
    let later = at + Duration::from_secs(2);
    output.record(b"thread 'fixture' panicked at deliberate failure\n", later);
    assert_eq!(
        output.event_by("drain-requested", at, later).unwrap(),
        Some(at)
    );
    let error = output.validate(&["drain-requested"], &[]).unwrap_err();
    assert!(error.contains("child panicked") && error.contains("deliberate failure"));
}

#[test]
fn panic_without_startup_still_fails_before_the_deadline() {
    let at = Instant::now();
    let mut output = Output::default();
    output.record(b"thread 'fixture' panicked at deliberate failure\n", at);
    let error = output
        .event_by("drain-requested", at + Duration::from_secs(5), at)
        .unwrap_err();
    assert!(error.contains("child panicked") && error.contains("deliberate failure"));
}
