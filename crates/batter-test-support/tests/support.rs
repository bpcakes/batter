use batter_test_support::{Script, ScriptError, TestFailure, finish};

#[test]
fn success_survives_successful_teardown() {
    assert_eq!(finish::<_, &str, &str>(Ok(42), Ok(())).unwrap(), 42);
}

#[test]
fn body_and_teardown_failures_both_survive() {
    let result = finish::<(), _, _>(Err("body"), Err("cleanup"));
    assert!(matches!(
        result,
        Err(TestFailure::Both {
            body: "body",
            cleanup: "cleanup"
        })
    ));
}

#[test]
fn body_only_failure_remains_distinct() {
    assert!(matches!(
        finish::<(), _, &str>(Err("body"), Ok(())),
        Err(TestFailure::Body("body"))
    ));
}

#[test]
fn cleanup_only_failure_remains_distinct() {
    assert!(matches!(
        finish::<(), &str, _>(Ok(()), Err("cleanup")),
        Err(TestFailure::Cleanup("cleanup"))
    ));
}

#[test]
fn script_clones_share_outcomes_and_never_fabricate_a_result() {
    let script = Script::new([Err("busy"), Ok(42)]);
    let other = script.clone();
    assert!(matches!(script.next(), Err(ScriptError::Expected("busy"))));
    assert_eq!(other.next().unwrap(), 42);
    assert!(matches!(script.next(), Err(ScriptError::Exhausted)));
    assert_eq!(script.calls(), 3);
    assert_eq!(other.calls(), 3);
}
