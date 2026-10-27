//! A concurrent DROP may commit after the verifier takes its catalog snapshot.

use std::{sync::Arc, time::Duration};

use runledger_postgres::{
    OnlineIndexProblem, PgSessionProfile, RunledgerDatabase, SchemaCompatibilityError,
    ensure_schema_compatible_after_idempotency_cutover,
};
use sqlx::migrate::Migrate;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use tokio::task::JoinHandle;

mod online_support;
mod support;
use online_support::{Harness, hold_writer_lock, sqlx_history, wait_for};
use support::{migrate_profiled, verify_profiled};

const INDEX: &str = "idx_job_queue_scope_created_id";
const GATE_CLASS: i32 = 726_341;
const DROP_GATE: i32 = 1;
const VERIFIER_GATE: i32 = 2;
const COMPLETION_GATE: i32 = 3;

struct Fixture {
    harness: Harness,
    verifier: Arc<RunledgerDatabase>,
    role: String,
    gates: PgConnection,
    invalid_oid: i64,
}

impl Fixture {
    async fn fresh() -> Self {
        let harness = Harness::fresh("online_snapshot_drop").await;
        migrate_profiled(&harness.pool)
            .await
            .expect("canonical schema");
        let invalid_oid = interrupted_index(&harness.pool).await;
        let role = format!("snapshot_reader_{}", sqlx::types::Uuid::new_v4().simple());
        install_gates(&harness.pool, &role, invalid_oid).await;
        let options = harness.pool.connect_options();
        let profile = PgSessionProfile::new(
            options.get_username(),
            &role,
            vec!["public".into()],
            Duration::from_secs(20),
            Duration::from_secs(20),
        )
        .expect("restricted verifier profile");
        let verifier = RunledgerDatabase::connect((*options).clone(), profile, 1)
            .await
            .expect("public verifier database");
        let mut gates = PgConnection::connect(harness.database.url())
            .await
            .expect("gate connection");
        for gate in [DROP_GATE, VERIFIER_GATE, COMPLETION_GATE] {
            sqlx::query("SELECT pg_catalog.pg_advisory_lock($1, $2)")
                .bind(GATE_CLASS)
                .bind(gate)
                .execute(&mut gates)
                .await
                .expect("hold gate");
        }
        Self {
            harness,
            verifier: Arc::new(verifier),
            role,
            gates,
            invalid_oid,
        }
    }

    async fn release(&mut self, gate: i32) {
        assert!(
            sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_advisory_unlock($1, $2)")
                .bind(GATE_CLASS)
                .bind(gate)
                .fetch_one(&mut self.gates)
                .await
                .expect("release gate")
        );
    }

    async fn assert_drop_committed(&self, drop_pid: i32, cooperating: bool) {
        wait_for("DROP committed outside the verifier snapshot", || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_index WHERE indexrelid::bigint = $1)
                        AND pg_catalog.pg_get_indexdef($1::bigint::oid) IS NULL",
            )
            .bind(self.invalid_oid)
            .fetch_one(&self.harness.pool)
            .await
            .expect("independent live catalog observation")
        })
        .await;
        let migration_lock: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks
             WHERE locktype = 'advisory' AND objsubid = 1 AND granted AND pid = $1)",
        )
        .bind(drop_pid)
        .fetch_one(&self.harness.pool)
        .await
        .expect("cooperating dropper retains its SQLx migration lock");
        assert_eq!(migration_lock, cooperating);
    }

    async fn remove_gates(&self) {
        sqlx::raw_sql(
            "DROP EVENT TRIGGER pause_index_drop;
             DROP FUNCTION pause_index_drop();
             DROP POLICY pause_verifier ON _sqlx_migrations;
             ALTER TABLE _sqlx_migrations DISABLE ROW LEVEL SECURITY;
             DROP FUNCTION pause_verifier();",
        )
        .execute(&self.harness.pool)
        .await
        .expect("remove fixture synchronization");
    }

    async fn teardown(self) {
        self.verifier.pool().close().await;
        self.gates.close().await.expect("close gate connection");
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP OWNED BY {0}; DROP ROLE {0}",
            self.role
        )))
        .execute(&self.harness.pool)
        .await
        .expect("remove verifier role");
        self.harness.teardown().await;
    }
}

async fn interrupted_index(pool: &PgPool) -> i64 {
    sqlx::raw_sql("DROP INDEX idx_job_queue_scope_created_id")
        .execute(pool)
        .await
        .expect("remove canonical index");
    let writer = hold_writer_lock(pool).await;
    let mut builder = pool.acquire().await.expect("builder");
    builder.close_on_drop();
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bounded concurrent build");
    let error = sqlx::raw_sql(
        "CREATE INDEX CONCURRENTLY idx_job_queue_scope_created_id
         ON job_queue (organization_id, created_at DESC, id DESC)",
    )
    .execute(&mut *builder)
    .await
    .expect_err("interrupted canonical build");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("57014")
    );
    builder.close().await.expect("retire interrupted builder");
    writer.commit().await.expect("release writer");
    let (oid, valid): (i64, bool) = sqlx::query_as(
        "SELECT indexrelid::bigint, indisvalid FROM pg_catalog.pg_index
         WHERE indexrelid = pg_catalog.to_regclass($1)",
    )
    .bind(INDEX)
    .fetch_one(pool)
    .await
    .expect("real interrupted index");
    assert!(!valid);
    oid
}

async fn install_gates(pool: &PgPool, role: &str, invalid_oid: i64) {
    // sql_drop runs after concurrent DROP's waiter phases but before commit.
    // The RLS policy pauses the actual public verifier's history query after
    // its own table locks and snapshot setup; it does not replace that query.
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE ROLE {role};
         GRANT USAGE ON SCHEMA public TO {role};
         GRANT SELECT ON ALL TABLES IN SCHEMA public TO {role};
         CREATE FUNCTION pause_index_drop() RETURNS event_trigger LANGUAGE plpgsql AS $$
         BEGIN
             PERFORM pg_catalog.pg_advisory_xact_lock({GATE_CLASS}, {DROP_GATE});
         END $$;
         CREATE EVENT TRIGGER pause_index_drop ON sql_drop WHEN TAG IN ('DROP INDEX')
             EXECUTE FUNCTION pause_index_drop();
         CREATE FUNCTION pause_verifier() RETURNS boolean LANGUAGE plpgsql AS $$
         BEGIN
             IF current_setting('transaction_isolation') <> 'repeatable read'
                OR current_setting('transaction_read_only') <> 'on' THEN
                 RAISE EXCEPTION 'verifier must own a read-only repeatable-read snapshot';
             END IF;
             PERFORM pg_catalog.pg_advisory_xact_lock({GATE_CLASS}, {VERIFIER_GATE});
             IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_index
                            WHERE indexrelid = {invalid_oid} AND NOT indisvalid) THEN
                 RAISE EXCEPTION 'verifier snapshot must retain the interrupted index';
             END IF;
             RETURN true;
         END $$;
         ALTER TABLE _sqlx_migrations ENABLE ROW LEVEL SECURITY;
         CREATE POLICY pause_verifier ON _sqlx_migrations FOR SELECT TO {role}
             USING (pause_verifier());"
    )))
    .execute(pool)
    .await
    .expect("install independent synchronization gates");
}

async fn waiting_at_gate(pool: &PgPool, gate: i32) -> i32 {
    let query = "SELECT pid FROM pg_catalog.pg_locks
                 WHERE locktype = 'advisory' AND objsubid = 2 AND NOT granted
                   AND classid::bigint = $1 AND objid::bigint = $2
                   AND database = (SELECT oid FROM pg_catalog.pg_database
                                   WHERE datname = pg_catalog.current_database())";
    wait_for("session at synchronization gate", || async {
        sqlx::query_scalar::<_, i32>(query)
            .bind(i64::from(GATE_CLASS))
            .bind(i64::from(gate))
            .fetch_optional(pool)
            .await
            .expect("gate waiters")
            .is_some()
    })
    .await;
    sqlx::query_scalar(query)
        .bind(i64::from(GATE_CLASS))
        .bind(i64::from(gate))
        .fetch_one(pool)
        .await
        .expect("one waiter on the database-local gate")
}

fn start_drop(pool: PgPool, cooperating: bool) -> JoinHandle<Result<(), SchemaCompatibilityError>> {
    tokio::spawn(async move {
        let mut conn = pool.acquire().await?;
        conn.close_on_drop();
        sqlx::raw_sql("SET statement_timeout = '20s'")
            .execute(&mut *conn)
            .await?;
        if cooperating {
            // A concurrent manual drop may follow the migration-lock protocol.
            // Protected repair now removes INVALID indexes transactionally,
            // so its table lock prevents this older-snapshot interleaving.
            (*conn).lock().await?;
        }
        sqlx::raw_sql("DROP INDEX CONCURRENTLY idx_job_queue_scope_created_id")
            .execute(&mut *conn)
            .await?;
        if cooperating {
            sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1, $2)")
                .bind(GATE_CLASS)
                .bind(COMPLETION_GATE)
                .execute(&mut *conn)
                .await?;
            (*conn).unlock().await?;
        }
        Ok(())
    })
}

async fn assert_drop_commit_classification(cooperating: bool) {
    let mut fixture = Fixture::fresh().await;
    let history = sqlx_history(&fixture.harness.pool).await;
    let dropper = start_drop(fixture.harness.pool.clone(), cooperating);
    let drop_pid = waiting_at_gate(&fixture.harness.pool, DROP_GATE).await;
    let database = Arc::clone(&fixture.verifier);
    let verifier =
        tokio::spawn(
            async move { ensure_schema_compatible_after_idempotency_cutover(&database).await },
        );
    waiting_at_gate(&fixture.harness.pool, VERIFIER_GATE).await;
    fixture.release(DROP_GATE).await;
    fixture.assert_drop_committed(drop_pid, cooperating).await;
    fixture.release(VERIFIER_GATE).await;
    let result = tokio::time::timeout(Duration::from_secs(20), verifier)
        .await
        .expect("verifier completes")
        .expect("verifier task");
    fixture.release(COMPLETION_GATE).await;
    tokio::time::timeout(Duration::from_secs(20), dropper)
        .await
        .expect("drop or repair completes after verifier rollback")
        .expect("dropper task")
        .expect("drop or repair succeeds");
    fixture.remove_gates().await;
    assert_eq!(sqlx_history(&fixture.harness.pool).await, history);
    // A settled manual DROP is repaired by the next cooperating startup.
    migrate_profiled(&fixture.harness.pool)
        .await
        .expect("settled schema");
    verify_profiled(&fixture.harness.pool)
        .await
        .expect("fresh verification");
    fixture.teardown().await;
    let expected = if cooperating {
        OnlineIndexProblem::BuildInProgress
    } else {
        OnlineIndexProblem::Missing
    };
    let SchemaCompatibilityError::OnlineIndex { index, problem } =
        result.expect_err("the old snapshot cannot certify the removed index")
    else {
        panic!("unexpected verification result")
    };
    assert_eq!(index, INDEX);
    assert_eq!(problem, expected);
}

#[tokio::test]
async fn verifier_reports_cooperating_drop_after_it_commits_inside_its_snapshot() {
    assert_drop_commit_classification(true).await;
}

#[tokio::test]
async fn verifier_reports_missing_after_manual_drop_commits_inside_its_snapshot() {
    assert_drop_commit_classification(false).await;
}
