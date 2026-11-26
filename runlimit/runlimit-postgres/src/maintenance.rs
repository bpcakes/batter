use crate::{
    CleanupPhase, MaintenanceError,
    protocol::CLEANUP_SQL,
    transaction::{BoundedConnection, ConnectionOutcome, Maintenance},
};

/// Deletes at most `maximum_rows` expired fixed windows in one bounded
/// transaction and returns how many it removed.
pub(crate) async fn run_cleanup_transaction(
    connection: &mut BoundedConnection<Maintenance>,
    maximum_rows: u32,
) -> Result<u64, ConnectionOutcome<MaintenanceError>> {
    let mut transaction = connection.begin().await?;
    let deleted = transaction
        .execute(
            CleanupPhase::DeletingExpiredWindows,
            sqlx::query(CLEANUP_SQL).bind(i64::from(maximum_rows)),
        )
        .await?;
    transaction.commit().await?;
    Ok(deleted.rows_affected())
}
