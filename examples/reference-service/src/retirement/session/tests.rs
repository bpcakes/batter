use super::*;
use crate::config::{ConfigMode, RootSettings};
use batter::{
    cleanup::CleanupBudget,
    command::{Command, CommandCause},
    operation::OperationContext,
    settings::SettingsSource,
};
use sqlx::{Connection, PgConnection, postgres::PgSslMode};

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn maintenance_session_replacement_is_refused() -> Result<(), batter::BoxError> {
    let url = std::env::var("POSTGRES_TEST_ADMIN_URL")?;
    let settings = RootSettings::from_sources(
        ConfigMode::Setup,
        None,
        SettingsSource::default(),
        SettingsSource::from_pairs([("DATABASE_URL".into(), url.into())])?,
    )?;
    let options = settings.connect_options_from_process()?;
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
    let mut observer = PgConnection::connect_with(&options).await?;
    let (system, oid): (i64, i64) = sqlx::query_as(
        "SELECT system_identifier, oid::bigint FROM pg_control_system(), pg_database WHERE datname=current_database()"
    ).fetch_one(&mut observer).await?;
    observer.close().await?;
    let expected = DatabaseIdentity::new(system, u32::try_from(oid)?)?;
    let second = Duration::from_secs(1);
    let command = Command::new(
        OperationContext::new(second * 10)?,
        CleanupBudget::new(second * 3, second * 3, second)?,
        move |scope| {
            Box::pin(async move {
                let session = Session::new(scope, options.clone())?;
                session.verify(expected).await?;
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&session.pool)
                    .await?;
                let mut observer = PgConnection::connect_with(&options).await?;
                let stopped =
                    sqlx::query_scalar::<_, bool>("SELECT pg_terminate_backend($1, 5000)")
                        .bind(pid)
                        .fetch_one(&mut observer)
                        .await;
                observer.close().await?;
                assert!(
                    stopped?,
                    "the exact owned maintenance backend must terminate"
                );
                let result = session
                    .pool
                    .acquire()
                    .await
                    .map(|_| ())
                    .map_err(RetirementError::from);
                session.finish(result)
            })
        },
    )
    .start();
    let report = command.wait().await?;
    assert!(
        report
            .cleanup
            .as_ref()
            .is_ok_and(|cleanup| cleanup.is_success())
    );
    let Err(CommandCause::Failed(RetirementError::SessionReplaced {
        original: Some(error),
    })) = &report.work
    else {
        panic!("replacement must fail with retained acquisition error: {report:?}")
    };
    assert!(matches!(
        &**error,
        RetirementError::Database(sqlx::Error::PoolTimedOut)
    ));
    Ok(())
}
