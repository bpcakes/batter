use super::*;

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn admission_and_completion_share_restricted_role_schema_and_rls_settings() {
    let f = ProfileFixture::new().await;
    // Contaminate the session through the native escape hatch. Mandatory release
    // and acquisition hooks must restore policy before the native limiter runs.
    let mut connection = f.database.pool().acquire().await.unwrap();
    sqlx::raw_sql("RESET ROLE; SET search_path = public; SET app.tenant = 'wrong'")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    for accepted in [false, true] {
        let label = if accepted { "accepted" } else { "rejected" };
        let output = f.runner().run(&context(3000), f.subject(label),
            |_| async { Ok::<_, Infallible>(accepted) }, async |sql, accepted| {
                let authority: (String, String, String) = sqlx::query_as(
                    "SELECT current_user::text, current_schema()::text, current_setting('app.tenant')")
                    .fetch_one(sql.executor()).await?;
                assert_eq!(authority, (f.role.clone(), f.schema.clone(), "expected".into()));
                sqlx::query("INSERT INTO events(kind) VALUES ($1)")
                    .bind(label).execute(sql.executor()).await?;
                Ok::<Authentication<(), ()>, sqlx::Error>(if accepted {
                    Authentication::Accepted(())
                } else { Authentication::Rejected(()) })
            }).await.unwrap();
        assert_eq!(
            output.completion().consecutive_failures(),
            u32::from(!accepted)
        );
    }
    let events = f.events().await;
    assert_eq!(
        events.len(),
        4,
        "both admissions and both decisions persist"
    );
    for (_, role, tenant, schema) in events {
        assert_eq!(
            (role, tenant, schema),
            (f.role.clone(), "expected".into(), f.schema.clone())
        );
    }
    let result = f
        .runner()
        .run(
            &context(3000),
            f.subject("forbidden"),
            |_| async { Ok::<_, Infallible>(()) },
            async |sql, ()| {
                sqlx::query("SELECT * FROM private_admin_data")
                    .execute(sql.executor())
                    .await?;
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(
        matches!(result, Err(AttemptError::Atomic(PgAtomicError::Rejected(
        PgScopeError::Application(AttemptWriteError::Application(ref error))
    ))) if error.as_database_error().unwrap().code().as_deref() == Some("42501"))
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn missing_authoritative_attempts_never_falls_back_to_public() {
    let f = ProfileFixture::new().await;
    let public_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public.runlimit_attempts') IS NOT NULL")
            .fetch_one(&f.base.pool)
            .await
            .unwrap();
    assert!(public_exists, "public decoy exists");
    sqlx::query("DROP TABLE runlimit_attempts CASCADE")
        .execute(f.migration.pool())
        .await
        .unwrap();
    let calls = AtomicUsize::new(0);
    let result = f
        .runner()
        .run(
            &context(3000),
            f.subject("missing"),
            |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Infallible>(())
            },
            async |_, ()| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(result, Err(AttemptError::Admission(_))));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(f.events().await.is_empty());
    f.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn profile_drift_in_decision_prevents_committed_output() {
    let f = ProfileFixture::new().await;
    let result = f
        .runner()
        .run(
            &context(3000),
            f.subject("drift"),
            |_| async { Ok::<_, Infallible>(()) },
            async |sql, ()| {
                sqlx::query("INSERT INTO events(kind) VALUES ('must-rollback')")
                    .execute(sql.executor())
                    .await?;
                sqlx::query("RESET ROLE").execute(sql.executor()).await?;
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(result, Err(AttemptError::Atomic(_))));
    // Retirement is not synchronous server rollback; synchronize observation
    // using a native claim of the same row before checking durable audit.
    let native = PostgresAttemptLimiter::new(f.database.pool().clone());
    assert!(matches!(
        native.admit(f.subject("drift")).await.unwrap(),
        AttemptAdmission::Denied(AttemptDenial::Busy { .. })
    ));
    assert_eq!(f.events().await.len(), 1, "only admission audit committed");
    f.close().await;
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL 18 DATABASE_URL"]
async fn wrong_login_profile_prevents_both_factories() {
    let f = ProfileFixture::new().await;
    let profile = PgSessionProfile::new(
        "not_the_login",
        &f.role,
        vec![f.schema.clone()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let database = PgProfiledPool::connect_lazy(
        (*f.base.pool.connect_options()).clone(),
        profile,
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(100)),
    )
    .unwrap();
    let calls = AtomicUsize::new(0);
    let result = AttemptRunner::new(database.clone())
        .unwrap()
        .run(
            &context(1000),
            f.subject("wrong-login"),
            |_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Infallible>(())
            },
            async |_, ()| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<Authentication<(), ()>, sqlx::Error>(Authentication::Accepted(()))
            },
        )
        .await;
    assert!(matches!(result, Err(AttemptError::Admission(_))));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(f.events().await.is_empty());
    database.pool().close().await;
    f.close().await;
}
