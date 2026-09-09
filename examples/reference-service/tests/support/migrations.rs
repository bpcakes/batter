use sqlx::{PgPool, migrate::Migrate};

use super::ProbeResult;

pub async fn upgrade_probe(pool: PgPool) -> ProbeResult {
    initialize_previous_schema(&pool).await?;
    let mut application = sqlx::migrate!("./migrations");
    application.set_ignore_missing(true);
    application.run(&pool).await?;
    let inserted = sqlx::query("INSERT INTO compatibility_probe (value) VALUES (42)")
        .execute(&pool)
        .await?;
    assert_eq!(inserted.rows_affected(), 1);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    let missing =
        runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(&pool).await;
    assert!(
        matches!(
            missing,
            Err(runledger_postgres::SchemaCompatibilityError::Incompatible(
                sqlx::migrate::MigrateError::VersionTooNew(202609050001, _)
            ))
        ),
        "old initialized schema must be rejected by current startup"
    );
    runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
    runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(&pool).await?;
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    assert!(after > before);
    let values: Vec<i64> = sqlx::query_scalar("SELECT value FROM compatibility_probe")
        .fetch_all(&pool)
        .await?;
    assert_eq!(values, [42]);
    application.run(&pool).await?;
    runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
    let repeated: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(after, repeated);
    Ok(())
}

async fn initialize_previous_schema(pool: &PgPool) -> ProbeResult {
    // Fixture-only initialization of the precise pre-index schema. Inspect the
    // public bundled migrations; never run/undo the raw upstream migrator on a
    // shared pool. The upgrade itself uses the public cutover-aware entrypoint.
    let previous_version = 202608240002;
    let mut connection = pool.acquire().await?;
    connection.close_on_drop();
    connection
        .ensure_migrations_table("_sqlx_migrations")
        .await?;
    for migration in runledger_postgres::MIGRATOR.iter().filter(|migration| {
        migration.migration_type.is_up_migration() && migration.version <= previous_version
    }) {
        connection.apply("_sqlx_migrations", migration).await?;
    }
    connection.close().await?;
    Ok(())
}
