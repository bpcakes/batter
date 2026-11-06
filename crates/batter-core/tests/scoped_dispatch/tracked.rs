use super::*;
use batter_core::operation::{Interruption, OperationOwner, TrackedOperationError};

#[test]
fn tracked_refusal_and_abort_preserve_scoped_destruction_and_one_observation() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    ambient.block_on(async {
        let owner = OperationOwner::new(Duration::from_secs(60)).unwrap();
        owner.cancel();
        let resource = DropTrace("tracked.unstarted");
        let result = owner.context().run_tracked("tracked.refused", move |_| async move {
            let _resource = resource;
            panic!("refused factory invoked");
            #[expect(unreachable_code, reason = "specifies the result type")]
            Ok::<(), ()>(())
        }).with_subscriber(scoped.dispatch.clone()).await;
        assert!(matches!(result, Err(TrackedOperationError::NotStarted(proof)) if proof.reason() == Interruption::Cancelled));

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let owner = OperationOwner::new(Duration::from_secs(60)).unwrap();
            owner.context().run_tracked("tracked.abort", |_| async move {
                let _resource = DropTrace("tracked.started");
                started_tx.send(()).unwrap();
                std::future::pending::<Result<(), ()>>().await
            }).await
        }.with_subscriber(scoped.dispatch.clone()));
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    });
    let text = scoped.text();
    assert_eq!(
        text.matches("operation boundary finished").count(),
        2,
        "{text}"
    );
    assert!(text.contains("tracked.unstarted"), "{text}");
    assert!(text.contains("tracked.started"), "{text}");
    assert!(text.contains("outcome=\"cancelled\""), "{text}");
    assert!(text.contains("outcome=\"dropped\""), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}
