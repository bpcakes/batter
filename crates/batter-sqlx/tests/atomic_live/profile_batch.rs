use super::{
    fixture,
    support::{Result, bounded, combine},
    validation::counted,
};
use batter_sqlx::{PgProfiledPool, PgSessionProfile};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn idle_acquisition_statement_count_is_bounded_with_custom_settings() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let options = fixture.pool.connect_options();
        let login = options.get_username();
        for setting_count in [0, 1, 1700] {
            let mut profile = PgSessionProfile::with_timeouts(
                login,
                login,
                vec!["public".into()],
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
            )?;
            for i in 0..setting_count {
                profile = profile.with_setting(format!("batch.value_{i}"), "retained")?;
            }
            let database = PgProfiledPool::connect_lazy(
                options.as_ref().clone(),
                profile,
                PgPoolOptions::new().max_connections(1),
            )?;
            let check = async {
                let mut connection = bounded(database.pool().acquire()).await??;
                let original: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut *connection)
                    .await?;
                drop(connection);
                for _ in 0..2 {
                    // Exclude release normalization from acquisition accounting.
                    bounded(async {
                        while database.pool().num_idle() != 1 {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await?;
                    let expected = if setting_count == 0 { 1 } else { 2 };
                    let mut connection =
                        counted(expected, bounded(database.pool().acquire())).await??;
                    let reused: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                        .fetch_one(&mut *connection)
                        .await?;
                    assert_eq!(
                        reused, original,
                        "must measure idle reuse, not replacement setup"
                    );
                    drop(connection);
                }
                Ok(())
            }
            .await;
            let close = bounded(database.pool().close()).await;
            combine(check, close)?;
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn batched_profiles_preserve_large_declarations_and_statement_count() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let legacy = PgSessionProfile::new(
            &login,
            &login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )?;
        let complete = PgSessionProfile::with_timeouts(
            &login,
            &login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        )?;
        let mut connection = fixture.pool.acquire().await?;
        // Both modes cross the previous 1664-target limit. Every value is
        // checked, including quotes and SQL-looking content passed as data.
        for mut profile in [legacy, complete] {
            let names: Vec<_> = (0..1700).map(|i| format!("batch.value_{i}")).collect();
            let values: Vec<_> = (0..1700)
                .map(|i| format!("value '{i}'; SELECT 1 --"))
                .collect();
            for (name, value) in names.iter().zip(&values) {
                profile = profile.with_setting(name, value)?;
            }
            counted(5, profile.reset_and_apply(&mut connection)).await?;
            let actual: Vec<String> = sqlx::query_scalar(
                "SELECT current_setting(name) FROM unnest($1::text[]) \
                 WITH ORDINALITY AS settings(name, position) ORDER BY position",
            )
            .bind(&names)
            .fetch_all(&mut *connection)
            .await?;
            assert_eq!(actual, values);
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profile_timeout_display_units_preserve_exact_milliseconds() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let mut connection = fixture.pool.acquire().await?;
        for (millis, displayed) in [
            (0, "0"), (999, "999ms"), (1000, "1s"), (60_000, "1min"),
            (3_600_000, "1h"), (86_400_000, "1d"), (i32::MAX as u64, "2147483647ms"),
        ] {
            let duration = Duration::from_millis(millis);
            // Keep the total/idle limits disabled so this representation test
            // does not depend on completing a transaction within a tiny limit.
            let profile = PgSessionProfile::with_timeouts(&login, &login, vec!["public".into()],
                Duration::ZERO, duration, Duration::ZERO, Duration::ZERO)?;
            profile.reset_and_apply(&mut connection).await?;
            let rendered: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
                .fetch_one(&mut *connection).await?;
            assert_eq!(rendered, displayed);
            for style in ["postgres", "postgres_verbose", "sql_standard", "iso_8601"] {
                sqlx::query("SELECT set_config('intervalstyle', $1, false)")
                    .bind(style).execute(&mut *connection).await?;
                let observed: i64 = sqlx::query_scalar(
                    "SELECT (EXTRACT(EPOCH FROM current_setting('lock_timeout')::interval) * 1000)::bigint")
                    .fetch_one(&mut *connection).await?;
                assert_eq!(observed, millis as i64, "{style}");
            }
        }
        Ok(())
    }.await;
    fixture.finish(body).await
}
