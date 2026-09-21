//! Explicit reference-policy pools for compatibility fixtures. Close every new
//! pool before returning to the outer fixture owner, including ordinary errors.
use runledger_postgres::RunledgerDatabase;
use sqlx::PgPool;

pub async fn submit(
    pool: &PgPool,
    context: &batter::operation::OperationContext,
    owner: batter_example_reference_service::delivery::OwnerId,
    record: sqlx::types::Uuid,
    request: batter_example_reference_service::delivery::SubmitDelivery,
) -> Result<
    batter_example_reference_service::delivery::SubmitResult,
    batter_example_reference_service::delivery::SubmitError,
> {
    with(pool, async |database| {
        batter_example_reference_service::delivery::DeliveryService::new(database.clone())
            .submit(context, owner, record, request)
            .await
    })
    .await
}

pub fn configured(pool: &PgPool) -> RunledgerDatabase {
    batter_example_reference_service::database::configured(
        (*pool.connect_options()).clone(),
        pool.options().clone(),
    )
    .expect("reference test profile")
}

pub async fn with<T, E>(
    pool: &PgPool,
    work: impl AsyncFnOnce(&RunledgerDatabase) -> Result<T, E>,
) -> Result<T, E> {
    let database = configured(pool);
    let result = work(&database).await;
    database.pool().close().await;
    result
}

pub async fn initialize_schema(
    pool: &PgPool,
) -> Result<(), batter_example_reference_service::schema::SchemaInitializationError> {
    with(pool, async |database| {
        batter_example_reference_service::schema::initialize_schema(database).await
    })
    .await
}

pub async fn migrate(pool: &PgPool) -> Result<(), runledger_postgres::SchemaCompatibilityError> {
    with(pool, async |database| {
        runledger_postgres::migrate_after_idempotency_cutover(database).await
    })
    .await
}

pub async fn verify(
    pool: &PgPool,
) -> Result<
    runledger_postgres::SchemaCompatibilitySnapshot,
    runledger_postgres::SchemaCompatibilityError,
> {
    with(pool, async |database| {
        runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(database).await
    })
    .await
}
