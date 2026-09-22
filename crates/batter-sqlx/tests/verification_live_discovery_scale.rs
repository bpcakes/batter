use super::support::{Result, require};
use super::{AuthorityFixture, exec, quote};
use batter_core::operation::OperationContext;
use batter_sqlx::verification::{
    AllowedPrivilege, AuthorityPolicyBuilder, DatabasePolicy, DiscoveryScope, FindingKind,
    Identifier, ObjectPrivilege, ParameterName, ParameterPolicy, PublicGrant, PublicObject,
    QualifiedName, RequiredPrivilege, TypePolicy, VerificationReport, verify_authority,
};
use std::time::Duration;

pub(super) async fn inspect(
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

pub(super) fn clean_policy(schema: &str) -> Result<AuthorityPolicyBuilder> {
    let privileges = vec![
        AllowedPrivilege::new(ObjectPrivilege::Connect, false),
        AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
    ];
    let mut policy = AuthorityPolicyBuilder {
        discovery: DiscoveryScope::Schemas(vec![Identifier::new(schema)?]),
        database: DatabasePolicy {
            privileges: privileges.clone(),
            allow_owner: false,
        },
        parameters: vec![ParameterPolicy {
            parameter: ParameterName::new("work_mem")?,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
        }],
        public_grants: privileges
            .into_iter()
            .map(|privilege| PublicGrant {
                object: PublicObject::Database,
                privilege,
            })
            .collect(),
        ..AuthorityPolicyBuilder::default()
    };
    policy
        .defaults
        .schemas
        .privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Usage, false));
    policy
        .defaults
        .relations
        .privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    policy.defaults.allow_row_type_public_usage = true;
    policy.defaults.invoker_routines.privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)];
    policy.defaults.invoker_routines.public_privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)];
    Ok(policy)
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_discovery_dependent_types_follow_native_acl() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let schema = quote(&names.schema_b);
        let role = quote(&names.login_b);
        // Rebuild one fixture-owned schema with a deliberately clean manifest.
        // The fixture's finalizer still owns the schema and login cleanup.
        exec(&mut fixture.admin, format!("DROP SCHEMA {schema} CASCADE")).await?;
        exec(&mut fixture.admin, format!("CREATE SCHEMA {schema}")).await?;
        exec(&mut fixture.admin, format!("GRANT USAGE ON SCHEMA {schema} TO {role}")).await?;
        exec(&mut fixture.admin, format!("CREATE TABLE {schema}.records (id integer)")).await?;
        exec(&mut fixture.admin, format!("GRANT SELECT ON {schema}.records TO {role}")).await?;
        exec(&mut fixture.admin, format!("CREATE TYPE {schema}.record_kind AS ENUM ('ready')")).await?;
        exec(&mut fixture.admin, format!("REVOKE USAGE ON TYPE {schema}.record_kind FROM PUBLIC")).await?;
        exec(&mut fixture.admin, format!("CREATE TYPE {schema}.record_range AS RANGE (subtype = integer)")).await?;
        exec(&mut fixture.admin, format!("REVOKE USAGE ON TYPE {schema}.record_range FROM PUBLIC")).await?;
        let pool = fixture.login(&names.login_b).await?;
        let policy = clean_policy(&names.schema_b)?;
        let clean = inspect(&pool, &policy).await?;
        require(clean.is_within_declared_policy() && clean.findings().is_empty(),
            "clean discovered schema was rejected because dependent types acquired invented default PUBLIC ACLs")?;
        let aliases: Vec<String> = sqlx::query_scalar(
            "SELECT a.typname::text FROM pg_catalog.pg_type t
             JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
             JOIN pg_catalog.pg_type a ON a.oid = t.typarray
             WHERE n.nspname = $1 AND t.typname IN ('record_kind', 'record_range')
             UNION ALL
             SELECT m.typname::text FROM pg_catalog.pg_type t
             JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
             JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid
             JOIN pg_catalog.pg_type m ON m.oid IN (r.rngmultitypid, (SELECT typarray FROM pg_catalog.pg_type WHERE oid = r.rngmultitypid))
             WHERE n.nspname = $1 AND t.typname = 'record_range'"
        ).bind(&names.schema_b).fetch_all(&mut fixture.admin).await?;
        require(aliases.len() == 4, "fixture did not create enum/range arrays and multirange aliases")?;
        let mut declared = clean_policy(&names.schema_b)?;
        declared.discovery = DiscoveryScope::Declared;
        for alias in &aliases {
            let object = QualifiedName::new(&names.schema_b, alias)?;
            let native: bool = sqlx::query_scalar("SELECT has_type_privilege(current_user, $1, 'USAGE')")
                .bind(format!("{schema}.{}", quote(alias))).fetch_one(&pool).await?;
            require(!native, "native dependent type unexpectedly retained PUBLIC usage after source revoke")?;
            declared.types.push(TypePolicy { type_name: object.clone(), privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)], allow_owner: false });
            declared.required_privileges.push(RequiredPrivilege { object: PublicObject::Type(object), privilege: ObjectPrivilege::Usage });
        }
        let revoked = inspect(&pool, &declared).await?;
        require(revoked.findings().iter().filter(|finding| finding.kind == FindingKind::MissingPrivilege).count() == aliases.len(),
            "explicit dependent type checks did not consult the source type's revoked ACL")?;
        exec(&mut fixture.admin, format!("GRANT USAGE ON TYPE {schema}.record_kind, {schema}.record_range TO {role}")).await?;
        for alias in &aliases {
            let native: bool = sqlx::query_scalar("SELECT has_type_privilege(current_user, $1, 'USAGE')")
                .bind(format!("{schema}.{}", quote(alias))).fetch_one(&pool).await?;
            require(native, "source grant did not authorize native dependent type usage")?;
        }
        require(inspect(&pool, &declared).await?.is_within_declared_policy(),
            "dependent types did not follow the source type's granted ACL")?;
        // Catalog vector identities remain explicitly selectable even though
        // PostgreSQL resolves their effective ACL through their element type.
        for vector in ["oidvector", "int2vector"] {
            let object = QualifiedName::new("pg_catalog", vector)?;
            declared.types.push(TypePolicy { type_name: object.clone(), privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)], allow_owner: false });
            declared.public_grants.push(PublicGrant { object: PublicObject::Type(object.clone()), privilege: AllowedPrivilege::new(ObjectPrivilege::Usage, false) });
            declared.required_privileges.push(RequiredPrivilege { object: PublicObject::Type(object), privilege: ObjectPrivilege::Usage });
            let native: bool = sqlx::query_scalar("SELECT has_type_privilege(current_user, $1, 'USAGE')")
                .bind(format!("pg_catalog.{vector}")).fetch_one(&pool).await?;
            require(native, "native vector type did not retain its effective USAGE")?;
        }
        require(inspect(&pool, &declared).await?.is_within_declared_policy(),
            "vector types were hidden or treated as automatic array aliases")?;
        Ok(())
    })).await
}
