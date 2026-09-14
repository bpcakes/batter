use super::VerificationPolicy;
use crate::support::{Result, require};
use crate::verify_policy as verify;
use crate::{AuthorityFixture, Names, exact_finding, exec, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, Identifier, ObjectPrivilege, ParameterName, ParameterPolicy,
    PublicGrant, PublicObject, RequiredSurface, VerificationStatus,
};
use sqlx::PgPool;

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub(super) async fn check(
    fixture: &mut AuthorityFixture,
    names: &Names,
    login_a: &PgPool,
    broad: &VerificationPolicy,
) -> Result<VerificationPolicy> {
    // The fixture-owned custom name is intentionally absent from
    // pg_settings. A
    // cluster ACL for an unrelated role must not conceal that every
    // login can SET a syntactically valid custom placeholder.
    exec(
        &mut fixture.admin,
        "SELECT pg_catalog.pg_advisory_lock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    exec(
        &mut fixture.admin,
        format!(
            "GRANT SET ON PARAMETER {} TO {}",
            names.hidden_parameter,
            quote(&names.unrelated_creator)
        ),
    )
    .await?;
    let long_parameter = format!("{}.{}", names.hidden_parameter, "x".repeat(63));
    exec(
        &mut fixture.admin,
        format!(
            "GRANT SET ON PARAMETER {long_parameter} TO {}",
            quote(&names.unrelated_creator)
        ),
    )
    .await?;
    exec(
        &mut fixture.admin,
        "SELECT pg_catalog.pg_advisory_unlock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    exec(
        login_a,
        format!("SET {} = 'observed'", names.hidden_parameter),
    )
    .await?;
    exec(login_a, format!("RESET {}", names.hidden_parameter)).await?;
    let mut hidden_default_denied = broad.clone();
    hidden_default_denied
        .authority
        .parameters
        .push(ParameterPolicy {
            parameter: ParameterName::new(&names.hidden_parameter)?,
            privileges: Vec::new(),
        });
    let hidden_default_report = verify(login_a, &hidden_default_denied).await?;
    require(
        exact_finding(
            &hidden_default_report,
            FindingKind::ProtectedParameter,
            Some(&names.hidden_parameter),
            Some(&names.login_a),
            Some(ObjectPrivilege::Set),
        ),
        "an unrelated ACL concealed implicit custom-parameter SET authority",
    )?;
    require(
        !hidden_default_report
            .findings()
            .iter()
            .any(|finding| finding.object.as_deref() == Some(&long_parameter)),
        "an unrelated long custom-parameter ACL became login authority",
    )?;

    exec(
        &mut fixture.admin,
        format!(
            "GRANT SET ON PARAMETER {long_parameter} TO {}",
            quote(&names.login_a)
        ),
    )
    .await?;

    // Add direct, PUBLIC and grant-option authority to exercise
    // their independent catalog paths.
    exec(
        &mut fixture.admin,
        "SELECT pg_catalog.pg_advisory_lock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    exec(
        &mut fixture.admin,
        format!(
            "GRANT SET ON PARAMETER {} TO {} WITH GRANT OPTION",
            names.hidden_parameter,
            quote(&names.login_a)
        ),
    )
    .await?;
    exec(
        &mut fixture.admin,
        format!(
            "GRANT SET ON PARAMETER {} TO PUBLIC",
            names.hidden_parameter
        ),
    )
    .await?;
    exec(
        &mut fixture.admin,
        "SELECT pg_catalog.pg_advisory_unlock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    let hidden_denied = verify(login_a, broad).await?;
    require(
        hidden_denied.findings().iter().any(|item| {
            item.kind == FindingKind::ProtectedParameter
                && item.object.as_deref() == Some(names.hidden_parameter.as_str())
        }),
        "an undeclared hidden parameter ACL was silently accepted",
    )?;
    require(
        exact_finding(
            &hidden_denied,
            FindingKind::ProtectedParameter,
            Some(&names.hidden_parameter),
            Some(&names.login_a),
            Some(ObjectPrivilege::Set),
        ),
        "hidden parameter login ACL was not reported exactly",
    )?;
    require(
        exact_finding(
            &hidden_denied,
            FindingKind::ProtectedParameter,
            Some(&names.hidden_parameter),
            Some("public"),
            Some(ObjectPrivilege::Set),
        ),
        "hidden parameter PUBLIC ACL was not reported exactly",
    )?;
    require(
        exact_finding(
            &hidden_denied,
            FindingKind::GrantOption,
            Some(&names.hidden_parameter),
            Some(&names.login_a),
            Some(ObjectPrivilege::Set),
        ),
        "hidden parameter grant option was not reported exactly",
    )?;
    require(
        !hidden_denied.findings().iter().any(|item| {
            item.kind == FindingKind::MissingObject
                && item.object.as_deref() == Some(names.hidden_parameter.as_str())
        }),
        "a catalog-visible hidden parameter was reported as missing",
    )?;
    let mut hidden_allowed = broad.clone();
    hidden_allowed.authority.parameters.push(ParameterPolicy {
        parameter: ParameterName::new(&names.hidden_parameter)?,
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, true)],
    });
    hidden_allowed.authority.parameters.push(ParameterPolicy {
        parameter: ParameterName::new(long_parameter.clone())?,
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
    });
    hidden_allowed.authority.public_grants.push(PublicGrant {
        object: PublicObject::Parameter(ParameterName::new(&names.hidden_parameter)?),
        privilege: AllowedPrivilege::new(ObjectPrivilege::Set, false),
    });
    let hidden_report = verify(login_a, &hidden_allowed).await?;
    require(
        hidden_report.status() == VerificationStatus::WithinDeclaredPolicy,
        "declared hidden parameter ACL was not accepted",
    )?;
    exec(
        &mut fixture.admin,
        "SELECT pg_catalog.pg_advisory_lock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    let revoke_hidden = exec(
        &mut fixture.admin,
        format!(
            "REVOKE ALL ON PARAMETER {} FROM PUBLIC",
            names.hidden_parameter
        ),
    )
    .await;
    let unlock_hidden = exec(
        &mut fixture.admin,
        "SELECT pg_catalog.pg_advisory_unlock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await;
    revoke_hidden?;
    unlock_hidden?;

    let mut complete = hidden_allowed.clone();
    complete.authority.required_surfaces = vec![RequiredSurface::SecurityDefinerBody];
    complete.authority.routines[0].allow_security_definer = true;
    let report = verify(login_a, &complete).await?;
    require(
        report.status() == VerificationStatus::Incomplete,
        "a requested unsupported routine surface was reported as complete",
    )?;

    exec(
        &mut fixture.admin,
        format!(
            "GRANT pg_read_all_data TO {} WITH INHERIT TRUE, SET FALSE, ADMIN FALSE",
            quote(&names.login_a)
        ),
    )
    .await?;
    let predefined_denied = verify(login_a, &hidden_allowed).await?;
    require(
        predefined_denied.findings().iter().any(|item| {
            item.kind == FindingKind::RoleAttribute
                && item.object.as_deref() == Some("pg_read_all_data")
        }),
        "an inherited predefined role capability was silently accepted",
    )?;
    require(
        exact_finding(
            &predefined_denied,
            FindingKind::RoleAttribute,
            Some("pg_read_all_data"),
            None,
            None,
        ),
        "the inherited predefined role oracle was not exact",
    )?;
    let mut predefined_allowed = hidden_allowed.clone();
    predefined_allowed
        .authority
        .roles
        .allowed_predefined_roles
        .push(Identifier::new("pg_read_all_data")?);
    let predefined_report = verify(login_a, &predefined_allowed).await?;
    require(
        predefined_report.status() == VerificationStatus::WithinDeclaredPolicy,
        "an explicitly allowed predefined role was rejected",
    )?;
    crate::verification_live_public::check_row_type_grants(
        fixture,
        names,
        login_a,
        &predefined_allowed,
    )
    .await?;
    Ok(predefined_allowed)
}
