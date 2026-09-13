use super::support::{Result, require};
use super::verification_live_discovery_scale::{clean_policy, inspect};
use super::{AuthorityFixture, exec, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, PublicAllowance, PublicObject, QualifiedName,
    TypePolicy,
};

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_cross_schema_aliases_preserve_scope_and_exact_policy() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let selected = quote(&names.schema_b);
        let source = format!("{}.source_range", quote(&names.schema_a));
        let alias = format!("{selected}.\"visible.spans\"");
        let role = quote(&names.login_b);
        exec(&mut fixture.admin, format!("DROP SCHEMA {selected} CASCADE")).await?;
        exec(&mut fixture.admin, format!("CREATE SCHEMA {selected}")).await?;
        exec(&mut fixture.admin, format!("GRANT USAGE ON SCHEMA {selected} TO {role}")).await?;
        exec(&mut fixture.admin, format!("CREATE TYPE {source} AS RANGE (SUBTYPE = int4, MULTIRANGE_TYPE_NAME = {alias})")).await?;
        exec(&mut fixture.admin, format!("REVOKE USAGE ON TYPE {source} FROM PUBLIC")).await?;
        let pool = fixture.login(&names.login_b).await?;
        let mut policy = clean_policy(&names.schema_b)?;
        require(inspect(&pool, &policy).await?.is_within_declared_policy(), "revoked cross-schema alias was not clean")?;
        exec(&mut fixture.admin, format!("GRANT USAGE ON TYPE {source} TO {role}")).await?;
        let native: bool = sqlx::query_scalar("SELECT pg_catalog.has_type_privilege(current_user, $1, 'USAGE')")
            .bind(&alias).fetch_one(&pool).await?;
        require(native, "native source grant did not authorize the cross-schema multirange")?;
        let denied = inspect(&pool, &policy).await?;
        require(denied.findings().iter().any(|finding| finding.kind == FindingKind::Privilege
            && finding.object.as_deref() == Some(alias.as_str())
            && finding.subject.as_deref() == Some(names.login_b.as_str())),
            "selected multirange vanished when its ACL source lived outside the scope")?;
        // Source names must not leak into the selected scope's policy identities.
        require(!denied.findings().iter().any(|finding| finding.object.as_deref() == Some(source.as_str())),
            "ACL source was confused with the selected policy object")?;
        policy.defaults.types.privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)];
        require(inspect(&pool, &policy).await?.is_within_declared_policy(), "default allowance did not cover derived type identities")?;
        let object = QualifiedName::new(&names.schema_b, "visible.spans")?;
        policy.types.push(TypePolicy { type_name: object.clone(), privileges: Vec::new(), allow_owner: false });
        let denied = inspect(&pool, &policy).await?;
        require(denied.findings().len() == 1 && denied.findings()[0].object.as_deref() == Some(alias.as_str()),
            "an exact alias denial was merged away with a permissive source or array policy")?;
        policy.types.clear();
        exec(&mut fixture.admin, format!("GRANT USAGE ON TYPE {source} TO PUBLIC")).await?;
        let public = inspect(&pool, &policy).await?;
        require(public.findings().iter().any(|finding| finding.object.as_deref() == Some(alias.as_str())
            && finding.subject.as_deref() == Some("public")), "cross-schema PUBLIC authority was omitted")?;
        policy.defaults.types.public_privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)];
        policy.public_overrides.push(PublicAllowance { object: PublicObject::Type(object), privileges: Vec::new() });
        let denied = inspect(&pool, &policy).await?;
        require(denied.findings().len() == 1 && denied.findings()[0].object.as_deref() == Some(alias.as_str())
            && denied.findings()[0].subject.as_deref() == Some("public"), "exact PUBLIC alias policy was not preserved")?;
        pool.close().await;
        Ok(())
    })).await
}
