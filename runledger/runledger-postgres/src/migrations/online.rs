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
use std::fmt;

use sqlx::migrate::{Migrate, Migration};
use sqlx::{AssertSqlSafe, PgConnection, SqlSafeStr};

use super::{
    MIGRATOR, SchemaCompatibilityError, applied_runledger_migrations,
    expected_runledger_migrations, first_conflicting_runledger_version,
    first_dirty_runledger_version, first_missing_runledger_version, has_migrations_table,
    has_runledger_migration_history_table, list_migration_history,
    list_recorded_runledger_migrations, validate_checksum,
};
use crate::{PgQueryExecutor, RunledgerDatabase};

mod lock;
pub(super) use lock::MigrationLock;

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
    },
    OnlineIndex {
        migration_version: 202609050001,
        table: "job_queue",
        name: "idx_job_queue_scope_created_id",
        canonical_statement: "CREATE INDEX idx_job_queue_scope_created_id
    ON job_queue (organization_id, created_at DESC, id DESC);",
        expected_definition: "btree (organization_id, created_at DESC, id DESC)",
    },
    OnlineIndex {
        migration_version: 202609050001,
        table: "job_queue",
        name: "idx_job_queue_created_id",
        canonical_statement: "CREATE INDEX idx_job_queue_created_id
    ON job_queue (created_at DESC, id DESC);",
        expected_definition: "btree (created_at DESC, id DESC)",
    },
];

/// One reason a declared online index does not match its bundled migration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OnlineIndexProblem {
    /// The migration is recorded as applied but the index does not exist.
    Missing,
    /// The index exists but PostgreSQL marks it `INVALID`, typically after a
    /// failed or interrupted concurrent build.
    Invalid,
    /// A relation with the declared name exists that is not the declared
    /// index: a different definition, an index on another table, or another
    /// kind of relation.
    DefinitionMismatch,
    /// The bundled migration SQL does not contain the declared statement
    /// exactly once. This is a packaging defect, not a database state.
    StatementNotEmbedded,
    /// Another session is still building the index concurrently: its build
    /// shows in `pg_stat_progress_create_index`, or the index is missing or
    /// `INVALID` while another session holds the migration lock. The state is
    /// transient; retry after that build ends.
    BuildInProgress,
}

impl fmt::Display for OnlineIndexProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "is missing although its migration is recorded as applied",
            Self::Invalid => {
                "is INVALID after an interrupted concurrent build; the protected migration path rebuilds it"
            }
            Self::DefinitionMismatch => {
                "names a relation that is not the bundled index definition on its declared table"
            }
            Self::StatementNotEmbedded => {
                "declares a statement the bundled migration SQL does not contain exactly once"
            }
            Self::BuildInProgress => {
                "is still being built concurrently by another session; retry after that build ends"
            }
        })
    }
}

/// Outcome of [`prepare_online_indexes_for_external_migrator`].
#[derive(Debug, Default)]
pub struct OnlineIndexPreparation {
    built: Vec<&'static str>,
    already_valid: Vec<&'static str>,
    deferred: Vec<&'static str>,
    external_build_blocks: Vec<&'static str>,
}

impl OnlineIndexPreparation {
    /// Indexes this call built with `CREATE INDEX CONCURRENTLY`.
    #[must_use]
    pub fn built(&self) -> &[&'static str] {
        &self.built
    }

    /// Indexes that already existed, valid and with the bundled definition.
    #[must_use]
    pub fn already_valid(&self) -> &[&'static str] {
        &self.already_valid
    }

    /// Pending indexes not built by this call: an adoptable index whose
    /// migration cannot run yet because an earlier bundled migration is
    /// unapplied, or any index whose table does not exist. On a fresh database
    /// the pending migrations build these on empty tables. On a populated
    /// database, apply the migrations through the version preceding the
    /// adoptable index's own migration, then call
    /// [`crate::prepare_online_indexes_for_external_migrator`] again; otherwise
    /// the external migrator builds the index with a blocking statement.
    #[must_use]
    pub fn deferred(&self) -> &[&'static str] {
        &self.deferred
    }

    /// Pending indexes on an existing table whose canonical statement has no
    /// `IF NOT EXISTS`, so an external migrator builds them with a blocking
    /// `CREATE INDEX`. Only [`crate::migrate_after_idempotency_cutover`] builds
    /// them online.
    #[must_use]
    pub fn external_build_blocks(&self) -> &[&'static str] {
        &self.external_build_blocks
    }
}

enum IndexState {
    Missing,
    Invalid,
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

fn problem(index: &OnlineIndex, problem: OnlineIndexProblem) -> SchemaCompatibilityError {
    SchemaCompatibilityError::OnlineIndex {
        index: index.name,
        problem,
    }
}

fn quote_identifier(identifier: &str) -> String {
    let mut chars = identifier.chars();
    let plain = chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_');
    if plain {
        identifier.to_owned()
    } else {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}

/// Compare a live `pg_get_indexdef` rendering with the declared build. The
/// table is matched by name because the catalog query already pins its schema;
/// a type rendered with the authoritative schema prefix is normalized away.
fn definition_matches(index: &OnlineIndex, definition: &str, schema: &str) -> bool {
    let Some((head, tail)) = definition.split_once(" USING ") else {
        return false;
    };
    let schema_prefix = format!("{}.", quote_identifier(schema));
    head.starts_with(&format!("CREATE INDEX {} ON ", index.name))
        && head.ends_with(&format!(".{}", index.table))
        && tail.replace(&schema_prefix, "") == index.expected_definition
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
    // migration is recorded instead of failing the later build. Progress rows
    // of other roles hide their index, so the migration lock is the second,
    // role-independent sign of a build in progress.
    let row: Option<DeclaredNameRow> = sqlx::query_as(
        "SELECT i.indisvalid AS valid,
                    EXISTS (
                        SELECT 1 FROM pg_catalog.pg_stat_progress_create_index p
                        WHERE p.index_relid = c.oid
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
            valid: Some(valid),
            building,
            table: Some(table),
            definition: Some(definition),
        }) if table == index.table => {
            if valid {
                IndexState::Valid(definition)
            } else if !definition_matches(index, &definition, database.schema()) {
                // An INVALID leftover of someone else's build is refused like
                // any other foreign relation, never dropped.
                IndexState::Foreign
            } else if building || MigrationLock::held_by_another_session(conn).await? {
                IndexState::Building
            } else {
                IndexState::Invalid
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

/// The session's profile timeouts, saved while a concurrent build runs
/// without them and restored afterwards so whatever follows on that session
/// runs under the profile again: the cutover validation in the protected
/// runner, or the next build in external preparation.
///
/// A concurrent build waits for in-flight writers and older snapshots, so a
/// profile timeout would cancel it and leave an `INVALID` index behind. In
/// the protected runner the migration is already recorded by then, so only a
/// later startup could replace that index.
struct ProfileTimeouts {
    statement: String,
    lock: String,
    transaction: Option<String>,
}

impl ProfileTimeouts {
    async fn lift(conn: &mut PgConnection) -> Result<Self, sqlx::Error> {
        // transaction_timeout exists from PostgreSQL 17; read and lift it only
        // where the server knows it so an older server is not rejected after
        // the migration was recorded.
        let (statement, lock, transaction): (String, String, Option<String>) = sqlx::query_as(
            "SELECT pg_catalog.current_setting('statement_timeout'),
                    pg_catalog.current_setting('lock_timeout'),
                    pg_catalog.current_setting('transaction_timeout', true)",
        )
        .fetch_one(&mut *conn)
        .await?;
        sqlx::raw_sql(
            "SELECT pg_catalog.set_config('statement_timeout', '0', false),
                    pg_catalog.set_config('lock_timeout', '0', false),
                    CASE WHEN pg_catalog.current_setting('transaction_timeout', true) IS NOT NULL
                         THEN pg_catalog.set_config('transaction_timeout', '0', false)
                    END",
        )
        .execute(&mut *conn)
        .await?;
        Ok(Self {
            statement,
            lock,
            transaction,
        })
    }

    async fn restore(self, conn: &mut PgConnection) -> Result<(), sqlx::Error> {
        sqlx::query(
            "SELECT pg_catalog.set_config('statement_timeout', $1, false),
                    pg_catalog.set_config('lock_timeout', $2, false),
                    CASE WHEN $3::text IS NOT NULL
                         THEN pg_catalog.set_config('transaction_timeout', $3, false)
                    END",
        )
        .bind(self.statement)
        .bind(self.lock)
        .bind(self.transaction)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }
}

/// Drop an `INVALID` leftover when asked, then build the declared index
/// concurrently and check the result.
async fn build_concurrently(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
    drop_invalid: bool,
) -> Result<BuildOutcome, SchemaCompatibilityError> {
    if drop_invalid {
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP INDEX CONCURRENTLY IF EXISTS {}",
            database.relation(index.name)
        )))
        .execute(&mut *conn)
        .await?;
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
        IndexState::Invalid => Err(problem(index, OnlineIndexProblem::Invalid)),
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
) -> Result<BuildOutcome, SchemaCompatibilityError> {
    let drop_invalid = match index_state(conn, database, index).await? {
        IndexState::Valid(definition) => {
            return check_valid(index, &definition, database.schema())
                .map(|()| BuildOutcome::AlreadyValid);
        }
        IndexState::Building => return Err(problem(index, OnlineIndexProblem::BuildInProgress)),
        IndexState::Foreign => {
            return Err(problem(index, OnlineIndexProblem::DefinitionMismatch));
        }
        IndexState::Invalid => true,
        IndexState::Missing => false,
    };
    let timeouts = ProfileTimeouts::lift(conn).await?;
    let built = build_concurrently(conn, database, index, drop_invalid).await;
    let restored = timeouts.restore(conn).await;
    match (built, restored) {
        (Err(error), _) => Err(error),
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(_), Err(error)) => Err(error.into()),
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
            IndexState::Invalid | IndexState::Missing => {}
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
        ensure_index_built(conn, database, index).await?;
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
    let history = list_migration_history(conn, database).await?;

    if let Some(version) = first_conflicting_runledger_version(&history, &expected_migrations) {
        return Err(sqlx::migrate::MigrateError::VersionMismatch(version).into());
    }

    if let Some(version) = first_dirty_runledger_version(&history, &expected_migrations) {
        return Err(sqlx::migrate::MigrateError::Dirty(version).into());
    }

    if has_runledger_migration_history_table(conn, database).await? {
        let recorded_versions = list_recorded_runledger_migrations(conn, database).await?;
        if let Some(version) =
            first_missing_runledger_version(&recorded_versions, &expected_migrations)
        {
            return Err(sqlx::migrate::MigrateError::VersionMissing(version).into());
        }
    }

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
        ensure_index_built(conn, database, index).await?;
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
            IndexState::Invalid => return Err(problem(index, OnlineIndexProblem::Invalid)),
            IndexState::Missing => return Err(problem(index, OnlineIndexProblem::Missing)),
        }
    }
    Ok(())
}

/// The applied bundled versions, after the same history integrity checks the
/// protected runner makes: a recorded row with a foreign checksum or a failed
/// run is refused here, never treated as pending and built under.
async fn applied_versions_checked(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    expected: &super::RunledgerMigrationMap,
) -> Result<HashSet<i64>, SchemaCompatibilityError> {
    if !has_migrations_table(conn, database).await? {
        return Ok(HashSet::new());
    }
    let history = list_migration_history(conn, database).await?;
    if let Some(version) = first_conflicting_runledger_version(&history, expected) {
        return Err(sqlx::migrate::MigrateError::VersionMismatch(version).into());
    }
    if let Some(version) = first_dirty_runledger_version(&history, expected) {
        return Err(sqlx::migrate::MigrateError::Dirty(version).into());
    }
    Ok(applied_runledger_migrations(&history, expected)
        .into_iter()
        .map(|migration| migration.version)
        .collect())
}

async fn prepare_adoptable_indexes(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
) -> Result<OnlineIndexPreparation, SchemaCompatibilityError> {
    let expected = expected_runledger_migrations();
    let applied = applied_versions_checked(conn, database, &expected).await?;
    let mut report = OnlineIndexPreparation::default();
    for index in &ONLINE_INDEXES {
        if applied.contains(&index.migration_version) {
            continue;
        }
        let table_exists = table_exists(conn, database, index.table).await?;
        if !index.adoptable_by_canonical_sql() {
            // External tooling builds this one with a blocking statement
            // whenever its table already exists, however many earlier
            // migrations are still pending.
            if table_exists {
                report.external_build_blocks.push(index.name);
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
        match ensure_index_built(conn, database, index).await? {
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
/// reported in [`OnlineIndexPreparation::external_build_blocks`]; the external
/// migrator builds it with a blocking `CREATE INDEX`, and only
/// [`crate::migrate_after_idempotency_cutover`] builds it online.
///
/// The call holds the SQLx migration advisory lock while building, so no SQLx
/// migrator or Runledger entry point runs at the same time. It is idempotent:
/// a valid prebuilt index is kept, an `INVALID` leftover is dropped and
/// rebuilt, and a foreign index under a declared name is refused with
/// [`OnlineIndexProblem::DefinitionMismatch`]. Applied migrations are never
/// touched here; [`crate::ensure_schema_compatible_after_idempotency_cutover`]
/// reports their declared indexes. A recorded migration whose checksum differs
/// from the bundle or whose run failed is refused with
/// [`crate::SchemaCompatibilityError::Incompatible`] (`VersionMismatch` or
/// `Dirty`) before any build, exactly as the protected runner refuses it,
/// instead of being treated as pending.
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
///
/// Runledger waits for the migration lock by polling `pg_try_advisory_lock`,
/// so a waiting Runledger starter holds no snapshot that an in-progress
/// concurrent build must wait for. The wait keeps the bound the blocking
/// acquisition had: the smallest nonzero of the session's `statement_timeout`,
/// `lock_timeout` and, on servers that define it, `transaction_timeout` fails
/// it with [`crate::SchemaCompatibilityError::Incompatible`], and only when
/// all of them are zero does it wait without bound; the expiry is a
/// `sqlx::Error::Protocol` naming the setting rather than a `55P03` or
/// `57014` database error or a terminated connection. A
/// migrator or older binary that waits with
/// blocking `pg_advisory_lock` does hold one, and PostgreSQL then aborts either
/// that waiter or the build with a deadlock. Do not start such tooling until
/// this call has returned, and do not call this while one is already waiting.
///
/// ```rust,no_run
/// # async fn demo(database: &runledger_postgres::RunledgerDatabase) -> Result<(), Box<dyn std::error::Error>> {
/// let report = runledger_postgres::prepare_online_indexes_for_external_migrator(database).await?;
/// if !report.external_build_blocks().is_empty() {
///     // These pending indexes still block queue writes under external tooling.
/// }
/// // Now run the external migrator over the canonical migration files.
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
        Some(MigrationLock::acquire(&mut conn).await.map_err(|error| {
            SchemaCompatibilityError::Incompatible(sqlx::migrate::MigrateError::Execute(error))
        })?)
    } else {
        None
    };
    let result = prepare_adoptable_indexes(&mut conn, database).await;
    let unlock_result = match lock {
        Some(lock) => lock
            .release(&mut conn)
            .await
            .map_err(sqlx::migrate::MigrateError::Execute),
        None => Ok(()),
    };
    match (result, unlock_result) {
        (Err(error), Err(unlock_error)) => {
            tracing::error!(
                error = %unlock_error,
                "failed to unlock migration lock after online index preparation failure"
            );
            Err(error)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(SchemaCompatibilityError::MigrationUnlock(error)),
        (Ok(report), Ok(())) => Ok(report),
    }
}

#[cfg(test)]
mod tests;
