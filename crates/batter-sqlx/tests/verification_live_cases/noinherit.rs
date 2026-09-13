use crate::support::{Result, require};
use crate::verify_policy as verify;
use crate::{AuthorityFixture, Names, finding, names_policy, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, RoleAttribute, RolePolicy,
};

pub(super) async fn check(fixture: &mut AuthorityFixture, names: &Names) -> Result {
    // A NOINHERIT login still receives an explicitly INHERIT edge
    // on PostgreSQL 18. The nested predefined role is therefore
    // effective even though SET ROLE is deliberately unavailable.
    let login_noinherit = fixture
        .login_with_options(&names.login_noinherit, [("ignore_system_indexes", "off")])
        .await?;
    let startup_setting: String = sqlx::query_scalar("SHOW ignore_system_indexes")
        .fetch_one(&login_noinherit)
        .await?;
    require(
        startup_setting == "off",
        "backend-context startup setting was not applied to the restricted login",
    )?;
    let noinherit_table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {noinherit_table} (id, visible, secret) VALUES (0, 'nested', 'nested')"
    )))
    .execute(&login_noinherit)
    .await?;
    let denied_set = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET ROLE {}",
        quote(&names.nested)
    )))
    .execute(&login_noinherit)
    .await;
    require(
        denied_set.is_err(),
        "the nested INHERIT-only membership unexpectedly allowed SET ROLE",
    )?;
    let noinherit_policy = names_policy(
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
    let noinherit_report = verify(&login_noinherit, &noinherit_policy).await?;
    require(
        noinherit_report.session_user() == names.login_noinherit
            && noinherit_report.current_user() == names.login_noinherit,
        "the NOINHERIT restricted login was not the checker session root",
    )?;
    require(
        noinherit_report.findings().iter().any(|item| {
            item.kind == FindingKind::RoleAttribute
                && item.object.as_deref() == Some("pg_write_all_data")
                && item.role_attribute == Some(RoleAttribute::PredefinedRole)
        }),
        "nested inherited predefined authority was hidden by the login INHERIT flag",
    )?;
    require(
        !finding(&noinherit_report, FindingKind::RoleAdminOption),
        "ADMIN authority crossed a membership with neither INHERIT nor SET",
    )?;
    Ok(())
}
