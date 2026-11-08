//! A per-service test seam after actual transaction acknowledgement and retention.
use super::*;
use batter::{
    BoxError,
    admission::BulkheadCapacity,
    operation::{Interruption, OperationOwner},
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub(in crate::delivery) struct AfterSettlement {
    owner: OperationOwner,
    fired: AtomicBool,
}
impl AfterSettlement {
    pub(super) async fn interrupt(&self) {
        self.fired.store(true, Ordering::SeqCst);
        self.owner.cancel();
        // Force the operation boundary to observe interruption instead of a
        // returned marker. Production has no suspension here; this is a test seam.
        std::future::pending::<()>().await;
    }
}

pub(in crate::delivery) async fn admitted_commit(
    database: &runledger_postgres::RunledgerDatabase,
) -> Result<(), BoxError> {
    let owner = OwnerId::new(Uuid::now_v7())?;
    let record = Uuid::now_v7();
    sqlx::query("INSERT INTO reference_records (id, owner_id, generation) VALUES ($1, $2, 1)")
        .bind(record)
        .bind(owner.as_uuid())
        .execute(database.pool())
        .await?;
    let probe = Arc::new(AfterSettlement {
        owner: OperationOwner::new(Duration::from_secs(5))?,
        fired: AtomicBool::new(false),
    });
    let mut service = DeliveryService::new(database.clone());
    service.after_settlement = Some(probe.clone());
    let bulkhead = Bulkhead::new(BulkheadCapacity::new(1)?);
    let result = service
        .submit_admitted(
            probe.owner.context(),
            &bulkhead,
            owner,
            record,
            SubmitDelivery {
                expected_generation: 1,
                idempotency_key: record.to_string(),
                payload: json!({"scenario":"admitted-commit"}),
            },
        )
        .await?;
    assert!(
        probe.fired.load(Ordering::SeqCst),
        "acknowledged transaction reached seam"
    );
    assert_eq!(probe.owner.context().check(), Err(Interruption::Cancelled));
    assert_eq!(result.disposition, SubmitDisposition::Accepted);
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reference_delivery_commands c
         JOIN reference_deliveries d ON d.id = c.delivery_id
         JOIN reference_delivery_effects e ON e.delivery_id = d.id
         JOIN job_queue j ON j.id = d.job_id
         WHERE c.delivery_id = $1 AND c.owner_id = $2",
    )
    .bind(result.delivery.delivery_id)
    .bind(owner.as_uuid())
    .fetch_one(database.pool())
    .await?;
    assert_eq!(
        rows, 1,
        "command, effect, delivery and native job committed together"
    );
    // Run the same service and bulkhead again: released capacity and exact
    // replay must produce the retained identity with no second enqueue.
    service.after_settlement = None;
    let fresh = OperationOwner::new(Duration::from_secs(5))?;
    let replay = service
        .submit_admitted(
            fresh.context(),
            &bulkhead,
            owner,
            record,
            SubmitDelivery {
                expected_generation: 1,
                idempotency_key: record.to_string(),
                payload: json!({"scenario":"admitted-commit"}),
            },
        )
        .await?;
    assert_eq!(replay.disposition, SubmitDisposition::Replayed);
    assert_eq!(replay.delivery, result.delivery);
    Ok(())
}
