use crate::support::{Result, require};
use crate::verify_policy as verify;
use crate::{AuthorityFixture, Names, exact_finding, exec, finding, names_policy, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, PublicObject, RolePolicy, VerificationPolicy,
    VerificationStatus,
};
use sqlx::PgPool;

#[allow(clippy::too_many_lines)]
pub(super) async fn check(
    fixture: &mut AuthorityFixture,
    names: &Names,
    login_a: &PgPool,
    predefined_allowed: &VerificationPolicy,
    table_object: &str,
) -> Result {
    exec(
        &mut fixture.admin,
        format!(
            "GRANT {} TO {} WITH INHERIT TRUE, SET FALSE, ADMIN FALSE",
            quote(&names.owner),
            quote(&names.login_a)
        ),
    )
    .await?;
    exec(login_a, "RESET ROLE".to_owned()).await?;
    let table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} (id, visible, secret) VALUES (2, 'owner', 'owner')"
    )))
    .execute(login_a)
    .await?;
    let owner_denied = verify(login_a, predefined_allowed).await?;
    require(
        exact_finding(
            &owner_denied,
            FindingKind::Ownership,
            Some(table_object),
            Some(&names.owner),
            None,
        ),
        "inherited owner authority was not reported exactly",
    )?;
    let mut owner_allowed = predefined_allowed.clone();
    owner_allowed.authority.public_grants.retain(|grant| {
        !matches!(
            &grant.object,
            PublicObject::Relation(relation)
                if relation.schema() == names.schema_b && relation.name() == names.table_b
        )
    });
    for relation in &mut owner_allowed.authority.relations {
        relation.allow_owner = true;
    }
    for sequence in &mut owner_allowed.authority.sequences {
        sequence.allow_owner = true;
    }
    for schema in &mut owner_allowed.authority.schemas {
        schema.allow_owner = true;
    }
    for type_policy in &mut owner_allowed.authority.types {
        type_policy.allow_owner = true;
    }
    for routine in &mut owner_allowed.authority.routines {
        routine.allow_owner = true;
    }
    owner_allowed.authority.database.allow_owner = true;
    let owner_report = verify(login_a, &owner_allowed).await?;
    require(
        owner_report.status() == VerificationStatus::WithinDeclaredPolicy,
        "inherited group ownership was not accepted by allow_owner",
    )?;

    exec(
        &mut fixture.admin,
        format!(
            "REVOKE SELECT ON {}.{} FROM PUBLIC",
            quote(&names.schema_a),
            quote(&names.table_a)
        ),
    )
    .await?;
    let login_b = fixture.login(&names.login_b).await?;
    let policy_b = names_policy(
        names,
        &names.schema_b,
        &names.ledger_b,
        &names.table_b,
        RolePolicy::default(),
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
        true,
        false,
        false,
        false,
        false,
    )?;
    let report = verify(&login_b, &policy_b).await?;
    require(
        report.status() == VerificationStatus::WithinDeclaredPolicy,
        "second independent fixture policy was not accepted",
    )?;
    require(
        report.session_user() == names.login_b,
        "second fixture used the wrong login",
    )?;
    require(
        !finding(&report, FindingKind::RoleAdminOption)
            && !report.findings().iter().any(|item| {
                item.kind == FindingKind::RoleAttribute
                    && item.object.as_deref() == Some(names.unrelated_creator.as_str())
            }),
        "an unrelated CREATEROLE role was treated as login_b ADMIN authority",
    )?;
    exec(
        &mut fixture.admin,
        format!(
            "ALTER TABLE {}.{} OWNER TO {}",
            quote(&names.schema_b),
            quote(&names.table_b),
            quote(&names.login_b),
        ),
    )
    .await?;
    let report = verify(&login_b, &policy_b).await?;
    require(
        finding(&report, FindingKind::Ownership),
        "reachable object ownership was not observed",
    )?;
    let mut owner_policy = policy_b.clone();
    owner_policy.authority.relations[0].allow_owner = true;
    for type_policy in &mut owner_policy.authority.types {
        type_policy.allow_owner = true;
    }
    let report = verify(&login_b, &owner_policy).await?;
    require(
        report.status() == VerificationStatus::WithinDeclaredPolicy,
        "explicit owner allowance did not accept the owned fixture",
    )?;

    crate::verification_live_public::check_public_routine(fixture, names, login_a, &owner_allowed)
        .await?;
    Ok(())
}
