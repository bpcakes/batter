use super::support::{self, Result, bounded};
use futures_util::FutureExt;
use sqlx::{
    Connection, Executor, PgConnection, PgPool, Postgres,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fmt::Write as _,
    fs::File,
    future::Future,
    io::Read,
    pin::Pin,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

mod provision;

use provision::provision;
#[derive(Clone)]
pub(crate) struct Names {
    pub(crate) schema_a: String,
    pub(crate) schema_b: String,
    pub(crate) login_a: String,
    pub(crate) login_b: String,
    pub(crate) login_noinherit: String,
    pub(crate) inherited: String,
    pub(crate) nested: String,
    pub(crate) settable: String,
    pub(crate) admin_target: String,
    pub(crate) unrelated_creator: String,
    pub(crate) owner: String,
    pub(crate) ledger_a: String,
    pub(crate) ledger_b: String,
    pub(crate) table_a: String,
    pub(crate) table_b: String,
    pub(crate) sequence_a: String,
    pub(crate) sequence_default: String,
    pub(crate) type_a: String,
    pub(crate) routine_a: String,
    pub(crate) hidden_parameter: String,
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
        let name = |prefix: &str| format!("batter_vfy_{prefix}_{suffix}");
        Self {
            schema_a: name("schema_a"),
            schema_b: name("schema_b"),
            login_a: name("login_a"),
            login_b: name("login_b"),
            login_noinherit: name("login_noinherit"),
            inherited: name("inherited"),
            nested: name("nested"),
            settable: name("settable"),
            admin_target: name("admin_target"),
            unrelated_creator: name("unrelated_creator"),
            owner: name("owner"),
            ledger_a: name("ledger_a"),
            ledger_b: name("ledger_b"),
            table_a: name("table_a"),
            table_b: name("table_b"),
            sequence_a: name("sequence_a"),
            sequence_default: name("sequence_default"),
            type_a: name("type_a"),
            routine_a: name("routine_a"),
            hidden_parameter: format!("batter_vfy.hidden_{suffix}"),
        }
    }
}

pub(crate) struct AuthorityFixture {
    pub(crate) admin: PgConnection,
    pub(crate) url: String,
    pub(crate) names: Names,
    pub(crate) password: String,
    pools: Vec<PgPool>,
}

impl AuthorityFixture {
    pub(crate) async fn create() -> Result<Self> {
        let url = std::env::var("BATTER_SQLX_ADMIN_URL").map_err(|_| {
            std::io::Error::other("live verification requires BATTER_SQLX_ADMIN_URL")
        })?;
        let mut admin = bounded(PgConnection::connect(&url)).await??;
        let names = Names::new();
        let password = random_password()?;
        if let Err(error) = provision(&mut admin, &names, &password).await {
            let cleanup_result = cleanup(&mut admin, &names).await;
            return match support::combine(Err(error), cleanup_result) {
                Ok(()) => Err(std::io::Error::other(
                    "fixture provisioning unexpectedly succeeded after failure",
                )
                .into()),
                Err(error) => Err(error),
            };
        }
        Ok(Self {
            admin,
            url,
            names,
            password,
            pools: Vec::new(),
        })
    }

    pub(crate) async fn login(&mut self, role: &str) -> Result<PgPool> {
        self.login_with_options(role, std::iter::empty::<(&str, &str)>())
            .await
    }

    pub(crate) async fn login_with_options<I, K, V>(
        &mut self,
        role: &str,
        options: I,
    ) -> Result<PgPool>
    where
        I: IntoIterator<Item = (K, V)>,
        K: std::fmt::Display,
        V: std::fmt::Display,
    {
        let options = PgConnectOptions::from_str(&self.url)?
            .username(role)
            .password(&self.password)
            .options(options);
        let pool = bounded(
            PgPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .connect_with(options),
        )
        .await??;
        self.pools.push(pool.clone());
        Ok(pool)
    }

    async fn cleanup(mut self) -> Result {
        let mut result = Ok(());
        for pool in self.pools.drain(..) {
            result = support::combine(result, bounded(pool.close()).await);
        }
        let rollback = sqlx::query("ROLLBACK")
            .execute(&mut self.admin)
            .await
            .map(|_| ())
            .map_err(Into::into);
        result = support::combine(result, rollback);
        support::combine(result, cleanup(&mut self.admin, &self.names).await)
    }

    pub(crate) async fn run<F>(mut self, body: F) -> Result
    where
        F: for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = Result> + 'a>>,
    {
        // Catch the body panic before its stack unwinds past the fixture owner.
        // This keeps cleanup and its failure cause available without exposing a
        // panic payload that could contain a native query or credential.
        let body = match std::panic::AssertUnwindSafe(body(&mut self))
            .catch_unwind()
            .await
        {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::other("verification fixture body panicked").into()),
        };
        let cleanup = self.cleanup().await;
        support::combine(body, cleanup)
    }
}

fn random_password() -> Result<String> {
    let mut bytes = [0_u8; 24];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut password = String::from("batter_vfy_pw_");
    for byte in bytes {
        write!(password, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(password)
}

async fn cleanup(connection: &mut PgConnection, names: &Names) -> Result {
    let mut first = None;
    // `DROP OWNED` updates the shared pg_parameter_acl row for work_mem;
    // serialize fixture teardown just as provisioning serializes its GRANT.
    exec(
        &mut *connection,
        "SELECT pg_catalog.pg_advisory_lock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    if let Err(error) = exec(
        &mut *connection,
        format!(
            "REVOKE ALL ON PARAMETER {} FROM PUBLIC",
            names.hidden_parameter
        ),
    )
    .await
    {
        first.get_or_insert(error);
    }
    for schema in [&names.schema_a, &names.schema_b] {
        if let Err(error) = exec(
            &mut *connection,
            format!("DROP SCHEMA IF EXISTS {} CASCADE", quote(schema)),
        )
        .await
        {
            first.get_or_insert(error);
        }
    }
    for role in [
        &names.login_a,
        &names.login_b,
        &names.login_noinherit,
        &names.inherited,
        &names.nested,
        &names.settable,
        &names.admin_target,
        &names.unrelated_creator,
        &names.owner,
    ] {
        if let Err(error) = exec(&mut *connection, format!("DROP OWNED BY {}", quote(role))).await {
            first.get_or_insert(error);
        }
        if let Err(error) = exec(
            &mut *connection,
            format!("DROP ROLE IF EXISTS {}", quote(role)),
        )
        .await
        {
            first.get_or_insert(error);
        }
    }
    if let Err(error) = exec(
        &mut *connection,
        "SELECT pg_catalog.pg_advisory_unlock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await
    {
        first.get_or_insert(error);
    }
    match first {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(crate) async fn exec<'e>(
    connection: impl Executor<'e, Database = Postgres>,
    sql: String,
) -> Result {
    exec_secret(connection, sql, "").await
}

async fn exec_secret<'e>(
    connection: impl Executor<'e, Database = Postgres>,
    sql: String,
    secret: &str,
) -> Result {
    // Every caller builds this SQL exclusively from `quote`d fixture
    // identifiers and fixed privilege keywords; no external value reaches
    // the statement text.
    let statement = sql.clone();
    let result = bounded(sqlx::query(sqlx::AssertSqlSafe(sql)).execute(connection)).await?;
    result.map_err(|error| {
        let safe_statement = if secret.is_empty() {
            statement.clone()
        } else {
            statement.replace(secret, "<redacted>")
        };
        std::io::Error::other(format!(
            "fixture statement failed: {}; native: {error}",
            safe_statement,
        ))
    })?;
    Ok(())
}

async fn grant(
    connection: &mut PgConnection,
    object: &str,
    privilege: &str,
    role: &str,
    grant_option: bool,
) -> Result {
    let option = if grant_option {
        " WITH GRANT OPTION"
    } else {
        ""
    };
    exec(
        connection,
        format!("GRANT {privilege} ON {object} TO {}{option}", quote(role)),
    )
    .await
}

async fn grant_column(
    connection: &mut PgConnection,
    schema: &str,
    table: &str,
    column: &str,
    privilege: &str,
    role: &str,
    grant_option: bool,
) -> Result {
    let option = if grant_option {
        " WITH GRANT OPTION"
    } else {
        ""
    };
    exec(
        connection,
        format!(
            "GRANT {privilege} ({}) ON {}.{} TO {}{option}",
            quote(column),
            quote(schema),
            quote(table),
            quote(role),
        ),
    )
    .await
}

pub(crate) fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn object_kind(object: &str) -> &str {
    if object.contains("sequence") {
        "SEQUENCE"
    } else if object.contains("type") {
        "TYPE"
    } else {
        "TABLE"
    }
}
