use super::*;
use std::os::unix::process::ExitStatusExt;

fn cleanup_error() -> io::Error {
    io::Error::other("private-cleanup-marker")
}

fn launch_error() -> io::Error {
    io::Error::other("private-launch-marker")
}

fn report(error: Cause) -> Box<HarnessFailure> {
    assert!(!format!("{error:?} {error}").contains("private-"));
    error.downcast::<HarnessFailure>().unwrap()
}

fn assert_cleanup(error: &Cause) {
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().to_string(),
        "private-cleanup-marker"
    );
}

#[test]
fn launch_and_teardown_failures_remain_independently_inspectable() {
    let failure = report(finish(Err(launch_error()), Err(cleanup_error())).unwrap_err());
    let TestFailure::Both {
        body: ExecutionFailure::Launch(launch),
        cleanup,
    } = &failure.0
    else {
        panic!("expected both original causes")
    };
    assert_eq!(launch.to_string(), "private-launch-marker");
    assert_cleanup(cleanup);
    let source = failure.source().unwrap();
    assert!(!format!("{source:?} {source}").contains("private-"));
    assert!(source.source().unwrap().is::<io::Error>());
}

#[test]
fn unsuccessful_exit_and_signal_remain_inspectable_with_teardown_failure() {
    for status in [ExitStatus::from_raw(7 << 8), ExitStatus::from_raw(15)] {
        let failure = report(finish(Ok(status), Err(cleanup_error())).unwrap_err());
        let TestFailure::Both {
            body: ExecutionFailure::Exit(observed),
            cleanup,
        } = &failure.0
        else {
            panic!("expected exit evidence and cleanup cause")
        };
        assert_eq!(*observed, status);
        assert_cleanup(cleanup);
        assert!(format!("{failure:?}").contains(&format!("{status:?}")));
    }
}

#[test]
fn single_failures_and_success_keep_their_original_classification() {
    let failure = report(finish(Err(launch_error()), Ok::<(), io::Error>(())).unwrap_err());
    assert!(
        matches!(&failure.0, TestFailure::Body(ExecutionFailure::Launch(error))
        if error.to_string() == "private-launch-marker")
    );
    let status = ExitStatus::from_raw(9 << 8);
    let failure = report(finish(Ok(status), Ok::<(), io::Error>(())).unwrap_err());
    assert!(
        matches!(&failure.0, TestFailure::Body(ExecutionFailure::Exit(observed))
        if *observed == status)
    );
    let success = ExitStatus::from_raw(0);
    let failure = report(finish(Ok(success), Err(cleanup_error())).unwrap_err());
    let TestFailure::Cleanup(cleanup) = &failure.0 else {
        panic!("expected cleanup-only failure")
    };
    assert_cleanup(cleanup);
    assert!(finish(Ok(success), Ok::<(), io::Error>(())).is_ok());
}
