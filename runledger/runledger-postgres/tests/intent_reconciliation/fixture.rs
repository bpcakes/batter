use batter_sqlx::verification::{
    DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
};
use runledger_postgres::grants::{RunledgerOperation, grant_fragment};
use runledger_postgres::jobs::JobScope;
use runledger_postgres::{
    DbPool, IntentReconciliation, PgSessionProfile, RetainedIntent, RunledgerDatabase, run_atomic,
};
use runledger_test_support::{EphemeralDatabase, setup_ephemeral_pool, teardown_ephemeral_pool};
use sqlx::{AssertSqlSafe, types::Uuid};
use std::time::Duration;

pub const TYPE: &str = "test.intent.correlation";
pub struct Fixture {
    pub pool: DbPool,
    pub service: RunledgerDatabase,
    pub role: String,
    database: EphemeralDatabase,
}
impl Fixture {
    pub async fn new(name: &str) -> Self {
        let (pool, database) = setup_ephemeral_pool(name, 5).await;
        let version: String = sqlx::query_scalar("SHOW server_version")
            .fetch_one(&pool)
            .await
            .expect("complete fixture database operation");
        eprintln!("intent reconciliation PostgreSQL {version}");
        crate::support::register_test_job_definition(&pool, TYPE).await;
        let role = format!("correlation_{}", Uuid::now_v7().simple());
        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN PASSWORD 'disposable-fixture-only'"
        )))
        .execute(&pool)
        .await
        .expect("complete fixture database operation");
        let schema = Identifier::new("public").expect("schema identifier");
        let policy = FragmentObjectPolicy::new(schema.clone());
        let sql = ExactRoleManifest::new(schema, DiscoveryScope::Declared)
            .expect("construct validated fixture input")
            .with_fragment(
                grant_fragment(
                    &policy,
                    [
                        RunledgerOperation::IntentReconciliation,
                        RunledgerOperation::IntentSubmission,
                    ],
                )
                .expect("construct validated fixture input"),
            )
            .expect("construct validated fixture input")
            .compile()
            .expect("compile grant manifest")
            .grant_plan()
            .render(&Identifier::new(&role).expect("role identifier"), None)
            .expect("render role grants");
        sqlx::raw_sql(AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .expect("complete fixture database operation");
        let options = (*pool.connect_options())
            .clone()
            .username(&role)
            .password("disposable-fixture-only");
        let profile = PgSessionProfile::new(
            &role,
            &role,
            vec!["public".into()],
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .expect("construct validated fixture input");
        let service = runledger_postgres::low_level::connect_database(options, profile, 3)
            .await
            .expect("complete fixture database operation");
        Self {
            pool,
            service,
            role,
            database,
        }
    }
    pub async fn restrict_to_reconciliation(&self) {
        sqlx::raw_sql(AssertSqlSafe(format!("DROP OWNED BY {}", self.role)))
            .execute(&self.pool)
            .await
            .expect("complete fixture database operation");
        let schema = Identifier::new("public").expect("schema identifier");
        let policy = FragmentObjectPolicy::new(schema.clone());
        let grants = ExactRoleManifest::new(schema, DiscoveryScope::Declared)
            .expect("construct validated fixture input")
            .with_fragment(
                grant_fragment(&policy, [RunledgerOperation::IntentReconciliation])
                    .expect("construct validated fixture input"),
            )
            .expect("construct validated fixture input")
            .compile()
            .expect("compile grant manifest")
            .grant_plan()
            .render(&Identifier::new(&self.role).expect("role identifier"), None)
            .expect("render role grants");
        sqlx::raw_sql(AssertSqlSafe(grants))
            .execute(&self.pool)
            .await
            .expect("complete fixture database operation");
    }
    pub async fn finish(self) {
        self.service.pool().close().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP OWNED BY {}; DROP ROLE {}",
            self.role, self.role
        )))
        .execute(&self.pool)
        .await
        .expect("complete fixture database operation");
        teardown_ephemeral_pool(self.pool, self.database).await;
    }
}
pub fn retained(evidence: IntentReconciliation) -> RetainedIntent {
    match evidence {
        IntentReconciliation::Retained(found) => found,
        IntentReconciliation::NotFound => panic!("expected retained intent"),
    }
}
pub async fn read(db: &RunledgerDatabase, scope: JobScope, id: Uuid) -> IntentReconciliation {
    run_atomic(db, async |mut tx| {
        tx.reconcile_job_enqueue_intent(scope, id).await
    })
    .await
    .expect("complete fixture database operation")
}
