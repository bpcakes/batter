use super::support::{Result, require};
use super::{AuthorityFixture, exec, names_policy, quote};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, ParameterName, ParameterPolicy, PublicObject,
    RequiredPrivilege, RolePolicy, UnsupportedSurface, VerificationStatus,
};

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_distinguishes_hidden_parameters_from_missing_objects() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
                let parameters = [
                    "session_preload_libraries",
                    "dynamic_library_path",
                    "missing_parameter",
                    "role",
                ];
                let visible: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_settings WHERE name = ANY($1::text[])",
                )
                .bind(&parameters[..])
                .fetch_one(&pool)
                .await?;
                require(
                    visible == 0,
                    "restricted-role control unexpectedly sees the hidden settings",
                )?;
                let mut policy = names_policy(
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
                for parameter in parameters {
                    policy.authority.parameters.push(ParameterPolicy {
                        parameter: ParameterName::new(parameter)?,
                        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
                    });
                    policy
                        .authority
                        .required_privileges
                        .push(RequiredPrivilege {
                            object: PublicObject::Parameter(ParameterName::new(parameter)?),
                            privilege: ObjectPrivilege::Set,
                        });
                }
                let report = super::verify_policy(&pool, &policy).await?;
                require(
                    report.status() == VerificationStatus::Incomplete
                        && report
                            .unsupported()
                            .contains(&UnsupportedSurface::ParameterVisibility),
                    "unobservable parameters did not make the report explicitly incomplete",
                )?;
                for parameter in parameters {
                    require(
                        report.findings().iter().any(|finding| {
                            finding.kind == FindingKind::ParameterUnobservable
                                && finding.object.as_deref() == Some(parameter)
                        }),
                        "hidden or absent setting was not reported unobservable",
                    )?;
                    require(
                        !report.findings().iter().any(|finding| {
                            matches!(
                                finding.kind,
                                FindingKind::MissingObject | FindingKind::MissingPrivilege
                            ) && finding.object.as_deref() == Some(parameter)
                        }),
                        "restricted visibility manufactured an absence verdict",
                    )?;
                }
                exec(
                    &mut fixture.admin,
                    format!(
                        "GRANT pg_read_all_settings TO {} WITH INHERIT FALSE, SET TRUE",
                        quote(&names.login_a)
                    ),
                )
                .await?;
                let visible: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_settings WHERE name = ANY($1::text[])",
                )
                .bind(&parameters[..])
                .fetch_one(&pool)
                .await?;
                require(
                    visible == 0,
                    "SET-only settings membership unexpectedly changed current visibility",
                )?;
                let report = super::verify_policy(&pool, &policy).await?;
                require(
                    report.status() == VerificationStatus::Incomplete,
                    "SET-only settings visibility was mistaken for current-role observation",
                )?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "GRANT pg_read_all_settings TO {} WITH INHERIT TRUE, SET FALSE",
                        quote(&names.login_a)
                    ),
                )
                .await?;
                let visible: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_settings WHERE name = ANY($1::text[])",
                )
                .bind(&parameters[..])
                .fetch_one(&pool)
                .await?;
                require(
                    visible == 2,
                    "inherited pg_read_all_settings did not reveal the two existing parameters",
                )?;
                let report = super::verify_policy(&pool, &policy).await?;
                require(
                    !report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::ParameterUnobservable
                            && finding.object.as_deref() != Some("role")
                    }),
                    "fully visible built-ins remained unobservable",
                )?;
                require(
                    report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::MissingObject
                            && finding.object.as_deref() == Some("missing_parameter")
                    }),
                    "fully visible nonexistent parameter was not reported missing",
                )?;
                let role_exists: bool =
                    sqlx::query_scalar("SELECT current_setting('role', true) IS NOT NULL")
                        .fetch_one(&pool)
                        .await?;
                require(
                    role_exists,
                    "the NO_SHOW_ALL built-in control did not exist",
                )?;
                require(
                    report.status() == VerificationStatus::Incomplete
                        && report.findings().iter().any(|finding| {
                            finding.kind == FindingKind::ParameterUnobservable
                                && finding.object.as_deref() == Some("role")
                        }),
                    "known NO_SHOW_ALL parameter was mistaken for complete context coverage",
                )?;
                require(
                    !report.findings().iter().any(|finding| {
                        matches!(
                            finding.kind,
                            FindingKind::MissingObject | FindingKind::MissingPrivilege
                        ) && finding.object.as_deref() == Some("role")
                    }),
                    "known NO_SHOW_ALL parameter was reported absent",
                )?;
                for parameter in &parameters[..2] {
                    require(
                        !report.findings().iter().any(|finding| {
                            finding.kind == FindingKind::MissingObject
                                && finding.object.as_deref() == Some(*parameter)
                        }),
                        "visible existing parameter was reported missing",
                    )?;
                }
                exec(
                    &mut fixture.admin,
                    format!("REVOKE pg_read_all_settings FROM {}", quote(&names.login_a)),
                )
                .await?;
                let report = super::verify_policy(&pool, &policy).await?;
                require(
                    report.status() == VerificationStatus::Incomplete,
                    "revoking visibility did not restore the incomplete verdict",
                )?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_reserved_custom_parameter_requirement() -> Result {
    use batter::operation::OperationContext;
    use batter_sqlx::verification::{
        AuthorityPolicyBuilder, DatabasePolicy, PublicGrant, verify_authority,
    };
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let role = fixture.names.login_b.clone();
                let pool = fixture.login(&role).await?;
                exec(&pool, "DO $$ BEGIN END $$".into()).await?;
                let native = sqlx::query("SET plpgsql.missing_parameter = 'x'")
                    .execute(&pool)
                    .await;
                require(
                    native.is_err(),
                    "native reserved prefix unexpectedly accepted the parameter",
                )?;
                let allowances = vec![
                    AllowedPrivilege::new(ObjectPrivilege::Connect, false),
                    AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
                ];
                let mut policy = AuthorityPolicyBuilder {
                    database: DatabasePolicy {
                        privileges: allowances.clone(),
                        allow_owner: false,
                    },
                    public_grants: allowances
                        .into_iter()
                        .map(|privilege| PublicGrant {
                            object: PublicObject::Database,
                            privilege,
                        })
                        .collect(),
                    parameters: vec![
                        ParameterPolicy {
                            parameter: ParameterName::new("work_mem")?,
                            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
                        },
                        ParameterPolicy {
                            parameter: ParameterName::new("plpgsql.missing_parameter")?,
                            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
                        },
                    ],
                    required_privileges: vec![RequiredPrivilege {
                        object: PublicObject::Parameter(ParameterName::new(
                            "plpgsql.missing_parameter",
                        )?),
                        privilege: ObjectPrivilege::Set,
                    }],
                    ..AuthorityPolicyBuilder::default()
                };
                let compiled = policy.clone().build()?;
                let report = verify_authority(
                    &pool,
                    &OperationContext::new(std::time::Duration::from_secs(5))?,
                    &compiled,
                )
                .await?;
                require(
                    report.status() == VerificationStatus::Incomplete
                        && report.findings().iter().any(|finding| {
                            finding.kind == FindingKind::ParameterUnobservable
                                && finding.object.as_deref() == Some("plpgsql.missing_parameter")
                        })
                        && !report
                            .findings()
                            .iter()
                            .any(|finding| finding.kind == FindingKind::MissingPrivilege),
                    "unknown reserved-prefix requirement did not retain honest incomplete coverage",
                )?;
                // A currently assignable placeholder still has no observable definition.
                // Do not mistake its successful native SET for catalog context proof.
                exec(&pool, "SET verification_fixture.user_setting = 'x'".into()).await?;
                policy.parameters[1].parameter =
                    ParameterName::new("verification_fixture.user_setting")?;
                policy.required_privileges[0].object = PublicObject::Parameter(ParameterName::new(
                    "verification_fixture.user_setting",
                )?);
                let compiled = policy.clone().build()?;
                let report = verify_authority(
                    &pool,
                    &OperationContext::new(std::time::Duration::from_secs(5))?,
                    &compiled,
                )
                .await?;
                require(
                    report.status() == VerificationStatus::Incomplete,
                    "placeholder syntax became positive required-authority evidence",
                )?;
                // PL/pgSQL's actual user-context setting is visible after initialization.
                exec(&pool, "SET plpgsql.print_strict_params = on".into()).await?;
                policy.parameters[1].parameter = ParameterName::new("plpgsql.print_strict_params")?;
                policy.required_privileges[0].object =
                    PublicObject::Parameter(ParameterName::new("plpgsql.print_strict_params")?);
                let compiled = policy.build()?;
                let report = verify_authority(
                    &pool,
                    &OperationContext::new(std::time::Duration::from_secs(5))?,
                    &compiled,
                )
                .await?;
                require(
                    report.is_within_declared_policy() && report.findings().is_empty(),
                    "observable loaded parameter failed its current-role requirement",
                )?;
                Ok(())
            })
        })
        .await
}
