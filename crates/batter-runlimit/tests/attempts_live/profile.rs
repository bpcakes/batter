//! Role/schema regression: admission evidence is written by an invoker trigger.
use super::*;
use sqlx::postgres::PgPoolOptions;

#[path = "profile_cases.rs"]
mod cases;

struct ProfileFixture {
    base: Fixture,
    database: PgProfiledPool,
    migration: PgProfiledPool,
    role: String,
    schema: String,
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

impl ProfileFixture {
    async fn new() -> Self {
        let base = Fixture::new("profile", 5000).await;
        let suffix: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
            .fetch_one(&base.pool)
            .await
            .unwrap();
        let role = format!("attempt_{suffix}");
        let schema = format!("attempt \"{suffix}\"");
        let (r, s) = (quote(&role), quote(&schema));
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {r}; CREATE SCHEMA {s}; GRANT USAGE ON SCHEMA {s} TO {r}"
        )))
        .execute(&base.pool)
        .await
        .unwrap();
        let options = base.pool.connect_options();
        let login = options.get_username();
        let profile = |role: &str| {
            PgSessionProfile::new(
                login,
                role,
                vec![schema.clone()],
                Duration::from_secs(2),
                Duration::from_millis(250),
            )
            .unwrap()
            .with_setting("app.tenant", "expected")
            .unwrap()
        };
        let migration = PgProfiledPool::connect((*options).clone(), profile(login), 2)
            .await
            .unwrap();
        PostgresAttemptLimiter::new(migration.pool().clone())
            .migrate()
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE events (kind text, actor text DEFAULT current_user, tenant text DEFAULT current_setting('app.tenant'), schema_name text DEFAULT current_schema());
             ALTER TABLE events ENABLE ROW LEVEL SECURITY;
             CREATE POLICY serving ON events USING (tenant = current_setting('app.tenant')) WITH CHECK (tenant = 'expected');
             CREATE TABLE private_admin_data (secret text);
             INSERT INTO private_admin_data VALUES ('not readable by serving role');
             CREATE FUNCTION admission_observation() RETURNS trigger LANGUAGE plpgsql AS $body$
             BEGIN INSERT INTO {s}.events(kind) VALUES ('admission'); RETURN NEW; END $body$;
             CREATE TRIGGER observe_admission AFTER INSERT ON runlimit_attempts FOR EACH ROW EXECUTE FUNCTION admission_observation();
             GRANT SELECT, INSERT, UPDATE, DELETE ON runlimit_attempts, events TO {r};"
        ))).execute(migration.pool()).await.unwrap();
        let database = PgProfiledPool::connect((*options).clone(), profile(&role), 1)
            .await
            .unwrap();
        Self {
            base,
            database,
            migration,
            role,
            schema,
        }
    }

    fn subject(&self, label: &str) -> AttemptSubject<'_> {
        self.base.hasher.hash_attempt_for(&self.base.policy, label)
    }

    fn runner(&self) -> AttemptRunner {
        AttemptRunner::new(self.database.clone()).unwrap()
    }

    async fn events(&self) -> Vec<(String, String, String, String)> {
        sqlx::query_as("SELECT kind, actor, tenant, schema_name FROM events ORDER BY kind")
            .fetch_all(self.migration.pool())
            .await
            .unwrap()
    }

    async fn close(self) {
        self.database.pool().close().await;
        self.migration.pool().close().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE; DROP ROLE {}",
            quote(&self.schema),
            quote(&self.role)
        )))
        .execute(&self.base.pool)
        .await
        .unwrap();
        self.base.pool.close().await;
    }
}

#[tokio::test]
async fn fallback_schemas_are_rejected_before_execution() {
    let profile = PgSessionProfile::new(
        "login",
        "serving",
        vec!["authoritative".into(), "public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let database = PgProfiledPool::connect_lazy(
        "postgres://login@127.0.0.1:1/unused".parse().unwrap(),
        profile,
        PgPoolOptions::new().min_connections(0),
    )
    .unwrap();
    assert!(matches!(
        AttemptRunner::new(database.clone()),
        Err(sqlx::Error::Protocol(_))
    ));
    assert_eq!(database.pool().size(), 0);
    database.pool().close().await;
}
