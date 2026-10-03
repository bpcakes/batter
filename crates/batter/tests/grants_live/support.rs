//! Disposable PostgreSQL 18 provisioning for the grant-composition suite.
//!
//! Every object this module creates is named with a per-process suffix and
//! dropped during cleanup. It provisions non-owner, non-superuser logins with
//! `NOINHERIT` and no role memberships, so a login reaches exactly the
//! privileges the rendered grant plan provisions plus PostgreSQL's PUBLIC
//! defaults, which the application policy declares explicitly.

use batter::sqlx::verification::CompiledExactRole;
use batter::sqlx::verification::Identifier;
use sqlx::{
    Connection, PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fmt::Write as _,
    fs::File,
    io::Read,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) fn fail(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.to_owned()))
}

pub(crate) fn require(condition: bool, message: &str) -> Result {
    if condition {
        Ok(())
    } else {
        Err(fail(message))
    }
}

pub(crate) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(crate) async fn exec(connection: &mut PgConnection, statement: String) -> Result {
    sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

/// Every schema and role this suite owns.
#[derive(Clone)]
pub(crate) struct Names {
    pub(crate) jobs: String,
    pub(crate) quotas: String,
    pub(crate) application: String,
    pub(crate) shadow: String,
    pub(crate) owner: String,
}

impl Names {
    fn new() -> Self {
        let suffix = format!(
            "{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after Unix epoch")
                .as_nanos()
                % 1_000_000_000,
        );
        Self {
            // Deliberately mixed case and never `public`, so quoting and the
            // session profile are exercised by every case.
            jobs: format!("BatterJobs_{suffix}"),
            quotas: format!("BatterQuotas_{suffix}"),
            application: format!("BatterApp_{suffix}"),
            shadow: format!("BatterShadow_{suffix}"),
            owner: format!("batter_grants_owner_{suffix}"),
        }
    }
}

pub(crate) struct Fixture {
    pub(crate) admin: PgConnection,
    pub(crate) names: Names,
    url: String,
    password: String,
    logins: Vec<String>,
    pools: Vec<PgPool>,
}

impl Fixture {
    pub(crate) async fn create() -> Result<Self> {
        let url = std::env::var("BATTER_SQLX_ADMIN_URL").map_err(|_| {
            fail("the grant-composition suite requires BATTER_SQLX_ADMIN_URL, a PostgreSQL superuser connection to a dedicated disposable PostgreSQL 18 cluster")
        })?;
        let mut admin = PgConnection::connect(&url).await?;
        let superuser: bool = sqlx::query_scalar(
            "SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = session_user",
        )
        .fetch_one(&mut admin)
        .await?;
        require(
            superuser,
            "BATTER_SQLX_ADMIN_URL must authenticate as a PostgreSQL superuser",
        )?;
        let version: String = sqlx::query_scalar("SHOW server_version_num")
            .fetch_one(&mut admin)
            .await?;
        require(
            version.parse::<u32>()? / 10_000 == 18,
            "the grant-composition suite is scoped to executed PostgreSQL 18 evidence",
        )?;
        let names = Names::new();
        let mut fixture = Self {
            admin,
            names,
            url,
            password: random_password()?,
            logins: Vec::new(),
            pools: Vec::new(),
        };
        if let Err(error) = fixture.provision().await {
            let cleanup = fixture.cleanup().await;
            return Err(cleanup.err().unwrap_or(error));
        }
        Ok(fixture)
    }

    async fn provision(&mut self) -> Result {
        let owner = quote(&self.names.owner);
        exec(
            &mut self.admin,
            format!(
                "CREATE ROLE {owner} LOGIN PASSWORD '{}' NOSUPERUSER NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS NOINHERIT",
                self.password
            ),
        )
        .await?;
        for schema in [
            &self.names.jobs,
            &self.names.quotas,
            &self.names.application,
            &self.names.shadow,
        ] {
            exec(
                &mut self.admin,
                format!("CREATE SCHEMA {} AUTHORIZATION {owner}", quote(schema)),
            )
            .await?;
        }
        Ok(())
    }

    /// Provision one restricted login and apply exactly one compiled role's
    /// rendered grant plan, plus the explicit PUBLIC revocations this suite
    /// declares.
    pub(crate) async fn login(&mut self, label: &str, role: &CompiledExactRole) -> Result<String> {
        let name = format!("{}_{label}", self.names.owner);
        exec(
            &mut self.admin,
            format!(
                "CREATE ROLE {} LOGIN PASSWORD '{}' NOSUPERUSER NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS NOINHERIT",
                quote(&name),
                self.password
            ),
        )
        .await?;
        self.logins.push(name.clone());
        let database: String = sqlx::query_scalar("SELECT pg_catalog.current_database()")
            .fetch_one(&mut self.admin)
            .await?;
        let rendered = role.grant_plan().render(
            &Identifier::new(name.clone())?,
            Some(&Identifier::new(database)?),
        )?;
        exec(&mut self.admin, rendered).await?;
        Ok(name)
    }

    /// An ordinary pooled connection for `role` with an exact trusted search path.
    pub(crate) async fn pool(&mut self, role: &str, schemas: &[&str]) -> Result<PgPool> {
        let search_path = schemas
            .iter()
            .map(|schema| quote(schema))
            .collect::<Vec<_>>()
            .join(", ");
        let options = PgConnectOptions::from_str(&self.url)?
            .username(role)
            .password(&self.password)
            .options([("search_path", search_path.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(20))
            .connect_with(options)
            .await?;
        self.pools.push(pool.clone());
        Ok(pool)
    }

    /// Connection options for `role`, for pools this suite does not own.
    pub(crate) fn options(&self, role: &str) -> Result<PgConnectOptions> {
        Ok(PgConnectOptions::from_str(&self.url)?
            .username(role)
            .password(&self.password))
    }

    pub(crate) fn track(&mut self, pool: PgPool) {
        self.pools.push(pool);
    }

    async fn cleanup(&mut self) -> Result {
        let mut first = None;
        for pool in std::mem::take(&mut self.pools) {
            pool.close().await;
        }
        for schema in [
            &self.names.jobs,
            &self.names.quotas,
            &self.names.application,
            &self.names.shadow,
        ] {
            if let Err(error) = exec(
                &mut self.admin,
                format!("DROP SCHEMA IF EXISTS {} CASCADE", quote(schema)),
            )
            .await
            {
                first.get_or_insert(error);
            }
        }
        for role in std::mem::take(&mut self.logins)
            .iter()
            .chain(std::iter::once(&self.names.owner))
        {
            for statement in [
                format!("DROP OWNED BY {}", quote(role)),
                format!("DROP ROLE IF EXISTS {}", quote(role)),
            ] {
                if let Err(error) = exec(&mut self.admin, statement).await {
                    first.get_or_insert(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    /// Run `body` and always attempt cleanup, retaining the first failure.
    pub(crate) async fn run<F>(body: F) -> Result
    where
        F: for<'a> AsyncFnOnce(&'a mut Self) -> Result,
    {
        let mut fixture = Self::create().await?;
        let outcome = body(&mut fixture).await;
        let cleanup = fixture.cleanup().await;
        match (outcome, cleanup) {
            (Ok(()), cleanup) => cleanup,
            (Err(error), _) => Err(error),
        }
    }
}

fn random_password() -> Result<String> {
    let mut bytes = [0_u8; 24];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut password = String::from("batter_grants_pw_");
    for byte in bytes {
        write!(password, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(password)
}

/// PostgreSQL's insufficient-privilege class.
pub(crate) const DENIED: &str = "42501";

/// Require that one statement is refused for want of privilege.
pub(crate) async fn require_denied(pool: &PgPool, statement: &str) -> Result {
    let error = sqlx::raw_sql(sqlx::AssertSqlSafe(statement.to_owned()))
        .execute(pool)
        .await
        .err()
        .ok_or_else(|| fail(&format!("forbidden statement was permitted: {statement}")))?;
    require(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref()
            == Some(DENIED),
        &format!("forbidden statement failed for another reason: {statement}: {error}"),
    )
}

/// Require that one statement succeeds under the restricted login.
pub(crate) async fn require_permitted(pool: &PgPool, statement: &str) -> Result {
    sqlx::raw_sql(sqlx::AssertSqlSafe(statement.to_owned()))
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|error| {
            fail(&format!(
                "required statement was refused: {statement}: {error}"
            ))
        })
}

/// Verify one compiled role under a restricted login and return its report.
pub(crate) async fn verification(
    pool: &PgPool,
    role: &CompiledExactRole,
) -> Result<batter::sqlx::verification::VerificationReport> {
    let context = batter::operation::OperationOwner::new(Duration::from_secs(60))?.into_context();
    Ok(batter::sqlx::verification::verify(
        pool,
        &context,
        batter::sqlx::verification::VerificationPlan::exact_role(role),
    )
    .await?)
}

/// Require that a compiled role is satisfied exactly by the provisioned login.
pub(crate) async fn require_within_policy(pool: &PgPool, role: &CompiledExactRole) -> Result {
    let report = verification(pool, role).await?;
    require(
        report.status() == batter::sqlx::verification::VerificationStatus::WithinDeclaredPolicy,
        &format!("a composed role was not satisfied exactly: {report:?}"),
    )
}

/// Require that verification reports a policy violation rather than passing.
pub(crate) async fn require_violation(pool: &PgPool, role: &CompiledExactRole) -> Result {
    let report = verification(pool, role).await?;
    require(
        report.status() == batter::sqlx::verification::VerificationStatus::Violations,
        &format!("verification accepted a drifted role: {report:?}"),
    )
}

/// Keep the concrete native atomic failure in the reported cause.
pub(crate) trait PgAtomicFailure {
    fn into_error(self) -> Box<dyn std::error::Error + Send + Sync>;
}

impl<T: std::fmt::Debug> PgAtomicFailure for batter::runledger::PgAtomicError<T, String> {
    fn into_error(self) -> Box<dyn std::error::Error + Send + Sync> {
        fail(&format!("{self:?}"))
    }
}
