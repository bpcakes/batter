//! Hold capacity by parking real admitted work, never by exposing its permit.
use super::bulkhead_foundation::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadError},
    operation::{OperationContext, OperationError},
};
use std::{
    future::{Future, pending},
    pin::Pin,
};

pub struct HeldWork {
    _work: Pin<Box<dyn Future<Output = Result<(), BulkheadError<()>>> + Send>>,
}

impl std::fmt::Debug for HeldWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HeldWork")
    }
}

pub trait BulkheadTestExt {
    fn hold(
        &self,
        context: &OperationContext,
        admission: Admission,
    ) -> impl Future<Output = Result<HeldWork, AdmissionError>> + Send;
}
impl BulkheadTestExt for Bulkhead {
    async fn hold(
        &self,
        context: &OperationContext,
        admission: Admission,
    ) -> Result<HeldWork, AdmissionError> {
        let bulkhead = self.clone();
        let context = context.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let mut work = Box::pin(async move {
            bulkhead
                .run(&context, admission, "test.held_work", |_| async move {
                    let _ = started.send(());
                    pending::<Result<(), ()>>().await
                })
                .await
        });
        tokio::select! {
            biased;
            result = &mut work => match result {
                Err(BulkheadError::NotAdmitted(error)) => Err(error),
                Err(BulkheadError::Operation(OperationError::Interrupted(reason))) =>
                    Err(AdmissionError::Interrupted(reason)),
                _ => panic!("parked work cannot return"),
            },
            ready = ready => {
                ready.expect("admitted work announces its start");
                Ok(HeldWork { _work: work })
            }
        }
    }
}
