use super::support::{Result, bounded, combine, require};
use super::{AuthorityFixture, exec, quote};
use batter::operation::OperationContext;
use batter_sqlx::verification::*;
use sqlx::Connection;
use std::time::Duration;

async fn inspect(
    pool: &sqlx::PgPool,
    draft: &AuthorityPolicyBuilder,
) -> Result<VerificationReport> {
    let policy = draft.clone().build()?;
    Ok(verify_authority(
        pool,
        &OperationContext::new(Duration::from_secs(10))?,
        &policy,
    )
    .await?)
}

async fn inspect_combined(
    pool: &sqlx::PgPool,
    migration: &MigrationPolicy,
    draft: &AuthorityPolicyBuilder,
) -> Result<VerificationReport> {
    let authority = draft.clone().build()?;
    let plan = VerificationPlan::migrations(migration).with_authority(&authority)?;
    Ok(verify(pool, &OperationContext::new(Duration::from_secs(10))?, plan).await?)
}

fn incomplete(report: &VerificationReport) -> Result {
    require(
        report.status() == VerificationStatus::Incomplete
            && report
                .unsupported()
                .contains(&UnsupportedSurface::TemporaryNamespaces)
            && report.findings().is_empty()
            && report.supported().is_empty(),
        "temporary selection was evaluated as an ordinary supported surface",
    )
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_temporary_namespace_requests_are_incomplete() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let pool = fixture.login(&names.login_a).await?;
        exec(&pool, "CREATE TEMP TABLE session_records(id integer)".into()).await?;
        let schema: String = sqlx::query_scalar("SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()")
            .fetch_one(&pool).await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let native: (bool, bool) = sqlx::query_as("SELECT has_schema_privilege(pg_my_temp_schema(), 'USAGE'), has_schema_privilege(pg_my_temp_schema(), 'CREATE')")
            .fetch_one(&pool).await?;
        require(native == (true, true), "native temporary namespace setup failed")?;
        let mut policy = AuthorityPolicyBuilder {
            schemas: vec![SchemaPolicy { schema: Identifier::new(&schema)?, privileges: Vec::new(), allow_owner: false }],
            ..AuthorityPolicyBuilder::default()
        };
        incomplete(&inspect(&pool, &policy).await?)?;
        policy.schemas[0].privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false), AllowedPrivilege::new(ObjectPrivilege::Create, false)];
        for privilege in [ObjectPrivilege::Usage, ObjectPrivilege::Create] {
            policy.required_privileges.push(RequiredPrivilege { object: PublicObject::Schema(Identifier::new(&schema)?), privilege });
        }
        incomplete(&inspect(&pool, &policy).await?)?;
        // Alias and other-session names must not bypass the boundary or depend
        // on which checkout happens to be selected from a pool.
        let other = fixture.login(&names.login_b).await?;
        exec(&other, "CREATE TEMP TABLE other_records(id integer)".into()).await?;
        let other_schema: String = sqlx::query_scalar("SELECT nspname::text FROM pg_namespace WHERE oid=pg_my_temp_schema()")
            .fetch_one(&other).await?;
        for selected in [&schema, &other_schema, "pg_temp"] {
            let discovery = AuthorityPolicyBuilder { discovery: DiscoveryScope::Schemas(vec![Identifier::new(selected)?]), ..AuthorityPolicyBuilder::default() };
            incomplete(&inspect(&pool, &discovery).await?)?;
            let migrations = MigrationPolicy::new(QualifiedName::new(selected, "absent_ledger")?, [])?;
            incomplete(&verify_migrations(&pool, &OperationContext::new(Duration::from_secs(10))?, &migrations).await?)?;
            incomplete(&inspect_combined(&pool, &migrations, &AuthorityPolicyBuilder::default()).await?)?;
        }
        // An authority request must be rejected before even a missing ordinary
        // ledger can produce unrelated migration findings.
        let missing = MigrationPolicy::new(QualifiedName::new(&names.schema_a, "absent_ledger")?, [])?;
        incomplete(&inspect_combined(&pool, &missing, &policy).await?)?;
        let database: String = sqlx::query_scalar("SELECT current_database()::text").fetch_one(&pool).await?;
        let public_temp: bool = sqlx::query_scalar("SELECT has_database_privilege('public', current_database(), 'TEMP')")
            .fetch_one(&mut fixture.admin).await?;
        require(public_temp, "temporary-scope fixture requires initial PUBLIC TEMP")?;
        let changed: Result = async {
            exec(&mut fixture.admin, format!("REVOKE TEMP ON DATABASE {} FROM PUBLIC", quote(&database))).await?;
            // Existing temporary objects still have native USAGE after TEMP is
            // revoked, including after switching to a role without TEMP.
            exec(&pool, format!("SET ROLE {}", quote(&names.settable))).await?;
            let native: (bool, bool) = sqlx::query_as("SELECT has_schema_privilege(pg_my_temp_schema(), 'USAGE'), has_schema_privilege(pg_my_temp_schema(), 'CREATE')")
                .fetch_one(&pool).await?;
            require(native == (true, false), "native temporary namespace did not reflect revoked TEMP")?;
            let report = inspect(&pool, &policy).await?;
            incomplete(&report)?;
            require(report.session_user() == names.login_a && report.current_user() == names.settable,
                "unsupported report lost serving identity")?;
            exec(&pool, "RESET ROLE".into()).await?;
            let state: (i32, i64) = sqlx::query_as("SELECT pg_backend_pid(), count(*) FROM session_records")
                .fetch_one(&pool).await?;
            require(state == (pid, 0), "unsupported inspection changed or retired the clean serving session")
        }.await;
        let restored = exec(&mut fixture.admin, format!("GRANT TEMP ON DATABASE {} TO PUBLIC", quote(&database))).await;
        combine(changed, restored)?;
        exec(&pool, "DROP TABLE session_records".into()).await?;
        exec(&other, "DROP TABLE other_records".into()).await?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_public_relation_overrides_control_column_defaults() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let relation = QualifiedName::new(&names.schema_a, "public_columns")?;
        let table = format!("{}.public_columns", quote(&names.schema_a));
        exec(&mut fixture.admin, format!("CREATE TABLE {table}(id integer, value integer)")).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT(id) ON {table} TO PUBLIC")).await?;
        let pool = fixture.login(&names.login_a).await?;
        let native: (bool, bool) = sqlx::query_as("SELECT has_table_privilege($1::text, 'SELECT'), has_column_privilege($1::text, 'id', 'SELECT')")
            .bind(&table).fetch_one(&pool).await?;
        require(native == (false, true), "native column-only PUBLIC grant setup failed")?;
        let mut policy = AuthorityPolicyBuilder { discovery: DiscoveryScope::Schemas(vec![Identifier::new(&names.schema_a)?]), ..AuthorityPolicyBuilder::default() };
        policy.defaults.columns.public_privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
        let column = PublicObject::Column(relation.clone(), Identifier::new("id")?);
        let selected = format!("{}.{}.{}", quote(&names.schema_a), quote("public_columns"), quote("id"));
        let forbidden = |report: &VerificationReport| report.findings().iter().any(|f| f.kind == FindingKind::Privilege && f.subject.as_deref() == Some("public")
            && f.object.as_deref() == Some(selected.as_str()) && f.privilege == Some(ObjectPrivilege::Select));
        require(!forbidden(&inspect(&pool, &policy).await?), "column PUBLIC default did not apply")?;
        policy.public_overrides.push(PublicAllowance { object: PublicObject::Relation(relation.clone()), privileges: Vec::new() });
        require(forbidden(&inspect(&pool, &policy).await?), "exact PUBLIC relation deny retained column defaults")?;
        policy.public_overrides.push(PublicAllowance { object: column.clone(), privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)] });
        require(!forbidden(&inspect(&pool, &policy).await?), "explicit column exception was lost under parent deny")?;
        policy.required_privileges.push(RequiredPrivilege { object: column, privilege: ObjectPrivilege::Select });
        require(!inspect(&pool, &policy).await?.findings().iter().any(|f| f.kind == FindingKind::MissingPrivilege),
            "explicit PUBLIC column allowance and native requirement disagreed")?;
        policy.public_overrides[1].privileges.clear();
        require(matches!(policy.clone().build(), Err(PolicyError::ContradictoryRequiredPrivilege)), "required validation disagreed with parent/column deny precedence")?;
        policy.required_privileges.clear();
        policy.public_overrides.clear();
        policy.public_grants.push(PublicGrant { object: PublicObject::Relation(relation), privilege: AllowedPrivilege::new(ObjectPrivilege::Update, false) });
        require(forbidden(&inspect(&pool, &policy).await?), "PublicGrant parent used different precedence from PublicAllowance")?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_required_and_excess_authority_share_captured_snapshot() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let pool = fixture.login(&names.login_a).await?;
        let schema = format!("{}_snapshot", names.schema_a);
        exec(&mut fixture.admin, format!("CREATE SCHEMA {} AUTHORIZATION {}", quote(&schema), quote(&names.owner))).await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let catalog: i64 = sqlx::query_scalar("SELECT 'pg_catalog.pg_parameter_acl'::regclass::oid::bigint").fetch_one(&mut fixture.admin).await?;
        // Opening a connection can itself read pg_parameter_acl during startup.
        // Acquire every connection before the catalog barrier, with a bound.
        let mut blocker = bounded(sqlx::PgConnection::connect(&fixture.url)).await??;
        let mut observer = bounded(sqlx::PgConnection::connect(&fixture.url)).await??;
        let mut barrier = blocker.begin().await?;
        sqlx::query("LOCK TABLE pg_catalog.pg_parameter_acl IN ACCESS EXCLUSIVE MODE").execute(&mut *barrier).await?;
        sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ").execute(&mut observer).await?;
        let before: (bool, bool) = sqlx::query_as("SELECT nspacl IS NULL, has_schema_privilege($1::text, oid, 'USAGE') FROM pg_namespace WHERE nspname=$2")
            .bind(&names.login_a).bind(&schema).fetch_one(&mut observer).await?;
        require(before == (true, false), "native snapshot reference setup failed")?;
        let policy = AuthorityPolicyBuilder {
            schemas: vec![SchemaPolicy { schema: Identifier::new(&schema)?, privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)], allow_owner: false }],
            required_privileges: vec![RequiredPrivilege { object: PublicObject::Schema(Identifier::new(&schema)?), privilege: ObjectPrivilege::Usage }],
            ..AuthorityPolicyBuilder::default()
        };
        let verification = inspect(&pool, &policy);
        tokio::pin!(verification);
        tokio::select! {
            _ = &mut verification => return Err(std::io::Error::other("verification completed before catalog barrier").into()),
            result = super::verification_live_recovery::pending_lock(&mut fixture.admin, pid, catalog) => result?,
        }
        exec(&mut fixture.admin, format!("GRANT USAGE, CREATE ON SCHEMA {} TO {}", quote(&schema), quote(&names.login_a))).await?;
        let after: (bool, bool) = sqlx::query_as("SELECT nspacl IS NULL, has_schema_privilege($1::text, oid, 'USAGE') FROM pg_namespace WHERE nspname=$2 /* observe after grant */")
            .bind(&names.login_a).bind(&schema).fetch_one(&mut observer).await?;
        require(after.0, "repeatable-read catalog unexpectedly saw the later ACL")?;
        require(after.1, "new native privilege query did not see the later grant")?;
        barrier.rollback().await?;
        let report = verification.await?;
        require(report.findings().iter().any(|f| f.kind == FindingKind::MissingPrivilege && f.privilege == Some(ObjectPrivilege::Usage))
            && !report.findings().iter().any(|f| f.privilege == Some(ObjectPrivilege::Create)),
            "required and excess authority mixed pre-grant and post-grant snapshots")?;
        let current = inspect(&pool, &policy).await?;
        require(!current.findings().iter().any(|f| f.kind == FindingKind::MissingPrivilege)
            && current.findings().iter().any(|f| f.kind == FindingKind::Privilege && f.privilege == Some(ObjectPrivilege::Create)),
            "fresh verification did not observe the committed grant")?;
        observer.close().await?;
        blocker.close().await?;
        exec(&mut fixture.admin, format!("DROP SCHEMA {}", quote(&schema))).await?;
        Ok(())
    })).await
}
