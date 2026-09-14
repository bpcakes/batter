//! Live acceptance scenarios for generic PostgreSQL verification.

use crate::verify_policy as verify;

use super::support::{Result, require};
use super::{AuthorityFixture, VerificationPolicy, exec, finding, names_policy, quote};
use batter_sqlx::verification::{FindingKind, RoleAttribute, RolePolicy};
use sqlx::postgres::PgPoolOptions;

mod narrow;
mod noinherit;
mod owners;
mod parameters;
mod roles;

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_uses_the_initial_authenticated_role() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture
        .run(move |fixture| {
            Box::pin(async move {
                let masked = PgPoolOptions::new()
                    .max_connections(1)
                    .connect(&fixture.url)
                    .await?;
                let (authenticated, superuser): (String, bool) = sqlx::query_as(
                    "SELECT session_user::text, rolsuper
                     FROM pg_catalog.pg_roles
                     WHERE rolname = session_user",
                )
                .fetch_one(&masked)
                .await?;
                require(
                    superuser,
                    "the administrative fixture login is not a superuser",
                )?;
                exec(
                    &masked,
                    format!("SET SESSION AUTHORIZATION {}", quote(&names.login_a)),
                )
                .await?;
                let policy = names_policy(
                    &names,
                    &names.schema_a,
                    &names.ledger_a,
                    &names.table_a,
                    RolePolicy::default(),
                    Vec::new(),
                    false,
                    true,
                    false,
                    false,
                    false,
                )?;
                let report = verify(&masked, &policy).await?;
                require(
                    report.session_user() == authenticated,
                    "changed session authorization concealed the authenticated role",
                )?;
                require(
                    report.current_user() == names.login_a,
                    "the report lost the effective identity present at verifier entry",
                )?;
                require(
                    report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::RoleAttribute
                            && finding.object.as_deref() == Some(authenticated.as_str())
                            && finding.role_attribute == Some(RoleAttribute::Superuser)
                    }),
                    "the authenticated superuser authority was not reported",
                )?;
                let restored_mask: (String, String) =
                    sqlx::query_as("SELECT session_user::text, current_user::text")
                        .fetch_one(&masked)
                        .await?;
                require(
                    restored_mask == (names.login_a.clone(), names.login_a.clone()),
                    "verification did not restore the caller's masked session identity",
                )?;
                sqlx::query("RESET SESSION AUTHORIZATION")
                    .execute(&masked)
                    .await?;
                masked.close().await;

                check_set_role_identity(fixture, &names, &policy).await?;
                Ok(())
            })
        })
        .await
}

async fn check_set_role_identity(
    fixture: &mut AuthorityFixture,
    names: &super::Names,
    policy: &VerificationPolicy,
) -> Result {
    // Remove the login's direct path to the ledger so the read can succeed
    // only while the caller's SET ROLE identity is active.
    exec(
        &mut fixture.admin,
        format!(
            "REVOKE SELECT ON {}.{} FROM {}",
            quote(&names.schema_a),
            quote(&names.ledger_a),
            quote(&names.login_a),
        ),
    )
    .await?;
    exec(
        &mut fixture.admin,
        format!(
            "REVOKE USAGE ON SCHEMA {} FROM {}",
            quote(&names.schema_a),
            quote(&names.login_a),
        ),
    )
    .await?;
    let connection = fixture.login(&names.login_a).await?;
    exec(&connection, format!("SET ROLE {}", quote(&names.settable))).await?;
    let report = verify(&connection, policy).await?;
    require(
        report.session_user() == names.login_a && report.current_user() == names.settable,
        "verification did not inspect through the caller's SET ROLE identity",
    )?;
    let restored_role: (String, String, String) =
        sqlx::query_as("SELECT session_user::text, current_user::text, current_setting('role')")
            .fetch_one(&connection)
            .await?;
    require(
        restored_role
            == (
                names.login_a.clone(),
                names.settable.clone(),
                names.settable.clone(),
            ),
        "verification did not restore the caller's SET ROLE state",
    )?;
    sqlx::query("RESET ROLE").execute(&connection).await?;
    connection.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_detects_createrole_admin_across_unusable_memberships() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture
        .run(move |fixture| {
            Box::pin(async move {
                let creator = fixture.login(&names.unrelated_creator).await?;
                let denied_set = sqlx::query(sqlx::AssertSqlSafe(format!(
                    "SET ROLE {}",
                    quote(&names.settable)
                )))
                .execute(&creator)
                .await;
                require(
                    denied_set.is_err(),
                    "the CREATEROLE control unexpectedly crossed a SET FALSE membership",
                )?;
                exec(
                    &creator,
                    format!(
                        "ALTER ROLE {} CONNECTION LIMIT 4",
                        quote(&names.admin_target)
                    ),
                )
                .await?;

                let policy = names_policy(
                    &names,
                    &names.schema_a,
                    &names.ledger_a,
                    &names.table_a,
                    RolePolicy {
                        allow_create_role: true,
                        ..RolePolicy::default()
                    },
                    Vec::new(),
                    false,
                    true,
                    false,
                    false,
                    false,
                )?;
                let report = verify(&creator, &policy).await?;
                require(
                    finding(&report, FindingKind::RoleAdminOption),
                    "CREATEROLE management authority crossed an unusable membership unnoticed",
                )?;
                creator.close().await;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_rejects_row_security_on_the_migration_ledger() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture
        .run(move |fixture| {
            Box::pin(async move {
                let ledger = format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a));
                exec(
                    &mut fixture.admin,
                    format!("INSERT INTO {ledger} VALUES (2, '\\x02', false)"),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!("ALTER TABLE {ledger} ENABLE ROW LEVEL SECURITY"),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "CREATE POLICY visible_required ON {ledger} TO {} USING (version = 1)",
                        quote(&names.login_a),
                    ),
                )
                .await?;
                let login = fixture.login(&names.login_a).await?;
                let visible_versions: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT version FROM {ledger} ORDER BY version"
                )))
                .fetch_all(&login)
                .await?;
                require(
                    visible_versions == vec![1],
                    "the RLS control did not hide the unexpected migration row",
                )?;
                let policy = names_policy(
                    &names,
                    &names.schema_a,
                    &names.ledger_a,
                    &names.table_a,
                    RolePolicy::default(),
                    Vec::new(),
                    false,
                    true,
                    false,
                    false,
                    false,
                )?;
                let report = verify(&login, &policy).await?;
                require(
                    report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::LedgerRowSecurity
                            && finding.object.as_deref() == Some(ledger.as_str())
                    }),
                    "row security on the migration ledger was silently accepted",
                )?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_checks_real_login_reachability_and_two_fixture_policies() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture
        .run(move |fixture| {
            Box::pin(async move {
                noinherit::check(fixture, &names).await?;
                let (login_a, narrow, table_object) = narrow::check(fixture, &names).await?;
                let broad = roles::check(fixture, &names, &login_a, &narrow).await?;
                let predefined_allowed =
                    parameters::check(fixture, &names, &login_a, &broad).await?;
                owners::check(
                    fixture,
                    &names,
                    &login_a,
                    &predefined_allowed,
                    &table_object,
                )
                .await
            })
        })
        .await?;

    // Exercise the same owner on an unwinding body. The panic is intentionally
    // caught by `AuthorityFixture::run`, so the test proves cleanup is retained
    // for panic paths as well as the normal and error paths above.
    let panic_fixture = AuthorityFixture::create().await?;
    let panic_result = panic_fixture
        .run(|_| {
            Box::pin(async {
                panic!("intentional fixture cleanup control");
            })
        })
        .await;
    require(
        panic_result.is_err(),
        "a panicking fixture body was reported as successful",
    )
}
