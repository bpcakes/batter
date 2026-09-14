use super::VerificationPolicy;
use crate::support::{Result, require};
use crate::verify_policy as verify;
use crate::{AuthorityFixture, Names, exact_finding, exec, finding, names_policy, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, ParameterName, PublicGrant, PublicObject,
    QualifiedName, RelationPolicy, RolePolicy, VerificationStatus,
};
use sqlx::PgPool;

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn check(
    fixture: &mut AuthorityFixture,
    names: &Names,
) -> Result<(PgPool, VerificationPolicy, String)> {
    let login_a = fixture.login(&names.login_a).await?;
    let table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} (id, visible, secret) VALUES (1, 'visible', 'secret')"
    )))
    .execute(&login_a)
    .await?;
    let selected: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT secret FROM {table} WHERE id = 1"
    )))
    .fetch_one(&login_a)
    .await?;
    require(
        selected == "secret",
        "direct SELECT and inherited INSERT did not work for the restricted login",
    )?;
    let denied_set = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET ROLE {}",
        quote(&names.inherited)
    )))
    .execute(&login_a)
    .await;
    require(
        denied_set.is_err(),
        "a SET FALSE membership was usable as SET ROLE",
    )?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET ROLE {}",
        quote(&names.settable)
    )))
    .execute(&login_a)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {table} SET visible = 'updated'"
    )))
    .execute(&login_a)
    .await?;
    let sequence_name = format!("{}.{}", names.schema_a, names.sequence_a);
    let _: i64 = sqlx::query_scalar("SELECT nextval($1::regclass)")
        .bind(sequence_name)
        .fetch_one(&login_a)
        .await?;
    sqlx::query("RESET ROLE").execute(&login_a).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET ROLE {}",
        quote(&names.admin_target)
    )))
    .execute(&login_a)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM {table} WHERE id = 1"
    )))
    .execute(&login_a)
    .await?;
    sqlx::query("RESET ROLE").execute(&login_a).await?;
    let narrow = names_policy(
        names,
        &names.schema_a,
        &names.ledger_a,
        &names.table_a,
        RolePolicy::default(),
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
        false,
        true,
        false,
        false,
        false,
    )?;
    let report = verify(&login_a, &narrow).await?;
    require(
        report.session_user() == names.login_a,
        "session_user was not rooted at the restricted login",
    )?;
    require(
        report.current_user() == names.login_a,
        "unexpected initial current_user",
    )?;
    require(
        report.status() == VerificationStatus::Violations,
        "narrow policy missed reachable authority",
    )?;
    let mut missing = narrow.clone();
    missing.authority.relations.push(RelationPolicy {
        relation: QualifiedName::new(&names.schema_a, "missing_relation")?,
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    });
    missing.authority.public_grants.push(PublicGrant {
        object: PublicObject::Parameter(ParameterName::new("missing_parameter")?),
        privilege: AllowedPrivilege::new(ObjectPrivilege::Set, false),
    });
    // Missing built-in names are knowably absent only under full settings visibility.
    exec(
        &mut fixture.admin,
        format!(
            "GRANT pg_read_all_settings TO {} WITH INHERIT TRUE, SET FALSE",
            quote(&names.login_a)
        ),
    )
    .await?;
    let report = verify(&login_a, &missing).await?;
    exec(
        &mut fixture.admin,
        format!("REVOKE pg_read_all_settings FROM {}", quote(&names.login_a)),
    )
    .await?;
    let table_object = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
    let sequence_object = format!("{}.{}", quote(&names.schema_a), quote(&names.sequence_a));
    let routine_object = format!(
        "{}.{}(\"pg_catalog\".\"int4\")",
        quote(&names.schema_a),
        quote(&names.routine_a)
    );
    require(
        report.findings().iter().any(|item| {
            item.kind == FindingKind::MissingObject
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains("missing_relation"))
        }),
        "a requested missing relation was silently accepted",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::MissingObject,
            Some("missing_parameter"),
            None,
            None,
        ),
        "a requested nonexistent built-in parameter was silently accepted",
    )?;
    require(
        !report.findings().iter().any(|item| {
            item.kind == FindingKind::MissingObject
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.ends_with(".\"visible\""))
        }),
        "a declared column without an explicit attacl was reported missing",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::Privilege,
            Some(&table_object),
            Some(&names.inherited),
            Some(ObjectPrivilege::Insert),
        ),
        "the inherited table INSERT oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::Privilege,
            Some(&table_object),
            Some(&names.settable),
            Some(ObjectPrivilege::Update),
        ),
        "the SET-reachable table UPDATE oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::Privilege,
            Some(&table_object),
            Some(&names.admin_target),
            Some(ObjectPrivilege::Delete),
        ),
        "the ADMIN-target table DELETE oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::GrantOption,
            Some(&table_object),
            Some(&names.settable),
            Some(ObjectPrivilege::Update),
        ),
        "the table UPDATE grant-option oracle was not observed",
    )?;
    let column_object = format!("{table_object}.\"secret\"");
    let mut column_denied = missing.clone();
    for relation in &mut column_denied.authority.relations {
        for column in &mut relation.columns {
            if column.column.as_str() == "secret" {
                column.privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
            }
        }
    }
    let column_report = verify(&login_a, &column_denied).await?;
    require(
        exact_finding(
            &column_report,
            FindingKind::GrantOption,
            Some(&column_object),
            Some(&names.login_a),
            Some(ObjectPrivilege::Select),
        ),
        "the column SELECT grant-option oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::ProtectedParameter,
            Some("work_mem"),
            Some(&names.login_a),
            Some(ObjectPrivilege::Set),
        ),
        "the context-backed work_mem oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::ProtectedParameter,
            Some("ignore_system_indexes"),
            Some(&names.login_a),
            Some(ObjectPrivilege::Set),
        ),
        "the backend startup parameter oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::Privilege,
            Some(&routine_object),
            Some(&names.login_a),
            Some(ObjectPrivilege::Execute),
        ),
        "the routine EXECUTE oracle was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::Privilege,
            Some(&sequence_object),
            Some(&names.inherited),
            Some(ObjectPrivilege::Usage),
        ),
        "the inherited sequence USAGE oracle was not observed",
    )?;
    require(
        finding(&report, FindingKind::RoleAdminOption),
        "ADMIN OPTION was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::RoleAdminOption,
            Some(&names.admin_target),
            None,
            None,
        ),
        "the ADMIN reachability oracle was not observed",
    )?;
    require(
        finding(&report, FindingKind::Privilege),
        "direct/inherited/SET table authority was not observed",
    )?;
    require(
        finding(&report, FindingKind::GrantOption),
        "table or column grant option was not observed",
    )?;
    require(
        finding(&report, FindingKind::ProtectedParameter),
        "protected parameter SET was not observed",
    )?;
    require(
        report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.privilege == Some(ObjectPrivilege::Execute)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.routine_a))
        }),
        "routine signature or EXECUTE authority was not observed",
    )?;
    require(
        report.findings().iter().any(|item| {
            item.kind == FindingKind::Privilege
                && item.privilege == Some(ObjectPrivilege::Usage)
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.sequence_a))
        }),
        "sequence mutation authority was not observed",
    )?;
    require(
        report.findings().iter().any(|item| {
            item.kind == FindingKind::GrantOption
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.ends_with(".\"secret\""))
        }),
        "column grant option was not observed",
    )?;
    let mut public_denied = narrow.clone();
    public_denied.authority.public_grants.retain(|grant| {
        !matches!(
            &grant.object,
            PublicObject::Relation(relation)
                if relation.schema() == names.schema_a && relation.name() == names.table_a
        )
    });
    let report = verify(&login_a, &public_denied).await?;
    require(
        report.findings().iter().any(|item| {
            item.subject.as_deref() == Some("public")
                && item
                    .object
                    .as_deref()
                    .is_some_and(|object| object.contains(&names.table_a))
        }),
        "PUBLIC relation authority was not observed",
    )?;
    require(
        exact_finding(
            &report,
            FindingKind::Privilege,
            Some(&table_object),
            Some("public"),
            Some(ObjectPrivilege::Select),
        ),
        "the PUBLIC relation SELECT oracle was not observed",
    )?;
    Ok((login_a, narrow, table_object))
}
