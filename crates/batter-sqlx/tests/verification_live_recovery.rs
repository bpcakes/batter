use super::support::{Result, require};
use super::{AuthorityFixture, FindingKind, RolePolicy, exec, finding, names_policy, quote};
use sqlx::{Connection, PgConnection};
use std::time::Duration;

// This bounds external PostgreSQL lock progress, not verifier semantics.
const EXTERNAL_LOCK_ALLOWANCE: Duration = Duration::from_secs(20);

async fn lock_is_pending(observer: &mut PgConnection, pid: i32, relation: i64) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks
         WHERE pid = $1 AND relation::bigint = $2 AND NOT granted)",
    )
    .bind(pid)
    .bind(relation)
    .fetch_one(observer)
    .await?)
}

pub(super) async fn pending_lock(observer: &mut PgConnection, pid: i32, relation: i64) -> Result {
    tokio::time::timeout(EXTERNAL_LOCK_ALLOWANCE, async {
        loop {
            if lock_is_pending(observer, pid, relation).await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| std::io::Error::other("expected relation lock was not observed"))?
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_ledger_ddl_before_lock_is_observed() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let ledger = format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a));
        let pool = fixture.login(&names.login_a).await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let oid: i64 = sqlx::query_scalar("SELECT $1::regclass::oid::bigint")
            .bind(&ledger).fetch_one(&mut fixture.admin).await?;
        let mut mutator = PgConnection::connect(&fixture.url).await?;
        let mut change = mutator.begin().await?;
        exec(&mut *change, format!("ALTER TABLE {ledger} ENABLE ROW LEVEL SECURITY")).await?;
        let policy = names_policy(&names, &names.schema_a, &names.ledger_a,
            &names.table_a, RolePolicy::default(), Vec::new(), false, true, false, false, false)?;
        let verification = super::verify_policy(&pool, &policy);
        tokio::pin!(verification);
        tokio::select! {
            _ = &mut verification => return Err(std::io::Error::other("verifier completed before held ledger lock released").into()),
            observed = pending_lock(&mut fixture.admin, pid, oid) => observed?,
        }
        change.commit().await?;
        let report = verification.await?;
        require(finding(&report, FindingKind::LedgerRowSecurity),
            "DDL committed ahead of ledger lock was absent from the verification snapshot")?;
        mutator.close().await?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_ledger_ddl_after_lock_waits_for_snapshot() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let ledger = format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a));
        exec(&mut fixture.admin, format!("INSERT INTO {ledger} VALUES (2, '\\x02', false)")).await?;
        exec(&mut fixture.admin, format!(
            "CREATE POLICY visible_required ON {ledger} TO {} USING (version = 1)", quote(&names.login_a)
        )).await?;
        let pool = fixture.login(&names.login_a).await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let oid: i64 = sqlx::query_scalar("SELECT $1::regclass::oid::bigint")
            .bind(&ledger).fetch_one(&mut fixture.admin).await?;
        let catalog_oid: i64 = sqlx::query_scalar("SELECT 'pg_catalog.pg_parameter_acl'::regclass::oid::bigint")
            .fetch_one(&mut fixture.admin).await?;
        let mut blocker = PgConnection::connect(&fixture.url).await?;
        let mut barrier = blocker.begin().await?;
        sqlx::query("LOCK TABLE pg_catalog.pg_parameter_acl IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *barrier).await?;
        let policy = names_policy(&names, &names.schema_a, &names.ledger_a,
            &names.table_a, RolePolicy::default(), Vec::new(), false, true, false, false, false)?;
        let verification = super::verify_policy(&pool, &policy);
        tokio::pin!(verification);
        tokio::select! {
            _ = &mut verification => return Err(std::io::Error::other("verifier completed before catalog barrier").into()),
            observed = pending_lock(&mut fixture.admin, pid, catalog_oid) => observed?,
        }
        let held: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks WHERE pid=$1
             AND relation::bigint=$2 AND mode='AccessShareLock' AND granted)"
        ).bind(pid).bind(oid).fetch_one(&mut fixture.admin).await?;
        require(held, "verifier did not retain the ledger lock across catalog inspection")?;
        let mut mutator = PgConnection::connect(&fixture.url).await?;
        let mutator_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut mutator).await?;
        let ddl = format!("ALTER TABLE {ledger} ENABLE ROW LEVEL SECURITY");
        let change = sqlx::query(sqlx::AssertSqlSafe(ddl)).execute(&mut mutator);
        tokio::pin!(change);
        tokio::select! {
            _ = &mut verification => return Err(std::io::Error::other("verifier left held catalog barrier early").into()),
            _ = &mut change => return Err(std::io::Error::other("DDL crossed the verifier's ledger lock").into()),
            observed = pending_lock(&mut fixture.admin, mutator_pid, oid) => observed?,
        }
        barrier.rollback().await?;
        let (report, changed) = tokio::time::timeout(EXTERNAL_LOCK_ALLOWANCE, async {
            tokio::join!(&mut verification, &mut change)
        }).await?;
        changed?;
        let report = report?;
        require(report.findings().iter().any(|item| item.kind == FindingKind::UnexpectedMigration && item.subject.as_deref() == Some("2"))
            && !finding(&report, FindingKind::LedgerRowSecurity),
            "concurrent DDL hid a ledger row from the pre-DDL snapshot")?;
        let visible: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT version FROM {ledger} ORDER BY version"
        ))).fetch_all(&pool).await?;
        require(visible == vec![1], "post-verification DDL did not activate the row-filtering control")?;
        blocker.close().await?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_catalog_resolution_preserves_serving_state() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let database: String = sqlx::query_scalar("SELECT pg_catalog.current_database()::text")
            .fetch_one(&mut fixture.admin).await?;
        exec(&mut fixture.admin, format!("GRANT CREATE ON DATABASE {} TO {}", quote(&database), quote(&names.login_a))).await?;
        exec(&mut fixture.admin, format!(
            "CREATE FUNCTION {}.current_database() RETURNS name LANGUAGE sql AS 'SELECT ''template1''::name'", quote(&names.schema_a)
        )).await?;
        exec(&mut fixture.admin, format!(
            "CREATE FUNCTION {}.\"left\"(name, integer) RETURNS text LANGUAGE sql AS 'SELECT ''pg_''::text'", quote(&names.schema_a)
        )).await?;
        let hidden_table = format!("{}.shadowed_records", quote(&names.schema_a));
        exec(&mut fixture.admin, format!("CREATE TABLE {hidden_table} (id integer)")).await?;
        exec(&mut fixture.admin, format!("GRANT DELETE ON {hidden_table} TO {}", quote(&names.login_a))).await?;
        let path = format!("{}, pg_catalog, pg_temp", quote(&names.schema_a));
        let pool = fixture.login_with_options(&names.login_a, [("search_path", path)]).await?;
        let mut native = pool.begin().await?;
        exec(&mut *native, format!("CREATE SCHEMA {}", quote(&format!("{}_native", names.schema_a)))).await?;
        // Exact overloads can win even when pg_catalog is placed first.
        exec(&mut *native, format!("SET LOCAL search_path = pg_catalog, {}, pg_temp", quote(&names.schema_a))).await?;
        let prefix: String = sqlx::query_scalar("SELECT left(nspname, 3) FROM pg_catalog.pg_namespace WHERE nspname=$1")
            .bind(&names.schema_a).fetch_one(&mut *native).await?;
        require(prefix == "pg_", "the exact name-typed overload did not intercept discovery's helper")?;
        native.rollback().await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {hidden_table}"))).execute(&pool).await?;
        let before: String = sqlx::query_scalar("SHOW search_path").fetch_one(&pool).await?;
        let intercepted: String = sqlx::query_scalar("SELECT current_database()::text").fetch_one(&pool).await?;
        require(intercepted == "template1" && intercepted != database,
            "the application-first search path did not intercept the helper")?;
        let mut policy = names_policy(&names, &names.schema_a, &names.ledger_a,
            &names.table_a, RolePolicy::default(), Vec::new(), false, true, false, false, false)?;
        policy.authority.discovery = batter_sqlx::verification::DiscoveryScope::UserSchemas;
        let report = super::verify_policy(&pool, &policy).await?;
        require(report.findings().iter().any(|item| item.kind == FindingKind::Privilege
            && item.object.as_deref().is_some_and(|name| name.contains("shadowed_records"))
            && item.privilege == Some(super::ObjectPrivilege::Delete)),
            "an exact application overload hid a discovered relation")?;
        require(report.findings().iter().any(|item| item.kind == FindingKind::Privilege
            && item.object.as_deref() == Some(database.as_str())
            && item.subject.as_deref() == Some(names.login_a.as_str())
            && item.privilege == Some(super::ObjectPrivilege::Create)),
            "an application helper redirected database ACL inspection")?;
        let after: String = sqlx::query_scalar("SHOW search_path").fetch_one(&pool).await?;
        let restored: String = sqlx::query_scalar("SELECT current_database()::text").fetch_one(&pool).await?;
        require(before == after && restored == "template1",
            "verification did not restore the application's serving resolution state")?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_ledger_descendant_truncate_waits_for_snapshot() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let ledger = format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a));
        let child = format!("{}.ledger_child", quote(&names.schema_a));
        exec(&mut fixture.admin, format!("CREATE TABLE {child} () INHERITS ({ledger})")).await?;
        exec(&mut fixture.admin, format!("INSERT INTO {child} VALUES (2, '\\x02', false)")).await?;
        let pool = fixture.login(&names.login_a).await?;
        // SELECT on the parent permits reading descendants without a separate child grant.
        let versions: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT version FROM {ledger} ORDER BY version")))
            .fetch_all(&pool).await?;
        require(versions == vec![1,2], "descendant control did not contribute a ledger row")?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let catalog_oid: i64 = sqlx::query_scalar("SELECT 'pg_catalog.pg_attribute'::regclass::oid::bigint")
            .fetch_one(&mut fixture.admin).await?;
        let child_oid: i64 = sqlx::query_scalar("SELECT $1::regclass::oid::bigint")
            .bind(&child).fetch_one(&mut fixture.admin).await?;
        // Warm observers and mutator before holding a system-catalog barrier.
        // Their own metadata reads must not become the barrier under test.
        lock_is_pending(&mut fixture.admin, pid, catalog_oid).await?;
        let _: String = sqlx::query_scalar("SELECT query FROM pg_catalog.pg_stat_activity WHERE pid=$1")
            .bind(pid).fetch_one(&mut fixture.admin).await?;
        let mut mutator = PgConnection::connect(&fixture.url).await?;
        let mutator_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut mutator).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {child}"))).execute(&mut mutator).await?;
        let mut blocker = PgConnection::connect(&fixture.url).await?;
        let mut barrier = blocker.begin().await?;
        sqlx::query("LOCK TABLE pg_catalog.pg_attribute IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *barrier).await?;
        let policy = names_policy(&names, &names.schema_a, &names.ledger_a,
            &names.table_a, RolePolicy::default(), Vec::new(), false, true, false, false, false)?;
        let verification = super::verify_policy(&pool, &policy);
        tokio::pin!(verification);
        tokio::select! {
            _ = &mut verification => return Err(std::io::Error::other("verifier completed before ledger shape barrier").into()),
            observed = pending_lock(&mut fixture.admin, pid, catalog_oid) => observed?,
        }
        let query: String = sqlx::query_scalar("SELECT query FROM pg_catalog.pg_stat_activity WHERE pid=$1")
            .bind(pid).fetch_one(&mut fixture.admin).await?;
        require(query.contains("FROM pg_catalog.pg_class AS c")
            && query.contains("JOIN pg_catalog.pg_locks AS l"),
            "barrier did not pause the ledger lock-attestation read")?;
        let truncate = format!("TRUNCATE {child}");
        let change = sqlx::query(sqlx::AssertSqlSafe(truncate)).execute(&mut mutator);
        tokio::pin!(change);
        tokio::select! {
            _ = &mut verification => return Err(std::io::Error::other("verifier left held shape barrier").into()),
            _ = &mut change => return Err(std::io::Error::other("descendant truncate crossed ledger snapshot protection").into()),
            observed = pending_lock(&mut fixture.admin, mutator_pid, child_oid) => observed?,
        }
        barrier.rollback().await?;
        let (report, changed) = tokio::time::timeout(EXTERNAL_LOCK_ALLOWANCE, async {
            tokio::join!(&mut verification, &mut change)
        }).await?;
        changed?;
        let report = report?;
        require(report.status() == batter_sqlx::verification::VerificationStatus::Incomplete
            && report.unsupported() == [batter_sqlx::verification::UnsupportedSurface::InheritedMigrationLedgers]
            && report.findings().is_empty(), "inherited ledger escaped the unsupported boundary")?;
        let remaining: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT version FROM {ledger} ORDER BY version")))
            .fetch_all(&pool).await?;
        require(remaining == vec![1], "released descendant truncate did not complete")?;
        blocker.close().await?;
        Ok(())
    })).await
}
