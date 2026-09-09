use super::*;
use std::error::Error;

#[tokio::test]
async fn receipt_and_report_expose_the_same_concrete_source() {
    let supervisor = finite_supervisor(1);
    let process = supervisor.process_handle().unwrap();
    let running = supervisor.start();
    running.handle().wait_ready().await.unwrap();
    let receipt = process
        .try_spawn("shared-error", |_| async {
            Err::<(), _>(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
        })
        .unwrap();
    let failure = receipt.wait().await.unwrap_err();
    let report = running.wait().await.unwrap();
    assert_eq!(report.cause, ShutdownCause::FiniteTaskExit("shared-error"));
    let receipt_source = failure
        .source()
        .expect("receipt must expose its original cause")
        .downcast_ref::<std::io::Error>()
        .expect("source must be the concrete error, not its Arc owner");
    let ProcessTaskError::Failed(shared) = &failure else {
        panic!("typed failure must survive")
    };
    let report_source = report.tasks[0]
        .error
        .as_ref()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(receipt_source.kind(), std::io::ErrorKind::ConnectionRefused);
    assert!(std::ptr::eq(shared.as_ref(), receipt_source));
    assert!(std::ptr::eq(receipt_source, report_source));
}

#[test]
fn terminated_receipt_does_not_fabricate_a_source() {
    assert!(
        ProcessTaskError::<std::io::Error>::Terminated
            .source()
            .is_none()
    );
}
