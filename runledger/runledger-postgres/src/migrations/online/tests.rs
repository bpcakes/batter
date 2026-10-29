use super::*;

fn bundled(version: i64) -> &'static Migration {
    MIGRATOR
        .iter()
        .find(|migration| {
            migration.migration_type.is_up_migration() && migration.version == version
        })
        .expect("declared migration version is bundled")
}

#[test]
fn declared_statements_are_embedded_exactly_once() {
    for index in &ONLINE_INDEXES {
        let sql = bundled(index.migration_version).sql.as_str();
        assert_eq!(
            sql.matches(index.canonical_statement).count(),
            1,
            "{} must appear exactly once in migration {}",
            index.name,
            index.migration_version
        );
    }
}

#[test]
fn concurrent_statements_never_adopt_leftovers() {
    for index in &ONLINE_INDEXES {
        let statement = index.concurrent_statement();
        assert!(
            statement.starts_with(&format!("CREATE INDEX CONCURRENTLY {}\n", index.name)),
            "{statement}"
        );
        assert!(!statement.contains("IF NOT EXISTS"), "{statement}");
        assert!(statement.ends_with(';'), "{statement}");
    }
}

#[test]
fn residual_sql_builds_no_index() {
    let mut versions: Vec<i64> = ONLINE_INDEXES
        .iter()
        .map(|index| index.migration_version)
        .collect();
    versions.dedup();
    for version in versions {
        let migration = bundled(version);
        let declared: Vec<&OnlineIndex> = ONLINE_INDEXES
            .iter()
            .filter(|index| index.migration_version == version)
            .collect();
        let residual = residual_sql(migration, &declared).expect("residual migration");
        for index in &declared {
            assert!(!residual.contains(index.name), "{residual}");
        }
        assert_ne!(residual, migration.sql.as_str());
    }
}

#[test]
fn only_the_cutover_pair_is_adoptable_by_canonical_sql() {
    let adoptable: Vec<&str> = ONLINE_INDEXES
        .iter()
        .filter(|index| index.adoptable_by_canonical_sql())
        .map(|index| index.name)
        .collect();
    assert_eq!(
        adoptable,
        [
            "idx_job_queue_missing_enqueue_request_snapshot",
            "idx_workflow_runs_missing_enqueue_request_snapshot",
        ]
    );
}

#[test]
fn definition_matching_pins_name_table_and_rendering() {
    let index = &ONLINE_INDEXES[4];
    let rendered = "CREATE INDEX idx_job_queue_created_id ON public.job_queue USING btree (created_at DESC, id DESC)";
    assert!(definition_matches(index, rendered, "public"));
    assert!(definition_matches(
        index,
        "CREATE INDEX idx_job_queue_created_id ON \"My Schema\".job_queue USING btree (created_at DESC, id DESC)",
        "My Schema"
    ));
    assert!(!definition_matches(
        index,
        "CREATE INDEX idx_job_queue_created_id ON public.job_queue USING btree (id)",
        "public"
    ));
    assert!(!definition_matches(
        index,
        "CREATE INDEX idx_job_queue_created_id ON public.job_attempts USING btree (created_at DESC, id DESC)",
        "public"
    ));
    assert!(!definition_matches(
        index,
        "CREATE UNIQUE INDEX idx_job_queue_created_id ON public.job_queue USING btree (created_at DESC, id DESC)",
        "public"
    ));
    let claim_order = &ONLINE_INDEXES[2];
    let qualified_type = "CREATE INDEX idx_job_queue_execution_resource_claim_order ON app.job_queue USING btree (priority DESC, next_run_at, created_at, id) INCLUDE (execution_resource_key, job_type) WHERE ((status = 'PENDING'::app.job_status) AND (execution_resource_key IS NOT NULL))";
    assert!(definition_matches(claim_order, qualified_type, "app"));
    assert!(!definition_matches(
        claim_order,
        &qualified_type.replace("'PENDING'", "'app.PENDING'"),
        "app"
    ));
    for schema in ["tenant USING data", "tenant USING \"data\"", "select"] {
        let quoted = format!("\"{}\"", schema.replace('"', "\"\""));
        let definition = format!(
            "CREATE INDEX {} ON {quoted}.job_queue USING btree (priority DESC, next_run_at, created_at, id) INCLUDE (execution_resource_key, job_type) WHERE ((status = 'PENDING'::{quoted}.job_status) AND (execution_resource_key IS NOT NULL))",
            claim_order.name
        );
        assert!(definition_matches(claim_order, &definition, schema));
        assert!(!definition_matches(
            claim_order,
            &definition.replace("'PENDING'", &format!("'{quoted}.PENDING'")),
            schema
        ));
        assert!(!definition_matches(
            claim_order,
            &definition.replace("priority DESC", "id DESC"),
            schema
        ));
        assert!(!definition_matches(
            claim_order,
            &definition,
            "other_schema"
        ));
    }
}

#[test]
fn fully_quoted_definitions_preserve_type_qualification_and_literals() {
    let index = &ONLINE_INDEXES[2];
    let rendered = r#"CREATE INDEX "idx_job_queue_execution_resource_claim_order" ON "app"."job_queue" USING "btree" ("priority" DESC, "next_run_at", "created_at", "id") INCLUDE ("execution_resource_key", "job_type") WHERE (("status" = 'PENDING'::"app"."job_status") AND ("execution_resource_key" IS NOT NULL))"#;
    assert!(definition_matches(index, rendered, "app"));
    assert!(definition_matches(
        index,
        &rendered.replace("::\"app\".", "::"),
        "app"
    ));
    assert!(!definition_matches(
        index,
        &rendered.replace("'PENDING'", "'\"app\".PENDING'"),
        "app"
    ));
    assert!(!definition_matches(index, rendered, "other"));
}

#[test]
fn problems_render_without_database_state() {
    assert_eq!(
        SchemaCompatibilityError::OnlineIndex {
            index: "idx_job_queue_created_id",
            problem: OnlineIndexProblem::Missing,
        }
        .to_string(),
        "Runledger online index idx_job_queue_created_id is missing although its migration is recorded as applied"
    );
}

#[tokio::test]
async fn lifted_profile_timeouts_are_restored_on_the_session() {
    use std::time::Duration;

    use runledger_test_support::{setup_unmigrated_ephemeral_pool, teardown_ephemeral_pool};

    async fn timeouts(conn: &mut PgConnection) -> (String, String, String, String) {
        sqlx::query_as(
            "SELECT pg_catalog.current_setting('statement_timeout'),
                    pg_catalog.current_setting('lock_timeout'),
                    pg_catalog.current_setting('transaction_timeout'),
                    pg_catalog.current_setting('client_connection_check_interval')",
        )
        .fetch_one(conn)
        .await
        .expect("session timeouts")
    }

    let (pool, ephemeral) = setup_unmigrated_ephemeral_pool("online_timeouts_restore", 1).await;
    let options = pool.connect_options();
    let login = options.get_username();
    // All three lifted settings nonzero, so a regression in any lift or
    // restore branch, including the transaction_timeout one, is visible.
    let profile = crate::PgSessionProfile::with_timeouts(
        login,
        login,
        vec!["public".into()],
        Duration::from_secs(2),
        Duration::from_millis(250),
        Duration::from_secs(3),
        Duration::from_secs(4),
    )
    .expect("short-timeout profile");
    let database = crate::RunledgerDatabase::connect((*options).clone(), profile, 1)
        .await
        .expect("profiled database");
    let mut conn = database.pool().acquire().await.expect("session");

    for (configured, expected) in [("0", "1s"), ("100ms", "100ms"), ("5s", "1s")] {
        sqlx::query("SELECT pg_catalog.set_config('client_connection_check_interval', $1, false)")
            .bind(configured)
            .execute(&mut *conn)
            .await
            .expect("initial disconnect check");
        let before = timeouts(&mut conn).await;
        assert_eq!(
            before,
            ("2s".into(), "250ms".into(), "4s".into(), configured.into())
        );
        let lifted = ProfileTimeouts::lift(&mut conn).await.expect("lift");
        assert_eq!(
            timeouts(&mut conn).await,
            ("0".into(), "0".into(), "0".into(), expected.into())
        );
        lifted.restore(&mut conn).await.expect("restore");
        assert_eq!(
            timeouts(&mut conn).await,
            before,
            "all original session settings return after the build"
        );
    }

    drop(conn);
    database.pool().close().await;
    teardown_ephemeral_pool(pool, ephemeral).await;
}

#[test]
fn execution_failure_retains_both_causes_without_rendering_them() {
    let error = execution_error(
        &ONLINE_INDEXES[3],
        IndexHistory::Recorded,
        OnlineIndexPhase::Build,
        sqlx::Error::Protocol("primary-secret".into()).into(),
        Some(sqlx::Error::Protocol("restore-secret".into())),
    );
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains("secret"));
    let SchemaCompatibilityError::OnlineIndexExecution(failure) = error else {
        panic!("context");
    };
    assert!(failure.cause().to_string().contains("primary-secret"));
    assert!(
        failure
            .restoration_error()
            .expect("secondary restoration cause retained")
            .to_string()
            .contains("restore-secret")
    );
    assert_eq!(failure.recorded_version(), Some(202609050001));
}
