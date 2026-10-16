//! Disposable schema, logins and manifests for managed-object live cases.

use super::support::{self, Result, bounded};
use batter_core::operation::{OperationContext, OperationOwner};
use batter_sqlx::PgLease;
use batter_sqlx::managed::{
    AppliedManagedObjects, ManagedApplyError, ManagedObject, ManagedObjectManifest, PublicExecute,
    Retirement,
};
use batter_sqlx::verification::{
    Identifier, MigrationExpectation, QualifiedName, RoutineSignature, RoutineType, SqlxLedgerMode,
    SqlxMigrationManifest, VerificationPlan, VerificationReport, verify,
};
use sqlx::{
    Connection, PgConnection, PgPool, SqlSafeStr,
    migrate::{Migration, MigrationType, Migrator},
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fmt::Write as _,
    fs::File,
    io::Read,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Which provisioned login a pool authenticates as.
#[derive(Clone, Copy)]
pub(crate) enum Role {
    /// Owns the schema; runs migrations and applies managed objects.
    Migrator,
    /// Serving login: schema `USAGE` and `SELECT` on tables the migrator creates.
    Reader,
}

pub(crate) struct Fixture {
    admin: PgConnection,
    url: String,
    password: String,
    schema: String,
    migrator: String,
    reader: String,
    delegate: String,
    /// Migrations after the `records` table, as versions 2, 3, ...
    migrations: Vec<String>,
    pools: Vec<PgPool>,
}

/// A validated manifest for this fixture's schema.
pub(crate) struct Objects {
    pub(crate) manifest: ManagedObjectManifest,
}

impl Fixture {
    /// Provision, run `body`, and clean up even when the body fails.
    pub(crate) async fn run(body: impl AsyncFnOnce(&mut Self) -> Result) -> Result {
        let mut fixture = Self::create().await?;
        let result = body(&mut fixture).await;
        support::combine(result, fixture.cleanup().await)
    }

    async fn create() -> Result<Self> {
        let url = std::env::var("BATTER_SQLX_ADMIN_URL")
            .map_err(|_| std::io::Error::other("live checks require BATTER_SQLX_ADMIN_URL"))?;
        let mut admin = bounded(PgConnection::connect(&url)).await??;
        let superuser: bool = bounded(
            sqlx::query_scalar(
                "SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = session_user",
            )
            .fetch_one(&mut admin),
        )
        .await??;
        support::require(
            superuser,
            "BATTER_SQLX_ADMIN_URL must authenticate as a PostgreSQL superuser",
        )?;
        let suffix = format!(
            "{}_{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() % 1_000_000_000
        );
        let mut fixture = Self {
            admin,
            url,
            password: random_password()?,
            schema: format!("batter_mo_{suffix}"),
            migrator: format!("batter_mo_migrator_{suffix}"),
            reader: format!("batter_mo_reader_{suffix}"),
            delegate: format!("batter_mo_delegate_{suffix}"),
            migrations: Vec::new(),
            pools: Vec::new(),
        };
        let provisioned = fixture.provision().await;
        if let Err(error) = provisioned {
            return Err(support::combine(Err(error), fixture.cleanup().await)
                .expect_err("provisioning failed"));
        }
        Ok(fixture)
    }

    async fn provision(&mut self) -> Result {
        let (schema, migrator, reader) = (&self.schema, &self.migrator, &self.reader);
        let delegate = &self.delegate;
        let password = self.password.replace('\'', "''");
        for statement in [
            format!("CREATE ROLE {migrator} LOGIN PASSWORD '{password}'"),
            format!("CREATE ROLE {reader} LOGIN PASSWORD '{password}'"),
            format!("CREATE ROLE {delegate} NOLOGIN"),
            format!("CREATE SCHEMA {schema} AUTHORIZATION {migrator}"),
            format!("GRANT USAGE ON SCHEMA {schema} TO {reader}, {delegate}"),
            format!(
                "ALTER DEFAULT PRIVILEGES FOR ROLE {migrator} IN SCHEMA {schema}
                 GRANT SELECT ON TABLES TO {reader}"
            ),
        ] {
            bounded(sqlx::raw_sql(sqlx::AssertSqlSafe(statement)).execute(&mut self.admin))
                .await??;
        }
        Ok(())
    }

    async fn cleanup(mut self) -> Result {
        let mut result = Ok(());
        for pool in self.pools.drain(..) {
            result = support::combine(result, bounded(pool.close()).await);
        }
        let (schema, migrator, reader) = (&self.schema, &self.migrator, &self.reader);
        let delegate = &self.delegate;
        for statement in [
            format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
            format!("DROP OWNED BY {migrator}, {reader}, {delegate}"),
            format!("DROP ROLE IF EXISTS {migrator}"),
            format!("DROP ROLE IF EXISTS {reader}"),
            format!("DROP ROLE IF EXISTS {delegate}"),
        ] {
            let outcome =
                bounded(sqlx::raw_sql(sqlx::AssertSqlSafe(statement)).execute(&mut self.admin))
                    .await
                    .and_then(|result| result.map(|_| ()).map_err(Into::into));
            result = support::combine(result, outcome);
        }
        support::combine(result, self.admin.close().await.map_err(Into::into))
    }

    pub(crate) fn schema(&self) -> &str {
        &self.schema
    }

    /// A non-login role with schema `USAGE` that tests may give grant options.
    pub(crate) fn delegate(&self) -> &str {
        &self.delegate
    }

    /// A one-connection pool whose session search path is the fixture schema,
    /// so the SQLx ledger `_sqlx_migrations` lands there.
    pub(crate) async fn pool(&mut self, role: Role) -> Result<PgPool> {
        self.pool_with(role, &[]).await
    }

    /// [`Self::pool`] with additional session settings, such as another
    /// default transaction isolation.
    pub(crate) async fn pool_with(
        &mut self,
        role: Role,
        settings: &[(&str, &str)],
    ) -> Result<PgPool> {
        let login = match role {
            Role::Migrator => &self.migrator,
            Role::Reader => &self.reader,
        };
        let options = PgConnectOptions::from_str(&self.url)?
            .username(login)
            .password(&self.password)
            .options([("search_path", self.schema.as_str())])
            .options(settings.iter().copied());
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(10))
            .connect_lazy_with(options);
        self.pools.push(pool.clone());
        Ok(pool)
    }

    /// Execute administrative SQL as the superuser.
    pub(crate) async fn admin(&mut self, sql: &str) -> Result {
        bounded(sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned())).execute(&mut self.admin))
            .await??;
        Ok(())
    }

    /// Append a versioned migration that every later apply and SQLx
    /// manifest of this fixture includes.
    pub(crate) fn add_migration(&mut self, sql: String) {
        self.migrations.push(sql);
    }

    fn migrator_bundle(&self) -> Migrator {
        let records = format!(
            "CREATE TABLE {}.records (
                 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                 note text NOT NULL,
                 amount bigint NOT NULL DEFAULT 0,
                 updated_at timestamptz NOT NULL DEFAULT now())",
            self.schema
        );
        Migrator::with_migrations(
            std::iter::once(("records".to_owned(), records))
                .chain(
                    (2..)
                        .zip(&self.migrations)
                        .map(|(version, sql)| (format!("step_{version}"), sql.clone())),
                )
                .zip(1..)
                .map(|((description, sql), version)| {
                    Migration::new(
                        version,
                        description.into(),
                        MigrationType::Simple,
                        sqlx::AssertSqlSafe(sql).into_sql_str(),
                        false,
                    )
                })
                .collect(),
        )
    }

    /// The four standard entries: a trigger routine, a SQL routine reading the
    /// migrated table, a `BEGIN ATOMIC` procedure and the trigger binding the
    /// first routine.
    pub(crate) fn standard_objects(&self) -> Vec<ManagedObject> {
        let schema = &self.schema;
        let routine = |name: &str, arguments: Vec<RoutineType>| {
            RoutineSignature::new(schema, name, arguments).expect("valid signature")
        };
        vec![
            ManagedObject::function(
                routine("touch_updated_at", Vec::new()),
                format!(
                    "CREATE OR REPLACE FUNCTION {schema}.touch_updated_at() RETURNS trigger
                     LANGUAGE plpgsql AS $$ BEGIN NEW.updated_at := now(); RETURN NEW; END $$;"
                ),
            )
            .expect("valid function"),
            ManagedObject::function(
                routine(
                    "record_total",
                    vec![RoutineType::new("pg_catalog", "int8").expect("valid type")],
                ),
                format!(
                    "CREATE OR REPLACE FUNCTION {schema}.record_total(extra bigint) RETURNS bigint
                     LANGUAGE sql STABLE
                     AS $$ SELECT coalesce(sum(amount), 0)::bigint + extra FROM {schema}.records $$;"
                ),
            )
            .expect("valid function"),
            ManagedObject::procedure(
                routine("archive_records", Vec::new()),
                format!(
                    "CREATE OR REPLACE PROCEDURE {schema}.archive_records() LANGUAGE sql
                     BEGIN ATOMIC DELETE FROM {schema}.records WHERE amount < 0; END;"
                ),
            )
            .expect("valid procedure"),
            ManagedObject::trigger(
                QualifiedName::new(schema, "records").expect("valid table"),
                Identifier::new("records_touch").expect("valid trigger"),
                format!(
                    "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON {schema}.records
                     FOR EACH ROW EXECUTE FUNCTION {schema}.touch_updated_at();"
                ),
            )
            .expect("valid trigger"),
        ]
    }

    pub(crate) fn objects(&self, policy: PublicExecute) -> Objects {
        self.objects_with(policy, self.standard_objects(), [])
            .expect("standard manifest is valid")
    }

    pub(crate) fn objects_with(
        &self,
        policy: PublicExecute,
        objects: impl IntoIterator<Item = ManagedObject>,
        retirements: impl IntoIterator<Item = Retirement>,
    ) -> Result<Objects> {
        Ok(Objects {
            manifest: ManagedObjectManifest::new(
                QualifiedName::new(&self.schema, "managed_objects")?,
                policy,
                objects,
                retirements,
            )?,
        })
    }

    pub(crate) fn objects_in_ledger(&self, policy: PublicExecute, ledger: &str) -> Result<Objects> {
        Ok(Objects {
            manifest: ManagedObjectManifest::new(
                QualifiedName::new(&self.schema, ledger)?,
                policy,
                self.standard_objects(),
                [],
            )?,
        })
    }

    pub(crate) async fn apply(
        &self,
        pool: &PgPool,
        manifest: &ManagedObjectManifest,
    ) -> std::result::Result<AppliedManagedObjects, ManagedApplyError> {
        let migrator = self.migrator_bundle();
        let context = context();
        let lease = PgLease::acquire(pool, &context)
            .await
            .expect("live lease acquisition");
        tokio::time::timeout(
            Duration::from_secs(30),
            lease.migrate_and_apply(&migrator, manifest),
        )
        .await
        .expect("apply completed within thirty seconds")
    }

    /// Apply `manifest` while another transaction holds `ROW EXCLUSIVE` on
    /// `table`, which every lock a trigger statement takes conflicts with,
    /// and require the apply to finish within ten seconds. An unchanged set
    /// must take no lock on the table, so it never waits for that writer.
    pub(crate) async fn apply_beside_writer(
        &mut self,
        pool: &PgPool,
        manifest: &ManagedObjectManifest,
        table: &str,
    ) -> Result<AppliedManagedObjects> {
        let writers = self.pool(Role::Migrator).await?;
        let mut writer = writers.begin().await?;
        bounded(
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "LOCK TABLE {}.{table} IN ROW EXCLUSIVE MODE",
                self.schema
            )))
            .execute(&mut *writer),
        )
        .await??;
        let applied = tokio::time::timeout(Duration::from_secs(10), self.apply(pool, manifest))
            .await
            .map_err(|_| std::io::Error::other("an apply waited for a table a writer held"))?;
        writer.rollback().await?;
        Ok(applied?)
    }

    pub(crate) async fn migrate_only(&self, pool: &PgPool) -> Result {
        let lease = PgLease::acquire(pool, &context()).await?;
        lease.migrate(&self.migrator_bundle()).await?;
        Ok(())
    }

    pub(crate) fn sqlx_manifest(&self) -> Result<SqlxMigrationManifest> {
        Ok(SqlxMigrationManifest::new(
            QualifiedName::new(&self.schema, "_sqlx_migrations")?,
            SqlxLedgerMode::Exact,
            self.migrator_bundle()
                .iter()
                .filter(|migration| !migration.migration_type.is_down_migration())
                .map(|migration| {
                    MigrationExpectation::new(migration.version, migration.checksum.to_vec())
                }),
        )?)
    }

    pub(crate) async fn verify(
        &self,
        pool: &PgPool,
        plan: VerificationPlan<'_>,
    ) -> Result<VerificationReport> {
        Ok(verify(pool, &context(), plan).await?)
    }

    /// Ledger object rows, excluding the set row, with their applied times.
    pub(crate) async fn object_rows(
        &self,
        pool: &PgPool,
    ) -> Result<Vec<(String, String, Vec<u8>, Vec<u8>, String)>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT identity, kind, source_checksum, rendered_checksum, applied_at::text
             FROM {}.managed_objects WHERE identity <> 'manifest' ORDER BY identity",
            self.schema
        )))
        .fetch_all(pool)
        .await?)
    }

    pub(crate) async fn exists(&self, pool: &PgPool, routine: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
                            WHERE n.nspname = $1 AND p.proname = $2)",
        )
        .bind(&self.schema)
        .bind(routine)
        .fetch_one(pool)
        .await?)
    }

    pub(crate) async fn trigger_exists(&self, pool: &PgPool, trigger: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
                            JOIN pg_namespace n ON n.oid = c.relnamespace
                            WHERE n.nspname = $1 AND t.tgname = $2)",
        )
        .bind(&self.schema)
        .bind(trigger)
        .fetch_one(pool)
        .await?)
    }

    pub(crate) async fn relation_exists(&self, pool: &PgPool, relation: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                            WHERE n.nspname = $1 AND c.relname = $2)",
        )
        .bind(&self.schema)
        .bind(relation)
        .fetch_one(pool)
        .await?)
    }

    pub(crate) async fn public_can_execute(&self, pool: &PgPool, routine: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT pg_catalog.has_function_privilege('public', $1, 'EXECUTE')")
                .bind(format!("{}.{routine}", self.schema))
                .fetch_one(pool)
                .await?,
        )
    }
}

fn context() -> OperationContext {
    OperationOwner::new(Duration::from_secs(30))
        .expect("constant budget")
        .into_context()
}

fn random_password() -> Result<String> {
    let mut bytes = [0_u8; 24];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut password = String::from("batter_mo_pw_");
    for byte in bytes {
        write!(password, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(password)
}

/// Every routine and user trigger in `schema`, including `xmin`, owner, ACL and
/// rendered definition, so equality proves no catalog row was rewritten.
pub(crate) async fn catalog_state(pool: &PgPool, schema: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text || '|' || p.oid::text || '|' || p.xmin::text || '|'
                || p.proowner::text || '|' || coalesce(p.proacl::text, '') || '|'
                || pg_get_functiondef(p.oid)
         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = $1
         UNION ALL
         SELECT t.tgname::text || '|' || t.oid::text || '|' || t.xmin::text || '|'
                || pg_get_triggerdef(t.oid)
         FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND NOT t.tgisinternal
         ORDER BY 1",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?)
}

pub(crate) async fn pid(pool: &PgPool) -> Result<i32> {
    Ok(sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pool)
        .await?)
}

pub(crate) fn sqlstate(error: &ManagedApplyError) -> Option<String> {
    error
        .native()?
        .native()
        .as_database_error()?
        .code()
        .map(|code| code.into_owned())
}
