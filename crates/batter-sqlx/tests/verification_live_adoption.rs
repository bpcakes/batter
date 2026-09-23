use super::support::{Result, require};
use super::{AuthorityFixture, exec, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, AuthorityPolicyBuilder, ColumnPolicy, DiscoveryScope, FindingKind,
    Identifier, ObjectPrivilege, PublicObject, QualifiedName, RelationPolicy, RequiredPrivilege,
    SupportedSurface, VerificationReport, verify_authority,
};
use std::time::Duration;

async fn inspect(
    pool: &sqlx::PgPool,
    draft: &AuthorityPolicyBuilder,
) -> Result<VerificationReport> {
    let policy = draft.clone().build()?;
    Ok(verify_authority(
        pool,
        &batter_core::operation::OperationOwner::new(Duration::from_secs(10))?.into_context(),
        &policy,
    )
    .await?)
}

fn has(
    report: &VerificationReport,
    kind: FindingKind,
    object: &str,
    privilege: ObjectPrivilege,
) -> bool {
    report.findings().iter().any(|item| {
        item.kind == kind
            && item
                .object
                .as_deref()
                .is_some_and(|name| name.contains(object))
            && item.privilege == Some(privilege)
    })
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_required_privileges_follow_current_inheritance() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let object = QualifiedName::new(&names.schema_a, "required_records")?;
        let table = format!("{}.required_records", quote(&names.schema_a));
        exec(&mut fixture.admin, format!("CREATE TABLE {table} (id integer, value integer)")).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT ON {table} TO {}", quote(&names.settable))).await?;
        let pool = fixture.login(&names.login_a).await?;
        let mut policy = AuthorityPolicyBuilder {
            relations: vec![RelationPolicy {
                relation: object.clone(),
                privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
                columns: vec![ColumnPolicy { column: Identifier::new("value")?,
                    privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)] }],
                allow_owner: false, allow_row_type_public_usage: true,
            }],
            required_privileges: vec![
                RequiredPrivilege { object: PublicObject::Relation(object.clone()), privilege: ObjectPrivilege::Select },
                RequiredPrivilege { object: PublicObject::Column(object, Identifier::new("value")?), privilege: ObjectPrivilege::Select },
            ],
            ..AuthorityPolicyBuilder::default()
        };
        let native = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}"))).execute(&pool).await;
        require(native.is_err(), "SET-only membership unexpectedly supplied current SELECT")?;
        let missing = inspect(&pool, &policy).await?;
        require(has(&missing, FindingKind::MissingPrivilege, "required_records", ObjectPrivilege::Select),
            "potential SET authority incorrectly satisfied a current-role requirement")?;
        require(!missing.supported().contains(&SupportedSurface::MigrationLedger),
            "authority-only check claimed ledger coverage")?;
        exec(&pool, format!("SET ROLE {}", quote(&names.settable))).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}"))).execute(&pool).await?;
        let switched = inspect(&pool, &policy).await?;
        require(!has(&switched, FindingKind::MissingPrivilege, "required_records", ObjectPrivilege::Select)
            && switched.session_user() == names.login_a && switched.current_user() == names.settable,
            "required privileges did not follow current role while retaining authenticated root")?;
        exec(&pool, "RESET ROLE".into()).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT ON {table} TO {}", quote(&names.inherited))).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}"))).execute(&pool).await?;
        require(!has(&inspect(&pool, &policy).await?, FindingKind::MissingPrivilege,
            "required_records", ObjectPrivilege::Select), "INHERIT SELECT did not satisfy requirements")?;
        exec(&mut fixture.admin, format!("REVOKE SELECT ON {table} FROM {}", quote(&names.inherited))).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT ON {table} TO PUBLIC")).await?;
        require(!has(&inspect(&pool, &policy).await?, FindingKind::MissingPrivilege,
            "required_records", ObjectPrivilege::Select), "PUBLIC table SELECT did not satisfy table/column requirements")?;
        exec(&mut fixture.admin, format!("REVOKE SELECT ON {table} FROM PUBLIC")).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT(value) ON {table} TO {}", quote(&names.login_a))).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT value FROM {table}"))).execute(&pool).await?;
        require(sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}"))).execute(&pool).await.is_err(),
            "column-only SELECT unexpectedly supplied full-table SELECT")?;
        let column_only = inspect(&pool, &policy).await?;
        require(column_only.findings().iter().filter(|item| item.kind == FindingKind::MissingPrivilege).count() == 1,
            "column-only grant did not distinguish the table requirement")?;
        // Required ledger permissions remain inspectable without reading it.
        policy.required_privileges.truncate(1);
        require(has(&inspect(&pool, &policy).await?, FindingKind::MissingPrivilege,
            "required_records", ObjectPrivilege::Select), "revoked required SELECT was hidden")?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_discovery_checks_unlisted_objects_and_exact_overrides() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let schema = quote(&names.schema_a);
        exec(&mut fixture.admin, format!("CREATE TABLE {schema}.unlisted_records (id integer)")).await?;
        exec(&mut fixture.admin, format!("CREATE SEQUENCE {schema}.unlisted_sequence")).await?;
        exec(&mut fixture.admin, format!("CREATE FUNCTION {schema}.unlisted_definer() RETURNS integer LANGUAGE sql SECURITY DEFINER AS 'SELECT 1'")).await?;
        exec(&mut fixture.admin, format!("REVOKE ALL ON FUNCTION {schema}.unlisted_definer() FROM PUBLIC")).await?;
        exec(&mut fixture.admin, format!("GRANT DELETE, UPDATE(id) ON {schema}.unlisted_records TO {}", quote(&names.login_a))).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT ON {schema}.unlisted_sequence TO {} WITH GRANT OPTION", quote(&names.login_a))).await?;
        let pool = fixture.login(&names.login_a).await?;
        let mut policy = AuthorityPolicyBuilder {
            discovery: DiscoveryScope::Schemas(vec![Identifier::new(&names.schema_a)?]),
            ..AuthorityPolicyBuilder::default()
        };
        policy.defaults.sequences.privileges.push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
        policy.defaults.allow_row_type_public_usage = true;
        let report = inspect(&pool, &policy).await?;
        require(has(&report, FindingKind::Privilege, "unlisted_records", ObjectPrivilege::Delete)
            && has(&report, FindingKind::Privilege, "\"unlisted_records\".\"id\"", ObjectPrivilege::Update)
            && has(&report, FindingKind::GrantOption, "unlisted_sequence", ObjectPrivilege::Select),
            "discovery skipped an unlisted relation, column, or sequence grant option")?;
        require(!has(&report, FindingKind::Privilege, "unlisted_definer", ObjectPrivilege::Execute),
            "an inaccessible definer was mistaken for reachable execution authority")?;
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {schema}.unlisted_records"))).execute(&pool).await?;
        require(sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {schema}.unlisted_definer()"))).execute(&pool).await.is_err(),
            "ungranted definer unexpectedly executable")?;
        policy.relations.push(RelationPolicy {
            relation: QualifiedName::new(&names.schema_a, "unlisted_records")?,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Delete, false)],
            columns: Vec::new(), allow_owner: false, allow_row_type_public_usage: true,
        });
        let allowed = inspect(&pool, &policy).await?;
        require(!has(&allowed, FindingKind::Privilege, "unlisted_records", ObjectPrivilege::Delete)
            && has(&allowed, FindingKind::Privilege, "\"unlisted_records\".\"id\"", ObjectPrivilege::Update),
            "exact relation allowance either failed or erased a forbidden column grant")?;
        exec(&mut fixture.admin, format!("GRANT EXECUTE ON FUNCTION {schema}.unlisted_definer() TO PUBLIC")).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {schema}.unlisted_definer()"))).execute(&pool).await?;
        require(has(&inspect(&pool, &policy).await?, FindingKind::Privilege,
            "unlisted_definer", ObjectPrivilege::Execute), "new PUBLIC definer authority escaped discovery")?;
        exec(&mut fixture.admin, format!("ALTER TABLE {schema}.unlisted_records OWNER TO {}", quote(&names.login_a))).await?;
        require(inspect(&pool, &policy).await?.findings().iter().any(|item| item.kind == FindingKind::Ownership
            && item.object.as_deref().is_some_and(|name| name.contains("unlisted_records"))),
            "unlisted relation ownership escaped discovery")?;
        Ok(())
    })).await
}
