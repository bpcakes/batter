use super::support::{Result, require};
use super::{
    AuthorityFixture, Names, VerificationPolicy, exact_finding, exec, names_policy, quote,
    verify_combined as verify,
};
use batter::operation::{OperationContext, OperationError};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, ObjectPrivilege, ParameterName, ParameterPolicy, RolePolicy,
    VerificationError,
};
use std::time::Duration;

fn policy(names: &Names) -> Result<VerificationPolicy> {
    names_policy(
        names,
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
    )
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_checks_mixed_case_builtin_parameter_names() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
                exec(&mut fixture.admin,
                    "SELECT pg_catalog.pg_advisory_lock(hashtext('batter-verification-work-mem'))".to_owned()).await?;
                for (parameter, value) in [("TimeZone", "UTC"), ("DateStyle", "ISO, MDY"), ("IntervalStyle", "iso_8601")] {
                    let key = parameter.to_ascii_lowercase();
                    let absent: bool = sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM pg_parameter_acl WHERE lower(parname) = $1)")
                        .bind(&key).fetch_one(&mut fixture.admin).await?;
                    require(absent, "mixed-case no-ACL control requires an absent parameter ACL row")?;
                    for explicit_acl in [false, true] {
                        if explicit_acl {
                            exec(&mut fixture.admin, format!("GRANT SET ON PARAMETER {parameter} TO {}", quote(&names.login_a))).await?;
                        }
                        let actual: String = sqlx::query_scalar("SELECT pg_catalog.set_config($1, $2, false)")
                            .bind(parameter).bind(value).fetch_one(&pool).await?;
                        require(actual == value, "the native mixed-case parameter SET control did not succeed")?;
                        let mut denied = policy(&names)?;
                        denied.authority.parameters.push(ParameterPolicy {
                            parameter: ParameterName::new(parameter)?, privileges: Vec::new(),
                        });
                        let report = super::verify_policy(&pool, &denied).await?;
                        require(exact_finding(&report, FindingKind::ProtectedParameter,
                            Some(&key), Some(&names.login_a), Some(ObjectPrivilege::Set)),
                            "mixed-case pg_settings name concealed working SET authority")?;
                        require(!report.findings().iter().any(|finding| finding.kind == FindingKind::MissingObject
                            && finding.object.as_deref() == Some(&key)), "existing mixed-case parameter was reported missing")?;
                        denied.authority.parameters.last_mut().expect("added parameter").privileges
                            .push(AllowedPrivilege::new(ObjectPrivilege::Set, false));
                        let report = super::verify_policy(&pool, &denied).await?;
                        require(!report.findings().iter().any(|finding| finding.object.as_deref() == Some(&key)),
                            "allowed mixed-case built-in parameter did not satisfy its policy")?;
                    }
                }
                exec(&mut fixture.admin,
                    "SELECT pg_catalog.pg_advisory_unlock(hashtext('batter-verification-work-mem'))".to_owned()).await?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_rejects_oversized_parameter_acl_catalog() -> Result {
    AuthorityFixture::create().await?.run(|fixture| Box::pin(async move {
        let names = fixture.names.clone();
        let pool = fixture.login(&names.login_a).await?;
        let baseline: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_parameter_acl CROSS JOIN LATERAL aclexplode(paracl)")
            .fetch_one(&mut fixture.admin).await?;
        let room = 10_000 - baseline;
        require(room >= 3, "catalog capacity fixture has insufficient isolated headroom")?;
        // Each new ACL has two bootstrap-owner privileges and one login SET.
        let parameters = (0..room / 3).map(|index| format!("{}_capacity_{index}", names.hidden_parameter))
            .collect::<Vec<_>>().join(", ");
        exec(&mut fixture.admin, format!("GRANT SET ON PARAMETER {parameters} TO {}", quote(&names.login_a))).await?;
        let first = format!("{}_capacity_0", names.hidden_parameter);
        for role in [&names.login_b, &names.login_noinherit].into_iter().take((room % 3) as usize) {
            exec(&mut fixture.admin, format!("GRANT SET ON PARAMETER {first} TO {}", quote(role))).await?;
        }
        let exact: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_parameter_acl CROSS JOIN LATERAL aclexplode(paracl)")
            .fetch_one(&mut fixture.admin).await?;
        require(exact == 10_000, "capacity fixture did not reach the exact expanded-ACL boundary")?;
        super::verify_policy(&pool, &policy(&names)?).await?;
        exec(&mut fixture.admin, format!("GRANT SET ON PARAMETER {first} TO {}", quote(&names.unrelated_creator))).await?;
        let context = OperationContext::new(Duration::from_secs(10))?;
        let result = verify(&pool, &context, &policy(&names)?).await;
        require(matches!(result, Err(OperationError::Failed(VerificationError::CatalogCapacity))),
            "oversized parameter ACL catalog produced a partial report instead of rejecting capacity")?;
        require(pool.size() == 0, "failed capacity inspection returned its client to the pool")?;
        Ok(())
    })).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_rejects_oversized_parameter_name() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
                let parameter = format!(
                    "{}.{}",
                    names.hidden_parameter,
                    vec!["x".repeat(63); 17].join(".")
                );
                require(
                    parameter.len() > 1024,
                    "parameter size control was not oversized",
                )?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "GRANT SET ON PARAMETER {parameter} TO {}",
                        quote(&names.login_a)
                    ),
                )
                .await?;
                let context = OperationContext::new(Duration::from_secs(10))?;
                let result = verify(&pool, &context, &policy(&names)?).await;
                require(
                    matches!(
                        result,
                        Err(OperationError::Failed(VerificationError::CatalogCapacity))
                    ),
                    "oversized variable-length catalog value was retained or silently omitted",
                )?;
                Ok(())
            })
        })
        .await
}
