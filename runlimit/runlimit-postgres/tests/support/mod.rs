//! The one disposable-schema fixture shared by every live `PostgreSQL` suite.
//!
//! Each suite is its own test binary and uses a different part of this module.
#![allow(
    dead_code,
    reason = "each live suite compiles this module and uses a different subset"
)]

use std::{
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use runlimit_core::{RateLimitPolicy, SubjectKey};
use sqlx::{AssertSqlSafe, PgPool, postgres::PgPoolOptions};

/// Names the disposable database every ignored live test requires.
pub const TEST_DATABASE_URL: &str = "RUNLIMIT_POSTGRES_TEST_DATABASE_URL";

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(0);

fn database_url() -> String {
    std::env::var(TEST_DATABASE_URL)
        .expect("RUNLIMIT_POSTGRES_TEST_DATABASE_URL is required for ignored integration tests")
}

/// Connects to the database's default schema.
pub async fn admin_pool(maximum_connections: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(maximum_connections)
        .connect(&database_url())
        .await
        .expect("connect to integration-test PostgreSQL")
}

/// A uniquely named schema, the pools bound to it, and the admin pool that
/// drops it.
///
/// Dynamic SQL here interpolates only fixture-generated schema identifiers.
/// Values continue to use bind parameters. `AssertSqlSafe` marks these audited
/// statements for `SQLx`.
pub struct IsolatedSchema {
    /// Connected to the default schema; creates and drops the isolated one.
    pub admin_pool: PgPool,
    /// Every connection resolves unqualified names in the isolated schema.
    pub pool: PgPool,
    pub schema: String,
    additional_pools: Vec<PgPool>,
}

impl IsolatedSchema {
    /// Creates an empty schema with its own two-connection admin pool.
    pub async fn create(label: &str, maximum_connections: u32) -> Self {
        Self::create_with_admin(admin_pool(2).await, label, maximum_connections).await
    }

    /// Creates an empty schema that `admin_pool` owns and later drops.
    pub async fn create_with_admin(
        admin_pool: PgPool,
        label: &str,
        maximum_connections: u32,
    ) -> Self {
        let schema = format!(
            "runlimit_{label}_{}_{}_{}",
            process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after the Unix epoch")
                .as_nanos(),
            NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
        );
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create isolated test schema");
        let pool = Self::schema_bound_pool(&schema, maximum_connections).await;

        Self {
            admin_pool,
            pool,
            schema,
            additional_pools: Vec::new(),
        }
    }

    async fn schema_bound_pool(schema: &str, maximum_connections: u32) -> PgPool {
        let connection_schema = schema.to_owned();
        let pool = PgPoolOptions::new()
            .max_connections(maximum_connections)
            .after_connect(move |connection, _metadata| {
                let search_path_sql = format!("SET search_path = {connection_schema}, pg_catalog");
                Box::pin(async move {
                    sqlx::query(AssertSqlSafe(search_path_sql))
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&database_url())
            .await
            .expect("connect isolated-schema pool");
        let search_path = sqlx::query_scalar::<_, Vec<String>>("SELECT current_schemas(false)")
            .fetch_one(&pool)
            .await
            .expect("read isolated-schema search path");
        assert_eq!(search_path, [schema, "pg_catalog"]);
        pool
    }

    /// Opens another pool bound to the same schema; teardown closes it.
    pub async fn additional_pool(&mut self, maximum_connections: u32) -> PgPool {
        let pool = Self::schema_bound_pool(&self.schema, maximum_connections).await;
        self.additional_pools.push(pool.clone());
        pool
    }

    /// Closes every schema-bound pool, drops the schema, then closes the admin
    /// pool.
    pub async fn teardown(self) {
        for pool in self.additional_pools {
            pool.close().await;
            assert!(
                pool.is_closed(),
                "additional pool must close before schema drop"
            );
        }
        self.pool.close().await;
        assert!(
            self.pool.is_closed(),
            "primary pool must close before schema drop"
        );
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin_pool)
        .await
        .expect("drop isolated test schema");
        self.admin_pool.close().await;
        assert!(self.admin_pool.is_closed(), "admin pool must close last");
    }
}

/// Builds a subject whose counter lands in `capacity_shard` under `policy`.
///
/// The persisted shard is the XOR of the leading fingerprint and subject
/// bytes. Every other subject byte is `fill`, which tells subjects in one
/// shard apart.
pub fn key_in_capacity_shard<P: RateLimitPolicy>(
    policy: &P,
    capacity_shard: u8,
    fill: u8,
) -> SubjectKey {
    let mut digest = [fill; 32];
    digest[0] = policy.fingerprint().as_bytes()[0] ^ capacity_shard;
    SubjectKey::from_digest(digest)
}

/// Returns the persisted capacity shard of `subject` under `policy`.
pub fn capacity_shard_for<P: RateLimitPolicy>(policy: &P, subject: SubjectKey) -> i16 {
    i16::from(policy.fingerprint().as_bytes()[0] ^ subject.as_bytes()[0])
}
