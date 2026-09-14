use batter::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadCapacity},
    operation::{Interruption, OperationContext},
};
use std::time::Duration;

fn context() -> OperationContext {
    OperationContext::new(Duration::from_secs(10)).unwrap()
}

#[test]
fn capacity_must_be_valid() {
    assert!(BulkheadCapacity::new(0).is_err());
    assert!(BulkheadCapacity::new(tokio::sync::Semaphore::MAX_PERMITS + 1).is_err());
    assert!(BulkheadCapacity::new(usize::MAX).is_err());
    assert!(BulkheadCapacity::new(tokio::sync::Semaphore::MAX_PERMITS).is_ok());
}

#[tokio::test(start_paused = true)]
async fn rejecting_admission_enforces_capacity_and_releases_on_drop() {
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let context = context();
    let permit = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    assert!(matches!(
        bulkhead.enter(&context, Admission::Reject).await,
        Err(AdmissionError::Overloaded)
    ));
    drop(permit);
    assert!(bulkhead.enter(&context, Admission::Reject).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn waiting_consumes_existing_deadline_without_leaking_a_permit() {
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let permit = bulkhead.enter(&context(), Admission::Reject).await.unwrap();
    let short = OperationContext::new(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        bulkhead.enter(&short, Admission::Wait).await,
        Err(AdmissionError::Interrupted(Interruption::DeadlineExceeded))
    ));
    drop(permit);
    assert!(bulkhead.enter(&context(), Admission::Reject).await.is_ok());
}

#[tokio::test]
async fn closing_wakes_waiters() {
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    let _permit = bulkhead.enter(&context(), Admission::Reject).await.unwrap();
    let owned = bulkhead.clone();
    let task = tokio::spawn(async move { owned.enter(&context(), Admission::Wait).await });
    tokio::task::yield_now().await;
    bulkhead.close();
    assert!(matches!(task.await.unwrap(), Err(AdmissionError::Closed)));
}

#[tokio::test(start_paused = true)]
async fn cancelled_call_is_not_admitted_even_when_capacity_exists() {
    let context = context();
    context.cancel();
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1).unwrap());
    assert!(matches!(
        bulkhead.enter(&context, Admission::Reject).await,
        Err(AdmissionError::Interrupted(Interruption::Cancelled))
    ));
}
