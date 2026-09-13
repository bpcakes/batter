use super::support::{Result, require};
use super::{AuthorityFixture, Names, exec, quote};
use crate::verify_policy as verify;
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, PublicObject, QualifiedName, TypePolicy,
    VerificationPolicy, VerificationStatus,
};
use sqlx::PgPool;

pub(crate) async fn check_public_relation_columns(
    fixture: &mut AuthorityFixture,
    names: &Names,
    login_a: &PgPool,
    policy: &VerificationPolicy,
) -> Result {
    // `table_b` is selected only by a relation-level PUBLIC allowance. A
    // column ACL on that selected relation must still be inspected.
    exec(
        &mut fixture.admin,
        format!(
            "GRANT UPDATE ({}) ON {}.{} TO {}",
            quote("visible"),
            quote(&names.schema_b),
            quote(&names.table_b),
            quote(&names.login_a),
        ),
    )
    .await?;
    let report = verify(login_a, policy).await?;
    require(
        report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.subject.as_deref() == Some(names.login_a.as_str())
                && item.privilege == Some(ObjectPrivilege::Update)
                && item.object.as_deref().is_some_and(|object| {
                    object.contains(&names.table_b) && object.ends_with(".\"visible\"")
                })
        }),
        "a column ACL was skipped on a relation selected only by PUBLIC policy",
    )?;
    exec(
        &mut fixture.admin,
        format!(
            "REVOKE UPDATE ({}) ON {}.{} FROM {}",
            quote("visible"),
            quote(&names.schema_b),
            quote(&names.table_b),
            quote(&names.login_a),
        ),
    )
    .await
}

pub(crate) async fn check_row_type_grants(
    fixture: &mut AuthorityFixture,
    names: &Names,
    login_a: &PgPool,
    broad: &VerificationPolicy,
) -> Result {
    // Composite row types have default PUBLIC USAGE. The relation policy's
    // explicit allowance accepts that default, while removing the allowance
    // must expose the exact PUBLIC ACL.
    let mut row_type_denied = broad.clone();
    for relation in &mut row_type_denied.authority.relations {
        if relation.relation.schema() == names.schema_a && relation.relation.name() == names.table_a
        {
            relation.allow_row_type_public_usage = false;
        }
    }
    let row_type_report = verify(login_a, &row_type_denied).await?;
    require(
        row_type_report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.subject.as_deref() == Some("public")
                && item.privilege == Some(ObjectPrivilege::Usage)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.table_a))
        }),
        "default PUBLIC composite-row-type USAGE was not checked",
    )?;

    exec(
        &mut fixture.admin,
        format!(
            "GRANT USAGE ON TYPE {}.{} TO {}",
            quote(&names.schema_a),
            quote(&names.table_a),
            quote(&names.login_a),
        ),
    )
    .await?;
    let direct_row_type_report = verify(login_a, broad).await?;
    require(
        direct_row_type_report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.subject.as_deref() == Some(names.login_a.as_str())
                && item.privilege == Some(ObjectPrivilege::Usage)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.table_a))
        }),
        "a direct composite-row-type grant was hidden by its relation policy",
    )?;
    let mut explicit_row_type = broad.clone();
    explicit_row_type.authority.types.push(TypePolicy {
        type_name: QualifiedName::new(&names.schema_a, &names.table_a)?,
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
        allow_owner: false,
    });
    let explicit_row_type_report = verify(login_a, &explicit_row_type).await?;
    require(
        explicit_row_type_report.status() == VerificationStatus::WithinDeclaredPolicy,
        "an explicit row-type policy did not compose with the relation PUBLIC allowance",
    )?;
    exec(
        &mut fixture.admin,
        format!(
            "REVOKE USAGE ON TYPE {}.{} FROM {}",
            quote(&names.schema_a),
            quote(&names.table_a),
            quote(&names.login_a),
        ),
    )
    .await?;

    Ok(())
}

pub(crate) async fn check_public_routine(
    fixture: &mut AuthorityFixture,
    names: &Names,
    login_a: &PgPool,
    broad: &VerificationPolicy,
) -> Result {
    // A PUBLIC-only executable SECURITY DEFINER routine must be visible even
    // when no reachable login role has EXECUTE.
    exec(
        &mut fixture.admin,
        format!(
            "ALTER FUNCTION {}.{}(integer) SECURITY DEFINER",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    exec(
        &mut fixture.admin,
        format!(
            "GRANT EXECUTE ON FUNCTION {}.{}(integer) TO PUBLIC",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    let public_denied = {
        let mut policy = broad.clone();
        policy
            .authority
            .public_grants
            .retain(|grant| !matches!(grant.object, PublicObject::Routine(_)));
        policy
    };
    let public_report = verify(login_a, &public_denied).await?;
    require(
        public_report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.subject.as_deref() == Some("public")
                && item.privilege == Some(ObjectPrivilege::Execute)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.routine_a))
        }),
        "PUBLIC-only routine EXECUTE was not observed",
    )?;
    let public_allowed_report = verify(login_a, broad).await?;
    require(
        public_allowed_report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.privilege == Some(ObjectPrivilege::Execute)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.routine_a))
        }),
        "PUBLIC SECURITY DEFINER execution was silently accepted",
    )?;
    exec(
        &mut fixture.admin,
        format!(
            "REVOKE EXECUTE ON FUNCTION {}.{}(integer) FROM PUBLIC",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    exec(
        &mut fixture.admin,
        format!(
            "REVOKE EXECUTE ON FUNCTION {}.{}(integer) FROM {}",
            quote(&names.schema_a),
            quote(&names.routine_a),
            quote(&names.login_a),
        ),
    )
    .await?;
    let owner_only_report = verify(login_a, broad).await?;
    require(
        owner_only_report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.subject.is_none()
                && item.privilege == Some(ObjectPrivilege::Execute)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.routine_a))
        }),
        "allowed owner execution concealed a SECURITY DEFINER routine",
    )?;
    Ok(())
}
