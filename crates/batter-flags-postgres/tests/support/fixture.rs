#![allow(dead_code, reason = "shared by independently compiled test targets")]
use batter_flags_core::{FlagKey, Version};
use batter_flags_postgres::*;
use postgres_test_harness::{DatabaseLease, HarnessConfig, PostgresHarness};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::time::Duration;
use tokio::time::Instant;

pub fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
pub fn timing() -> StoreTiming {
    StoreTiming::new(
        Duration::from_secs(3),
        Duration::from_secs(3),
        Duration::from_secs(2),
        Duration::from_secs(3),
    )
    .unwrap()
}
pub fn change(v: Option<u64>) -> Change {
    Change::new(
        Actor::new("test-actor").unwrap(),
        ChangeReason::new("test reason").unwrap(),
        v.map_or(Precondition::Unconditional, |v| {
            Precondition::UnchangedSince(Version::new(v))
        }),
    )
}
pub fn key(value: &str) -> FlagKey {
    FlagKey::new(value).unwrap()
}
pub fn set(name: &str, variant: &str, v: Option<u64>) -> Write {
    Write::set(key(name), Variant::new(variant).unwrap(), change(v))
}
pub struct Fixture {
    pub pool: PgPool,
    pub store: PooledStore,
    pub lease: DatabaseLease,
    harness: PostgresHarness,
}
impl Fixture {
    pub async fn new() -> Self {
        let url = std::env::var("BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL")
            .expect("explicit disposable PostgreSQL 18 URL required");
        let config = HarnessConfig::new("batter_flags")
            .unwrap()
            .with_admin_database_url(url)
            .with_connections_per_database(16)
            .unwrap()
            .with_connection_budget(32)
            .unwrap();
        let harness = PostgresHarness::start(config).await.unwrap();
        let lease = harness.empty_database().await.unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(lease.database_url())
            .await
            .unwrap();
        let version: i32 =
            sqlx::query_scalar("SELECT current_setting('server_version_num')::integer")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(version >= 180000);
        let store = PooledStore::new(pool.clone(), timing());
        Self {
            pool,
            store,
            lease,
            harness,
        }
    }
    pub async fn migrated() -> Self {
        let fixture = Self::new().await;
        fixture.store.migrate(deadline()).await.unwrap();
        fixture
    }
    pub async fn close(self) {
        self.pool.close().await;
        self.lease.cleanup().await.unwrap();
        self.harness.shutdown().await.unwrap();
    }
    pub async fn seed(&self, name: &str, kind: &str, value_type: &str, presence: &str) {
        let digest = name.as_bytes();
        sqlx::query("INSERT INTO flags.flag_catalogs(manifest_digest,catalog_digest,manifest) VALUES ($1,$1,'{}') ON CONFLICT DO NOTHING")
            .bind(digest).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO flags.flag_catalog_entries(manifest_digest,key,kind,value_type,fingerprint,variants,default_variant,flip_discipline) VALUES ($1,$2,$3,$4,$1,ARRAY['off','on','express'],'off','independent') ON CONFLICT DO NOTHING")
            .bind(digest).bind(name).bind(kind).bind(value_type).execute(&self.pool).await.unwrap();
        if presence == "orphan" {
            return;
        }
        sqlx::query("INSERT INTO flags.flag_processes(process_id,application,environment,build,manifest_digest,snapshot_source,snapshot_version,declared_revision,declared_shared_revision,held_keys,held_overflow,floor_engaged_keys,floor_overflow,liveness_seconds,retention_seconds,expires_at,stopped_at) VALUES (uuidv7(),'service','test',$2,$1,'control_plane',0,$1,$1,'{}',0,'{}',0,60,3600,clock_timestamp() + CASE WHEN $2 = 'live' THEN interval '1 hour' ELSE interval '-1 hour' END,CASE WHEN $2 = 'stopped' THEN clock_timestamp() ELSE NULL END)")
            .bind(digest).bind(presence).execute(&self.pool).await.unwrap();
    }
    pub async fn head(&self) -> i64 {
        sqlx::query_scalar("SELECT version FROM flags.flag_control")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    pub async fn writer(&self, role: &str) -> PgPool {
        // Identifiers come only from local fixture constants, never environment or requests.
        assert!(role.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
        let sql = format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER; GRANT USAGE ON SCHEMA flags TO {role}; GRANT SELECT ON ALL TABLES IN SCHEMA flags TO {role}; GRANT UPDATE(version) ON flags.flag_control TO {role}; GRANT INSERT, UPDATE ON flags.flag_state TO {role};"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&self.pool)
            .await
            .unwrap();
        let options: PgConnectOptions = self.lease.database_url().parse().unwrap();
        PgPoolOptions::new()
            .max_connections(3)
            .connect_with(options.username(role))
            .await
            .unwrap()
    }
    pub async fn drop_role(&self, role: &str) {
        assert!(role.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP OWNED BY {role}; DROP ROLE {role}"
        )))
        .execute(&self.pool)
        .await
        .unwrap();
    }
}
