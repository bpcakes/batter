use crate::support::{Result, require};
use crate::verify_policy as verify;
use crate::{AuthorityFixture, Names, exact_finding, exec, finding, names_policy, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, Identifier, ObjectPrivilege, RoleAttribute, RolePolicy,
    RoutinePolicy, RoutineSignature, RoutineType, VerificationPolicy, VerificationStatus,
};
use sqlx::PgPool;

#[allow(clippy::too_many_lines)]
pub(super) async fn check(
    fixture: &mut AuthorityFixture,
    names: &Names,
    login_a: &PgPool,
    narrow: &VerificationPolicy,
) -> Result<VerificationPolicy> {
    let set_role = format!("SET ROLE {}", quote(&names.settable));
    sqlx::query(sqlx::AssertSqlSafe(set_role))
        .execute(login_a)
        .await?;
    let report = verify(login_a, narrow).await?;
    require(
        report.session_user() == names.login_a,
        "SET ROLE changed session_user",
    )?;
    require(
        report.current_user() == names.settable,
        "SET ROLE did not change current_user",
    )?;
    require(
        finding(&report, FindingKind::RoleAdminOption),
        "session-rooted admin reachability disappeared under SET ROLE",
    )?;

    exec(
        &mut fixture.admin,
        format!("ALTER ROLE {} CREATEDB", quote(&names.inherited)),
    )
    .await?;
    let report = verify(login_a, narrow).await?;
    require(
        !report.findings().iter().any(|item| {
            item.kind == FindingKind::RoleAttribute
                && item.object.as_deref() == Some(names.inherited.as_str())
        }),
        "an inherited-only role attribute was treated as active",
    )?;
    exec(
        &mut fixture.admin,
        format!(
            "GRANT {} TO {} WITH INHERIT FALSE, SET TRUE, ADMIN FALSE",
            quote(&names.inherited),
            quote(&names.login_a),
        ),
    )
    .await?;
    let report = verify(login_a, narrow).await?;
    require(
        report.findings().iter().any(|item| {
            item.kind == FindingKind::RoleAttribute
                && item.object.as_deref() == Some(names.inherited.as_str())
                && item.role_attribute == Some(RoleAttribute::CreateDatabase)
        }),
        "a SET-reachable role attribute was not observed",
    )?;

    let broad = names_policy(
        names,
        &names.schema_a,
        &names.ledger_a,
        &names.table_a,
        RolePolicy {
            allowed_admin_roles: vec![Identifier::new(&names.admin_target)?],
            allow_create_database: true,
            ..RolePolicy::default()
        },
        vec![
            AllowedPrivilege::new(ObjectPrivilege::Select, true),
            AllowedPrivilege::new(ObjectPrivilege::Insert, true),
            AllowedPrivilege::new(ObjectPrivilege::Update, true),
            AllowedPrivilege::new(ObjectPrivilege::Delete, true),
        ],
        true,
        true,
        true,
        false,
        true,
    )?;
    let report = verify(login_a, &broad).await?;
    require(
        report.status() == VerificationStatus::WithinDeclaredPolicy,
        "matching policy did not accept declared authority",
    )?;
    require(
        !report
            .findings()
            .iter()
            .any(|item| item.kind == FindingKind::RoleAttribute),
        "inherited-only role attributes were treated as inherited",
    )?;
    require(
        !report.findings().iter().any(|finding| {
            finding.object.as_deref() == Some(names.superuser_target.as_str())
                && (finding.kind == FindingKind::RoleAdminOption
                    || finding.role_attribute == Some(RoleAttribute::Superuser))
        }),
        "an unusable ADMIN edge promoted a superuser target",
    )?;
    sqlx::query("RESET ROLE").execute(login_a).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT {} TO {} WITH INHERIT FALSE, SET FALSE, ADMIN FALSE",
        quote(&names.admin_target),
        quote(&names.login_b),
    )))
    .execute(login_a)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "REVOKE {} FROM {}",
        quote(&names.admin_target),
        quote(&names.login_b),
    )))
    .execute(login_a)
    .await?;
    let denied_grant = sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT {} TO {} WITH INHERIT TRUE, SET TRUE, ADMIN FALSE",
        quote(&names.superuser_target),
        quote(&names.login_b),
    )))
    .execute(login_a)
    .await;
    require(
        denied_grant
            .as_ref()
            .err()
            .and_then(sqlx::Error::as_database_error)
            .and_then(|error| error.code())
            .as_deref()
            == Some("42501"),
        "the ADMIN-only login unexpectedly granted a superuser role",
    )?;
    let denied_set = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET ROLE {}",
        quote(&names.superuser_target)
    )))
    .execute(login_a)
    .await;
    require(
        denied_set
            .as_ref()
            .err()
            .and_then(sqlx::Error::as_database_error)
            .and_then(|error| error.code())
            .as_deref()
            == Some("42501"),
        "the ADMIN-only login unexpectedly set an ungranted superuser role",
    )?;

    exec(
        &mut fixture.admin,
        format!(
            "GRANT pg_read_all_data TO {} WITH INHERIT FALSE, SET FALSE, ADMIN TRUE",
            quote(&names.login_a)
        ),
    )
    .await?;
    let report = verify(login_a, &broad).await?;
    require(
        exact_finding(
            &report,
            FindingKind::RoleAdminOption,
            Some("pg_read_all_data"),
            None,
            None,
        ),
        "an undeclared ADMIN target was hidden by another allowed target",
    )?;
    let mut predefined_admin_allowed = broad.clone();
    predefined_admin_allowed
        .authority
        .roles
        .allowed_admin_roles
        .push(Identifier::new("pg_read_all_data")?);
    let report = verify(login_a, &predefined_admin_allowed).await?;
    require(
        !report.findings().iter().any(|finding| {
            finding.kind == FindingKind::RoleAdminOption
                && finding.object.as_deref() == Some("pg_read_all_data")
        }),
        "an explicitly allowed ADMIN target remained denied",
    )?;
    require(
        report.findings().iter().any(|finding| {
            finding.kind == FindingKind::RoleAttribute
                && finding.object.as_deref() == Some("pg_read_all_data")
                && finding.role_attribute == Some(RoleAttribute::PredefinedRole)
        }),
        "an allowed ADMIN target concealed the role it can grant back to the login",
    )?;
    exec(
        &mut fixture.admin,
        format!("REVOKE pg_read_all_data FROM {}", quote(&names.login_a)),
    )
    .await?;

    crate::verification_live_public::check_public_relation_columns(fixture, names, login_a, &broad)
        .await?;

    let mut catalog_types = broad.clone();
    catalog_types.authority.routines.extend([
        RoutinePolicy {
            routine: RoutineSignature::new(
                &names.schema_a,
                &names.routine_a,
                [RoutineType::array("pg_catalog", "int4")?],
            )?,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)],
            allow_owner: false,
            allow_security_definer: false,
        },
        RoutinePolicy {
            routine: RoutineSignature::new(
                &names.schema_a,
                &names.routine_a,
                [RoutineType::new("pg_catalog", "name")?],
            )?,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)],
            allow_owner: false,
            allow_security_definer: false,
        },
        RoutinePolicy {
            routine: RoutineSignature::new(
                &names.schema_a,
                &names.routine_a,
                [RoutineType::new("pg_catalog", "oidvector")?],
            )?,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)],
            allow_owner: false,
            allow_security_definer: false,
        },
        RoutinePolicy {
            routine: RoutineSignature::new(
                &names.schema_a,
                &names.routine_a,
                [RoutineType::array("pg_catalog", "oid")?],
            )?,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)],
            allow_owner: false,
            allow_security_definer: false,
        },
    ]);
    let catalog_type_report = verify(login_a, &catalog_types).await?;
    require(
        catalog_type_report.status() == VerificationStatus::WithinDeclaredPolicy,
        "true arrays or subscriptable vector types were misidentified",
    )?;

    let mut work_mem_undeclared = broad.clone();
    work_mem_undeclared
        .authority
        .parameters
        .retain(|entry| entry.parameter.as_str() != "work_mem");
    let work_mem_report = verify(login_a, &work_mem_undeclared).await?;
    require(
        !work_mem_report.findings().iter().any(|item| {
            item.kind == FindingKind::ProtectedParameter
                && item.object.as_deref() == Some("work_mem")
                && item.subject.as_deref() == Some(names.login_a.as_str())
        }),
        "an unrelated explicit ACL exposed undeclared context-default authority",
    )?;
    Ok(broad)
}
