//! Repeated, forward-only startup for Runledger and application schema history.

use crate::delivery::delivery_job_spec;
use runledger_postgres::jobs::{
    JobDefinitionCatalogSyncError, JobDefinitionCatalogSyncMode, JobDefinitionUpsert,
    sync_catalog_job_definitions_tx,
};
use sqlx::PgPool;

/// Startup schema/producer-definition failure with every native cause retained.
#[derive(Debug, thiserror::Error)]
pub enum SchemaInitializationError {
    /// Runledger migration or compatibility failure.
    #[error("Runledger schema initialization failed")]
    RunledgerSchema(#[source] runledger_postgres::SchemaCompatibilityError),
    /// Application migration failure.
    #[error("reference application migration failed")]
    ApplicationMigration(#[source] sqlx::migrate::MigrateError),
    /// Static producer definition is invalid.
    #[error("delivery job definition is invalid")]
    JobSpec(#[source] runledger_core::jobs::JobSpecError),
    /// Definition transaction could not begin.
    #[error("delivery job definition transaction failed to begin")]
    DefinitionBegin(#[source] sqlx::Error),
    /// Definition synchronization failed.
    #[error("delivery job definition synchronization failed")]
    DefinitionSync(#[source] JobDefinitionCatalogSyncError),
    /// Definition transaction commit was not acknowledged.
    #[error("delivery job definition commit failed")]
    DefinitionCommit(#[source] sqlx::Error),
}

/// Apply both migration histories and synchronize the producer-only job definition.
///
/// Safe to call on repeated startup. Existing operator-disabled Runledger job
/// definitions remain disabled. This function registers no handler and starts
/// no worker.
pub async fn initialize_schema(pool: &PgPool) -> Result<(), SchemaInitializationError> {
    runledger_postgres::migrate_after_idempotency_cutover(pool)
        .await
        .map_err(SchemaInitializationError::RunledgerSchema)?;

    let mut application = sqlx::migrate!("./migrations");
    application.set_ignore_missing(true);
    application
        .run(pool)
        .await
        .map_err(SchemaInitializationError::ApplicationMigration)?;

    runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(pool)
        .await
        .map_err(SchemaInitializationError::RunledgerSchema)?;

    let spec = delivery_job_spec().map_err(SchemaInitializationError::JobSpec)?;
    let definition = JobDefinitionUpsert::from(&spec);
    let mut transaction = pool
        .begin()
        .await
        .map_err(SchemaInitializationError::DefinitionBegin)?;
    sync_catalog_job_definitions_tx(
        &mut transaction,
        &[definition],
        JobDefinitionCatalogSyncMode::PreserveExistingEnabledForEnabledDefinitions,
    )
    .await
    .map_err(SchemaInitializationError::DefinitionSync)?;
    transaction
        .commit()
        .await
        .map_err(SchemaInitializationError::DefinitionCommit)
}
