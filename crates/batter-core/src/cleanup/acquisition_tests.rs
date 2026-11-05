use super::*;
use crate::cleanup::{CleanupBudget, CleanupStack};
use std::{
    convert::Infallible,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Duration,
};

struct Resource(Arc<AtomicUsize>);
impl Drop for Resource {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct NativeReady(Option<Resource>);
impl Future for NativeReady {
    type Output = Result<Resource, Infallible>;
    fn poll(mut self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.0.take().unwrap()))
    }
}
impl Drop for NativeReady {
    fn drop(&mut self) {
        panic!("native destructor");
    }
}

#[tokio::test]
async fn raw_native_ready_resource_survives_future_destruction_panic_until_cleanup() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut stack = CleanupStack::new();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let future = stack
            .reserve("resource")
            .unwrap()
            .acquire_native(NativeReady(Some(Resource(dropped.clone()))));
        let mut future = pin!(future);
        let _ = future
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()));
    }));
    assert!(caught.is_err());
    assert_eq!(stack.len(), 1);
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        0,
        "registered ownership survived unwind"
    );
    let second = Duration::from_secs(1);
    assert!(
        stack
            .close(CleanupBudget::new(second, second, second).unwrap())
            .await
            .is_success()
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
