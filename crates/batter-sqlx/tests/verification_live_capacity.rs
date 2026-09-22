use super::support::{Result, require};
use super::{AuthorityFixture, exec, names_policy, quote};
use batter_core::operation::{Interruption, OperationContext, OperationError};
use batter_sqlx::verification::{
    AllowedPrivilege, FindingKind, Identifier, ObjectPrivilege, QualifiedName, RelationPolicy,
    RolePolicy, VerificationStatus, verify,
};
use sqlx::{
    Connection, PgConnection,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_uses_one_snapshot_for_concurrent_acl_changes() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture
        .run(move |fixture| {
            Box::pin(async move {
                let mut blocker = PgConnection::connect(&fixture.url).await?;
                let mut mutator = PgConnection::connect(&fixture.url).await?;
                // Ownership avoids a membership lookup during the early ledger
                // lock; this barrier exercises the later catalog snapshot read.
                exec(
                    &mut fixture.admin,
                    format!(
                        "ALTER SCHEMA {} OWNER TO {}",
                        quote(&names.schema_a),
                        quote(&names.login_a)
                    ),
                )
                .await?;
                exec(
                    &mut fixture.admin,
                    format!(
                        "ALTER TABLE {}.{} OWNER TO {}",
                        quote(&names.schema_a),
                        quote(&names.ledger_a),
                        quote(&names.login_a)
                    ),
                )
                .await?;
                let verifier = fixture.login(&names.login_a).await?;
                let verifier_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&verifier)
                    .await?;
                sqlx::query("BEGIN").execute(&mut blocker).await?;
                sqlx::query("LOCK TABLE pg_catalog.pg_auth_members IN ACCESS EXCLUSIVE MODE")
                    .execute(&mut blocker)
                    .await?;
                let policy = names_policy(
                    &names,
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
                let task =
                    tokio::spawn(async move { crate::verify_policy(&verifier, &policy).await });
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let waiting: bool = sqlx::query_scalar(
                        "SELECT EXISTS (
                            SELECT 1 FROM pg_locks l JOIN pg_stat_activity a USING (pid)
                            WHERE l.pid = $1 AND NOT l.granted
                              AND l.relation = 'pg_catalog.pg_auth_members'::regclass
                              AND a.query LIKE '%FROM pg_catalog.pg_auth_members%'
                        )",
                    )
                    .bind(verifier_pid)
                    .fetch_one(&mut fixture.admin)
                    .await?;
                    if waiting {
                        break;
                    }
                    require(
                        tokio::time::Instant::now() < deadline,
                        "snapshot verifier did not reach the role membership catalog lock",
                    )?;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }

                // Commit a new grant only after the verifier's repeatable-read
                // snapshot exists. The report must not mix this ACL with its
                // pre-change catalog view.
                let table = format!("{}.{}", quote(&names.schema_a), quote(&names.table_a));
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "GRANT DELETE ON {table} TO {}",
                    quote(&names.login_a)
                )))
                .execute(&mut mutator)
                .await?;
                sqlx::query("ROLLBACK").execute(&mut blocker).await?;
                let report = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .map_err(|_| std::io::Error::other("snapshot verifier did not finish"))?
                    .map_err(|_| std::io::Error::other("snapshot verifier task panicked"))??;
                require(
                    !report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::Privilege
                            && finding.privilege == Some(ObjectPrivilege::Delete)
                            && finding.subject.as_deref() == Some(names.login_a.as_str())
                            && finding
                                .object
                                .as_deref()
                                .is_some_and(|object| object.contains(&names.table_a))
                    }),
                    "a committed ACL change after the snapshot leaked into the report",
                )?;
                mutator.close().await?;
                blocker.close().await?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_keeps_many_declared_objects_set_oriented() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture
        .run(move |fixture| {
            Box::pin(async move {
                let login_a = fixture.login(&names.login_a).await?;
                let mut policy = names_policy(
                    &names,
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
                for index in 0..48_u32 {
                    let table = format!("bulk_{index}");
                    exec(
                        &mut fixture.admin,
                        format!(
                            "CREATE TABLE {}.{} (c0 integer, c1 integer, c2 integer, c3 integer, c4 integer, c5 integer, c6 integer, c7 integer)",
                            quote(&names.schema_a),
                            quote(&table),
                        ),
                    )
                    .await?;
                    policy.authority.relations.push(RelationPolicy {
                        relation: QualifiedName::new(&names.schema_a, table)?,
                        privileges: Vec::new(),
                        columns: vec![batter_sqlx::verification::ColumnPolicy {
                            column: Identifier::new("c0")?,
                            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
                        }],
                        allow_owner: false,
                        allow_row_type_public_usage: true,
                    });
                }
                let report = crate::verify_policy(&login_a, &policy).await?;
                require(
                    report.status() == VerificationStatus::WithinDeclaredPolicy,
                    "many declared relations did not remain within their empty ACL policy",
                )?;
                require(
                    !report.findings().iter().any(|finding| {
                        finding.kind == FindingKind::MissingObject
                            || finding
                                .object
                                .as_deref()
                                .is_some_and(|object| object.contains("bulk_"))
                    }),
                    "set-oriented many-object verification reported a false object finding",
                )?;
                Ok(())
            })
        })
        .await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
#[allow(clippy::too_many_lines)]
async fn verification_cancellation_releases_one_slot_capacity() -> Result {
    let fixture = AuthorityFixture::create().await?;
    let names = fixture.names.clone();
    fixture.run(move |fixture| Box::pin(async move {
        let mut blocker = PgConnection::connect(&fixture.url).await?;
        let mut observer = PgConnection::connect(&fixture.url).await?;
        sqlx::query("BEGIN").execute(&mut blocker).await?;
        let lock_table = format!(
            "LOCK TABLE {}.{} IN ACCESS EXCLUSIVE MODE",
            quote(&names.schema_a),
            quote(&names.ledger_a),
        );
        sqlx::query(sqlx::AssertSqlSafe(lock_table))
            .execute(&mut blocker)
            .await?;
        let options = PgConnectOptions::from_str(&fixture.url)?
            .username(&names.login_a)
            .password(&fixture.password);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with(options)
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
        let context = batter_core::operation::OperationOwner::new(Duration::from_secs(10))?.into_context();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await?;
        let task_pool = pool.clone();
        let task_context = context.clone();
        let authority = policy.authority.build()?;
        let migration = policy.migration;
        let mut task = tokio::spawn(async move {
            let plan = batter_sqlx::verification::VerificationPlan::migrations(&migration)
                .with_authority(&authority)
                .expect("capacity plan has one authority component");
            verify(&task_pool, &task_context, plan).await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')",
            )
            .bind(pid)
            .fetch_one(&mut observer)
            .await?;
            if waiting {
                break;
            }
            require(
                tokio::time::Instant::now() < deadline,
                "verification query did not reach the locked ledger",
            )?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        context.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), &mut task).await??;
        require(
            matches!(result, Err(OperationError::Interrupted(Interruption::Cancelled))),
            "checker cancellation was not classified",
        )?;
        require(
            pool.size() == 0,
            "cancelled checker retained one-slot local capacity",
        )?;
        tokio::time::timeout(Duration::from_secs(2), pool.close())
            .await
            .map_err(|_| std::io::Error::other("pool close remained blocked after lease retirement"))?;
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')",
        )
        .bind(pid)
        .fetch_one(&mut observer)
        .await?;
        require(
            waiting,
            "local capacity recovery was mistaken for remote query termination",
        )?;
        sqlx::query("ROLLBACK").execute(&mut blocker).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let present: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)",
            )
            .bind(pid)
            .fetch_one(&mut observer)
            .await?;
            if !present {
                break;
            }
            require(
                tokio::time::Instant::now() < deadline,
                "retired backend did not terminate after lock release",
            )?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        observer.close().await?;
        blocker.close().await?;
        Ok(())
    }))
    .await
}
