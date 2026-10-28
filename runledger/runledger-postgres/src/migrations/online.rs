//! Online index builds for bundled migrations whose ordinary `CREATE INDEX`
//! statements would block queue writes on populated tables.
//!
//! The canonical migration SQL stays byte-identical and keeps its SQLx
//! checksum. [`run_migrations_with_filtered_history`] records each declared
//! migration under that checksum but executes only the residual statements in
//! the migration transaction, then builds every declared index with
//! `CREATE INDEX CONCURRENTLY` outside a transaction. A build that fails, or a
//! process that dies between the recorded migration and the build, leaves a
//! missing or `INVALID` index; the next protected startup drops and rebuilds
//! it, and read-only verification reports it instead.

use std::collections::HashSet;

use sqlx::migrate::{Migrate, Migration};
use sqlx::{AssertSqlSafe, PgConnection, SqlSafeStr};

use super::{
    MIGRATOR, OnlineIndexExecutionError, OnlineIndexPhase, SchemaCompatibilityError,
    applied_runledger_migrations, expected_runledger_migrations,
    first_conflicting_runledger_version, first_dirty_runledger_version,
    first_missing_runledger_version, has_migrations_table, has_runledger_migration_history_table,
    list_migration_history, list_recorded_runledger_migrations, validate_checksum,
};
use crate::{PgQueryExecutor, RunledgerDatabase};

mod definition;
mod lock;
mod repair;
mod report;
mod session;
use definition::definition_matches;
pub(super) use lock::MigrationLock;
pub use report::{OnlineIndexPreparation, OnlineIndexProblem};
use session::ProfileTimeouts;

/// One bundled index whose ordinary build blocks writes on a populated table.
#[derive(Clone, Copy)]
struct OnlineIndex {
    migration_version: i64,
    table: &'static str,
    name: &'static str,
    /// The statement exactly as the canonical migration spells it, including
    /// the terminating semicolon. Removing it leaves the residual migration.
    canonical_statement: &'static str,
    /// The `pg_get_indexdef` rendering after `USING`, without schema
    /// qualification, as PostgreSQL 18 renders the canonical build.
    expected_definition: &'static str,
    /// The same canonical body with `quote_all_identifiers` enabled.
    expected_quoted_definition: &'static str,
}

impl OnlineIndex {
    /// Whether the canonical statement keeps an existing index, so an external
    /// migrator applying the canonical SQL adopts a prebuilt one.
    fn adoptable_by_canonical_sql(&self) -> bool {
        self.canonical_statement
            .starts_with("CREATE INDEX IF NOT EXISTS ")
    }

    /// The canonical statement as a concurrent build. It never uses
    /// `IF NOT EXISTS`, which would keep an `INVALID` leftover.
    fn concurrent_statement(&self) -> String {
        let body = self
            .canonical_statement
            .strip_prefix("CREATE INDEX IF NOT EXISTS ")
            .or_else(|| self.canonical_statement.strip_prefix("CREATE INDEX "))
            .unwrap_or(self.canonical_statement);
        format!("CREATE INDEX CONCURRENTLY {body}")
    }
}

const ONLINE_INDEXES: [OnlineIndex; 5] = [
    OnlineIndex {
        migration_version: 202605220001,
        table: "job_queue",
        name: "idx_job_queue_missing_enqueue_request_snapshot",
        canonical_statement:
            "CREATE INDEX IF NOT EXISTS idx_job_queue_missing_enqueue_request_snapshot
    ON job_queue (id)
    WHERE idempotency_key IS NOT NULL
      AND enqueue_request IS NULL;",
        expected_definition: "btree (id) WHERE ((idempotency_key IS NOT NULL) AND (enqueue_request IS NULL))",
        expected_quoted_definition: r#""btree" ("id") WHERE (("idempotency_key" IS NOT NULL) AND ("enqueue_request" IS NULL))"#,
    },
    OnlineIndex {
        migration_version: 202605220001,
        table: "workflow_runs",
        name: "idx_workflow_runs_missing_enqueue_request_snapshot",
        canonical_statement:
            "CREATE INDEX IF NOT EXISTS idx_workflow_runs_missing_enqueue_request_snapshot
    ON workflow_runs (id)
    WHERE idempotency_key IS NOT NULL
      AND enqueue_request IS NULL;",
        expected_definition: "btree (id) WHERE ((idempotency_key IS NOT NULL) AND (enqueue_request IS NULL))",
        expected_quoted_definition: r#""btree" ("id") WHERE (("idempotency_key" IS NOT NULL) AND ("enqueue_request" IS NULL))"#,
    },
    OnlineIndex {
        migration_version: 202607280004,
        table: "job_queue",
        name: "idx_job_queue_execution_resource_claim_order",
        canonical_statement: "CREATE INDEX idx_job_queue_execution_resource_claim_order
    ON job_queue (
        priority DESC,
        next_run_at,
        created_at,
        id
    )
    INCLUDE (execution_resource_key, job_type)
    WHERE status = 'PENDING'
      AND execution_resource_key IS NOT NULL;",
        expected_definition: "btree (priority DESC, next_run_at, created_at, id) INCLUDE (execution_resource_key, job_type) WHERE ((status = 'PENDING'::job_status) AND (execution_resource_key IS NOT NULL))",
        expected_quoted_definition: r#""btree" ("priority" DESC, "next_run_at", "created_at", "id") INCLUDE ("execution_resource_key", "job_type") WHERE (("status" = 'PENDING'::"job_status") AND ("execution_resource_key" IS NOT NULL))"#,
    },
    OnlineIndex {
        migration_version: 202609050001,
        table: "job_queue",
        name: "idx_job_queue_scope_created_id",
        canonical_statement: "CREATE INDEX idx_job_queue_scope_created_id
    ON job_queue (organization_id, created_at DESC, id DESC);",
        expected_definition: "btree (organization_id, created_at DESC, id DESC)",
        expected_quoted_definition: r#""btree" ("organization_id", "created_at" DESC, "id" DESC)"#,
    },
    OnlineIndex {
        migration_version: 202609050001,
        table: "job_queue",
        name: "idx_job_queue_created_id",
        canonical_statement: "CREATE INDEX idx_job_queue_created_id
    ON job_queue (created_at DESC, id DESC);",
        expected_definition: "btree (created_at DESC, id DESC)",
        expected_quoted_definition: r#""btree" ("created_at" DESC, "id" DESC)"#,
    },
];

enum IndexState {
    Missing,
    Invalid(i64),
    Building,
    /// A relation with the declared name that is not an index on the declared
    /// table: another kind of relation, or an index elsewhere.
    Foreign,
    Valid(String),
}

#[derive(Clone, Copy)]
enum BuildOutcome {
    Built,
    AlreadyValid,
}

#[derive(Clone, Copy)]
enum IndexHistory {
    Recorded,
    Pending,
}

fn execution_error(
    index: &OnlineIndex,
    history: IndexHistory,
    phase: OnlineIndexPhase,
    cause: SchemaCompatibilityError,
    restoration_error: Option<sqlx::Error>,
) -> SchemaCompatibilityError {
    SchemaCompatibilityError::OnlineIndexExecution(OnlineIndexExecutionError::new(
        index.name,
        matches!(history, IndexHistory::Recorded).then_some(index.migration_version),
        phase,
        cause,
        restoration_error,
    ))
}

fn problem(index: &OnlineIndex, problem: OnlineIndexProblem) -> SchemaCompatibilityError {
    SchemaCompatibilityError::OnlineIndex {
        index: index.name,
        problem,
    }
}

fn check_valid(
    index: &OnlineIndex,
    definition: &str,
    schema: &str,
) -> Result<(), SchemaCompatibilityError> {
    if definition_matches(index, definition, schema) {
        Ok(())
    } else {
        Err(problem(index, OnlineIndexProblem::DefinitionMismatch))
    }
}

/// The relation holding a declared index name, whatever its kind. The index
/// columns are `NULL` for a relation that is not an index.
#[derive(sqlx::FromRow)]
struct DeclaredNameRow {
    oid: i64,
    valid: Option<bool>,
    building: bool,
    table: Option<String>,
    definition: Option<String>,
}

async fn index_state(
    conn: &mut impl PgQueryExecutor,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
) -> Result<IndexState, sqlx::Error> {
    // Any relation with the declared name is looked up, so a same-named
    // relation of another kind or on another table is refused before the
    // migration is recorded instead of failing the later build. Other roles'
    // progress may hide the index but still exposes pid/datid. Correlate that
    // backend's table lock so manual builders need no migration-lock protocol.
    // Hidden progress can conservatively defer other indexes on the same table;
    // visible concurrent reindex names its replacement but locks the original.
    // The migration-lock fallback also covers builds not yet in the catalog.
    let row: Option<DeclaredNameRow> = sqlx::query_as(
        "SELECT i.indisvalid AS valid, c.oid::bigint AS oid,
                    EXISTS (
                        SELECT 1 FROM pg_catalog.pg_stat_progress_create_index p
                        WHERE p.datid = (
                              SELECT oid FROM pg_catalog.pg_database
                              WHERE datname = pg_catalog.current_database()
                          )
                          AND (p.index_relid = c.oid OR EXISTS (
                              SELECT 1 FROM pg_catalog.pg_locks l
                              WHERE l.pid = p.pid AND l.database = p.datid
                                AND l.locktype = 'relation' AND l.granted
                                AND ((p.index_relid IS NULL AND l.relation = i.indrelid
                                      AND l.mode IN ('ShareUpdateExclusiveLock', 'ShareLock'))
                                     OR (p.command = 'REINDEX CONCURRENTLY'
                                         AND l.relation = c.oid
                                         AND l.mode = 'ShareUpdateExclusiveLock'))
                          ))
                    ) AS building,
                    t.relname::text AS \"table\",
                    CASE WHEN i.indexrelid IS NULL THEN NULL
                         ELSE pg_catalog.pg_get_indexdef(i.indexrelid) END AS definition
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_catalog.pg_index i ON i.indexrelid = c.oid
             LEFT JOIN pg_catalog.pg_class t ON t.oid = i.indrelid
             WHERE n.nspname = $1 AND c.relname = $2",
    )
    .bind(database.schema())
    .bind(index.name)
    .fetch_optional(conn.executor())
    .await?;
    Ok(match row {
        // Between a recorded migration and the start of its build there is no
        // catalog row yet, but the builder already holds the migration lock.
        None if MigrationLock::held_by_another_session(conn).await? => IndexState::Building,
        None => IndexState::Missing,
        Some(DeclaredNameRow {
            oid,
            valid: Some(valid),
            building,
            table: Some(table),
            definition,
        }) if table == index.table => {
            // pg_get_indexdef reads current syscache state, while the joined
            // catalog rows can retain an index already dropped concurrently.
            // valid: Some and the table guard distinguish it from a foreign
            // non-index relation or an index on another table.
            let Some(definition) = definition else {
                return Ok(if MigrationLock::held_by_another_session(conn).await? {
                    IndexState::Building
                } else {
                    IndexState::Missing
                });
            };
            if valid {
                IndexState::Valid(definition)
            } else if !definition_matches(index, &definition, database.schema()) {
                // An INVALID leftover of someone else's build is refused like
                // any other foreign relation, never dropped.
                IndexState::Foreign
            } else if building || MigrationLock::held_by_another_session(conn).await? {
                IndexState::Building
            } else {
                IndexState::Invalid(oid)
            }
        }
        Some(_) => IndexState::Foreign,
    })
}

async fn table_exists(
    conn: &mut impl PgQueryExecutor,
    database: &RunledgerDatabase,
    table: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>("SELECT pg_catalog.to_regclass($1) IS NOT NULL")
        .bind(database.relation(table))
        .fetch_one(conn.executor())
        .await
}

/// Remove an inspected `INVALID` leftover under protected metadata locks,
/// then build the declared index concurrently and check the result.
async fn build_concurrently(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
    invalid_oid: Option<i64>,
) -> Result<BuildOutcome, SchemaCompatibilityError> {
    if let Some(oid) = invalid_oid
        && !repair::remove_invalid(conn, database, index, oid).await?
    {
        return Ok(BuildOutcome::AlreadyValid);
    }
    sqlx::raw_sql(AssertSqlSafe(index.concurrent_statement()))
        .execute(&mut *conn)
        .await?;
    match index_state(conn, database, index).await? {
        IndexState::Valid(definition) => {
            check_valid(index, &definition, database.schema()).map(|()| BuildOutcome::Built)
        }
        IndexState::Building => Err(problem(index, OnlineIndexProblem::BuildInProgress)),
        IndexState::Foreign => Err(problem(index, OnlineIndexProblem::DefinitionMismatch)),
        IndexState::Invalid(_) => Err(problem(index, OnlineIndexProblem::Invalid)),
        IndexState::Missing => Err(problem(index, OnlineIndexProblem::Missing)),
    }
}

/// Leave the declared index valid with the bundled definition. Runs outside
/// any transaction so `CONCURRENTLY` is permitted, and with the profile's
/// timeouts lifted only for the build so it can wait for in-flight
/// transactions while the rest of the startup keeps the profile's bounds.
async fn ensure_index_built(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
    history: IndexHistory,
) -> Result<BuildOutcome, SchemaCompatibilityError> {
    let invalid_oid = match index_state(conn, database, index).await.map_err(|error| {
        execution_error(
            index,
            history,
            OnlineIndexPhase::Inspect,
            error.into(),
            None,
        )
    })? {
        IndexState::Valid(definition) => {
            return check_valid(index, &definition, database.schema())
                .map(|()| BuildOutcome::AlreadyValid);
        }
        IndexState::Building => return Err(problem(index, OnlineIndexProblem::BuildInProgress)),
        IndexState::Foreign => {
            return Err(problem(index, OnlineIndexProblem::DefinitionMismatch));
        }
        IndexState::Invalid(oid) => Some(oid),
        IndexState::Missing => None,
    };
    let timeouts = ProfileTimeouts::lift(conn).await.map_err(|error| {
        execution_error(
            index,
            history,
            OnlineIndexPhase::Configure,
            error.into(),
            None,
        )
    })?;
    let built = build_concurrently(conn, database, index, invalid_oid).await;
    let restored = timeouts.restore(conn).await;
    match (built, restored) {
        (Err(error), restored) => Err(execution_error(
            index,
            history,
            OnlineIndexPhase::Build,
            error,
            restored.err(),
        )),
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(_), Err(error)) => Err(execution_error(
            index,
            history,
            OnlineIndexPhase::Restore,
            error.into(),
            None,
        )),
    }
}

fn residual_sql(
    migration: &Migration,
    declared: &[&OnlineIndex],
) -> Result<String, SchemaCompatibilityError> {
    let mut residual = migration.sql.as_str().to_owned();
    for index in declared {
        if residual.matches(index.canonical_statement).count() != 1 {
            return Err(problem(index, OnlineIndexProblem::StatementNotEmbedded));
        }
        residual = residual.replacen(index.canonical_statement, "", 1);
    }
    Ok(residual)
}

/// Record one pending migration under its canonical checksum while building
/// its declared indexes concurrently after the residual statements commit.
async fn apply_online(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    migration: &Migration,
) -> Result<(), SchemaCompatibilityError> {
    let declared: Vec<&OnlineIndex> = ONLINE_INDEXES
        .iter()
        .filter(|index| index.migration_version == migration.version)
        .collect();
    if declared.is_empty() {
        conn.apply("_sqlx_migrations", migration).await?;
        return Ok(());
    }
    // Refuse a foreign relation under a declared name before recording
    // anything; the migration would otherwise be recorded and its build fail.
    for index in &declared {
        match index_state(conn, database, index).await? {
            IndexState::Valid(definition) => {
                check_valid(index, &definition, database.schema())?;
            }
            IndexState::Foreign => {
                return Err(problem(index, OnlineIndexProblem::DefinitionMismatch));
            }
            IndexState::Building => {
                return Err(problem(index, OnlineIndexProblem::BuildInProgress));
            }
            IndexState::Invalid(_) | IndexState::Missing => {}
        }
    }
    let rendition = Migration {
        version: migration.version,
        description: migration.description.clone(),
        migration_type: migration.migration_type,
        sql: AssertSqlSafe(residual_sql(migration, &declared)?).into_sql_str(),
        checksum: migration.checksum.clone(),
        no_tx: migration.no_tx,
    };
    conn.apply("_sqlx_migrations", &rendition).await?;
    for index in declared {
        ensure_index_built(conn, database, index, IndexHistory::Recorded).await?;
    }
    Ok(())
}

/// Apply pending bundled migrations through Runledger's filtered history,
/// building declared indexes online, then leave every declared index of an
/// applied migration valid. The caller holds the migration lock.
pub(super) async fn run_migrations_with_filtered_history(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
) -> Result<(), SchemaCompatibilityError> {
    conn.ensure_migrations_table("_sqlx_migrations").await?;

    let expected_migrations = expected_runledger_migrations();
    let history = checked_history(conn, database, &expected_migrations).await?;

    let applied_by_version: std::collections::HashMap<_, _> =
        applied_runledger_migrations(&history, &expected_migrations)
            .into_iter()
            .map(|migration| (migration.version, migration))
            .collect();

    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.migration_type.is_up_migration())
    {
        match applied_by_version.get(&migration.version) {
            Some(applied_migration) => {
                validate_checksum(migration.version, applied_migration, migration)?;
            }
            None => apply_online(conn, database, migration).await?,
        }
    }

    // Every declared migration is applied now; repair an index lost between a
    // recorded migration and its concurrent build.
    for index in &ONLINE_INDEXES {
        ensure_index_built(conn, database, index, IndexHistory::Recorded).await?;
    }
    Ok(())
}

/// Read-only check that every declared index of an applied migration is valid
/// with the bundled definition.
pub(super) async fn verify_applied_indexes(
    conn: &mut impl PgQueryExecutor,
    database: &RunledgerDatabase,
    applied_versions: &HashSet<i64>,
) -> Result<(), SchemaCompatibilityError> {
    for index in &ONLINE_INDEXES {
        if !applied_versions.contains(&index.migration_version) {
            continue;
        }
        match index_state(conn, database, index).await? {
            IndexState::Valid(definition) => {
                check_valid(index, &definition, database.schema())?;
            }
            IndexState::Building => {
                return Err(problem(index, OnlineIndexProblem::BuildInProgress));
            }
            IndexState::Foreign => {
                return Err(problem(index, OnlineIndexProblem::DefinitionMismatch));
            }
            IndexState::Invalid(_) => return Err(problem(index, OnlineIndexProblem::Invalid)),
            IndexState::Missing => return Err(problem(index, OnlineIndexProblem::Missing)),
        }
    }
    Ok(())
}

/// Both migration entry points reject conflicting, dirty or incompatible newer
/// history before any index work, including a fence without SQLx history.
async fn checked_history(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    expected: &super::RunledgerMigrationMap,
) -> Result<Vec<super::MigrationHistoryRow>, SchemaCompatibilityError> {
    let history = if has_migrations_table(conn, database).await? {
        list_migration_history(conn, database).await?
    } else {
        Vec::new()
    };
    if let Some(version) = first_conflicting_runledger_version(&history, expected) {
        return Err(sqlx::migrate::MigrateError::VersionMismatch(version).into());
    }
    if let Some(version) = first_dirty_runledger_version(&history, expected) {
        return Err(sqlx::migrate::MigrateError::Dirty(version).into());
    }
    if has_runledger_migration_history_table(conn, database).await? {
        let recorded = list_recorded_runledger_migrations(conn, database).await?;
        if let Some(version) = first_missing_runledger_version(&recorded, expected) {
            return Err(sqlx::migrate::MigrateError::VersionMissing(version).into());
        }
    }
    Ok(history)
}

async fn prepare_adoptable_indexes(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
) -> Result<OnlineIndexPreparation, SchemaCompatibilityError> {
    let expected = expected_runledger_migrations();
    let history = checked_history(conn, database, &expected).await?;
    let applied: HashSet<_> = applied_runledger_migrations(&history, &expected)
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    let mut report = OnlineIndexPreparation::default();
    let mut eligible = Vec::new();
    for index in &ONLINE_INDEXES {
        if applied.contains(&index.migration_version) {
            continue;
        }
        let table_exists = table_exists(conn, database, index.table).await?;
        if !index.adoptable_by_canonical_sql() {
            if table_exists {
                match index_state(conn, database, index).await? {
                    IndexState::Missing => report.external_build_blocks.push(index.name),
                    IndexState::Foreign => {
                        return Err(problem(index, OnlineIndexProblem::DefinitionMismatch));
                    }
                    IndexState::Building => {
                        return Err(problem(index, OnlineIndexProblem::BuildInProgress));
                    }
                    IndexState::Valid(definition) => {
                        check_valid(index, &definition, database.schema())?;
                        return Err(problem(index, OnlineIndexProblem::ExternalCannotAdopt));
                    }
                    IndexState::Invalid(_) => {
                        return Err(problem(index, OnlineIndexProblem::ExternalCannotAdopt));
                    }
                }
            } else {
                report.deferred.push(index.name);
            }
            continue;
        }
        // The index can only be built on the schema its migration expects:
        // every earlier bundled migration must already be applied, which also
        // covers a table the baseline has not created yet.
        let earlier_pending = expected
            .keys()
            .any(|version| *version < index.migration_version && !applied.contains(version));
        if earlier_pending || !table_exists {
            report.deferred.push(index.name);
            continue;
        }
        eligible.push(index);
    }
    // Refuse occupied non-adoptable names before doing any preparation work.
    for index in eligible {
        match ensure_index_built(conn, database, index, IndexHistory::Pending).await? {
            BuildOutcome::Built => report.built.push(index.name),
            BuildOutcome::AlreadyValid => report.already_valid.push(index.name),
        }
    }
    Ok(report)
}

/// Build, outside any transaction, the pending bundled indexes that an
/// external migrator applying the canonical SQL can adopt.
///
/// Call this before external tooling applies pending Runledger migrations to a
/// populated database. Each adoptable index is built with
/// `CREATE INDEX CONCURRENTLY` while the application keeps writing; the
/// canonical `CREATE INDEX IF NOT EXISTS` statement then finds it and skips the
/// blocking build. An adoptable index is deferred while any earlier bundled
/// migration is unapplied, including when its table does not exist yet; on a
/// populated database apply the migrations through the preceding version and
/// call this again before the index's own migration. A
/// pending index whose canonical statement cannot adopt a prebuilt one is only
/// reported in [`OnlineIndexPreparation::external_build_blocks`] when its name
/// is absent. A matching existing name returns [`OnlineIndexProblem::ExternalCannotAdopt`],
/// because canonical SQL would fail instead of adopting it. Foreign or active
/// names retain their respective refusal. Only
/// [`crate::migrate_after_idempotency_cutover`] builds it online.
///
/// The call holds the SQLx migration advisory lock while building, so no SQLx
/// migrator or Runledger entry point runs at the same time. It is idempotent:
/// a valid prebuilt index is kept, an `INVALID` leftover is dropped and
/// rebuilt, and a foreign index under a declared name is refused with
/// [`OnlineIndexProblem::DefinitionMismatch`]. An active manual build is
/// refused with [`OnlineIndexProblem::BuildInProgress`], including when its
/// hidden progress can only be correlated with a build lock on the same table.
/// INVALID removal first locks and revalidates the actual object in a short
/// transaction. It requires an immediate exclusive table lock and never queues
/// a blocking DROP: a refused attempt rolls back and is retried for up to five
/// seconds, after which a still-busy table returns native `55P03`. Each
/// metadata transaction sets 100 ms lock and one-second statement, transaction
/// (where available) and idle-transaction timeouts. Exceeding the transaction
/// or idle bound ends the session, releasing its table and migration locks;
/// the termination is returned with the cleanup failures. An `INVALID` index
/// removed after inspection is refused as
/// [`OnlineIndexProblem::ChangedDuringRepair`]. The long concurrent build
/// starts only after its commit. A lost commit acknowledgement returns
/// [`crate::SchemaCompatibilityError::OnlineIndexRepairCommitUnconfirmed`]
/// inside the execution cause; no subsequent build is dispatched. Rollback
/// failure retains both causes in [`crate::SchemaCompatibilityError::OnlineIndexRollback`].
/// Applied migrations are never touched here;
/// [`crate::ensure_schema_compatible_after_idempotency_cutover`]
/// reports their declared indexes. A recorded migration whose checksum differs
/// from the bundle or whose run failed is refused with
/// [`crate::SchemaCompatibilityError::Incompatible`] (`VersionMismatch` or
/// `Dirty`) before any build, exactly as the protected runner refuses it,
/// instead of being treated as pending. A newer incompatible version in
/// `runledger_migration_history` is likewise refused with `VersionMissing`.
///
/// Run external tooling only after this call returned `Ok`. A build that fails
/// or is cancelled can leave an `INVALID` index; rerunning this call drops and
/// rebuilds it while its migration is still pending. Once the canonical
/// `CREATE INDEX IF NOT EXISTS` has adopted an `INVALID` index and recorded the
/// migration, this call no longer touches it and
/// [`crate::ensure_schema_compatible_after_idempotency_cutover`] refuses
/// startup with [`OnlineIndexProblem::Invalid`]. Recover by dropping that
/// index with `DROP INDEX CONCURRENTLY` and rebuilding it with the migration
/// file's statement plus `CONCURRENTLY`. Only where every other pending
/// bundled migration may also be applied now, running
/// [`crate::migrate_after_idempotency_cutover`] instead drops and rebuilds the
/// index itself, because it applies every pending migration.
///
/// Each concurrent build runs with the pool profile's statement, lock and
/// transaction timeouts lifted on the preparation session, because it waits
/// for in-flight writers and older snapshots; a long transaction therefore
/// delays the build instead of cancelling it and leaving an `INVALID` index
/// that a rerun must drop and replace. The saved values are restored after
/// each build, so later statements on that session keep the profile's bounds.
/// While a build waits, the call has no library-owned time bound; bound it
/// with the caller's own deadline or cancellation.
/// The build also enables client disconnect checks with an interval no longer
/// than one second, restoring the original value afterward. Socket closure
/// can then interrupt a blocked build; this is not a backend-death guarantee
/// across network failure or runtime death. Execution errors retain their
/// index and phase in [`crate::SchemaCompatibilityError::OnlineIndexExecution`],
/// with no recorded version for this preparation path. If lock release also
/// fails, [`crate::SchemaCompatibilityError::MigrationCleanup`] retains both
/// the original operation error and the unlock cause with redacted formatting.
///
/// Runledger waits for the migration lock by polling `pg_try_advisory_lock`,
/// so a waiting Runledger starter holds no snapshot that an in-progress
/// concurrent build must wait for. The wait keeps the bound the blocking
/// acquisition had: the smallest nonzero of the session's `statement_timeout`,
/// `lock_timeout` and, on servers that define it, `transaction_timeout` fails
/// it with [`crate::SchemaCompatibilityError::MigrationLockTimeout`], retaining
/// its typed setting and bound; only when all are zero is the wait unbounded. A
/// pending poll response is bounded locally as well; a successful response
/// observed after expiry cannot grant admission. This does not prove that
/// the remote session has already ended.
/// A migrator or older binary that waits with
/// blocking `pg_advisory_lock` does hold one, and PostgreSQL then aborts either
/// that waiter or the build with a deadlock. Do not start such tooling until
/// this call has returned, and do not call this while one is already waiting.
///
/// ```rust,no_run
/// # async fn demo(database: &runledger_postgres::RunledgerDatabase) -> Result<(), Box<dyn std::error::Error>> {
/// let report = runledger_postgres::prepare_online_indexes_for_external_migrator(database).await?;
/// if !report.deferred().is_empty() {
///     // Stop here on a populated database. Apply only the prerequisite prefix,
///     // then prepare again before applying the index's own migration.
///     return Err("indexes deferred: prerequisite migration prefix required".into());
/// }
/// if !report.external_build_blocks().is_empty() {
///     // Use the protected runner only if every pending migration is approved,
///     // or obtain application approval for blocking builds in a rollout window.
///     return Err("external tooling would still perform blocking index builds".into());
/// }
/// // Both lists empty means no index-preparation blockers were reported.
/// // Select external migrations only after their separate rollout gates are met,
/// // including draining old writers before a staged contract migration.
/// # Ok(())
/// # }
/// ```
pub async fn prepare_online_indexes_for_external_migrator(
    database: &RunledgerDatabase,
) -> Result<OnlineIndexPreparation, SchemaCompatibilityError> {
    let mut conn = database.pool().acquire().await?;
    // The advisory lock is session-scoped; never return a possibly locked
    // session to the pool if this future is cancelled. The wait polls without
    // a snapshot so a concurrent build on the lock holder cannot deadlock here.
    let lock = if MIGRATOR.locking {
        conn.close_on_drop();
        Some(MigrationLock::acquire(&mut conn).await?)
    } else {
        None
    };
    let result = prepare_adoptable_indexes(&mut conn, database).await;
    MigrationLock::release_after(lock, &mut conn, result).await
}

#[cfg(test)]
mod tests;
