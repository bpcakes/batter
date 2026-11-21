//! Exact delay endpoints, independent of database transaction timestamps.
use super::*;

#[test]
fn fixed_job_identities_reach_both_spread_endpoints() {
    // Fixed vectors protect the actual delay bound independently of the live
    // test's necessarily bracketed observation of the native completion clock.
    for (id, expected) in [
        (
            "00000000-0000-0000-0000-0000000016c5",
            Duration::from_secs(30),
        ),
        (
            "00000000-0000-0000-0000-000000000381",
            Duration::from_millis(37_500),
        ),
    ] {
        let mut job = context();
        job.job_id = serde_json::from_value(Value::String(id.into())).unwrap();
        assert_eq!(recheck().delay(&job), expected);
        job.run_number = 2;
        job.attempt = 3;
        assert_eq!(recheck().delay(&job), expected);
    }
}
