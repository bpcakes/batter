use super::{
    fixture,
    support::{Result, bounded, require},
};
use batter_sqlx::{
    PgAtomicError, PgReadOnlySnapshot, PgSessionProfile, PgSnapshotError, SqlxFailure,
    run_atomic_profiled,
};
use sqlx::postgres::PgPoolOptions;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

const MARKER: &str = "profile-secret-marker-not-a-timezone";

fn policy(login: &str, role: &str) -> Result<PgSessionProfile> {
    Ok(PgSessionProfile::with_timeouts(
        login,
        role,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
    )?)
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profile_setup_errors_redact_setting_and_role_values() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        for (profile, marker, code) in [
            (
                policy(&login, &login)?.with_setting("timezone", MARKER)?,
                MARKER,
                "22023",
            ),
            (
                policy(&login, "profile-secret-missing-role")?,
                "profile-secret-missing-role",
                "22023",
            ),
        ] {
            let error = profile
                .reset_and_apply(&mut fixture.observer)
                .await
                .unwrap_err();
            require(
                !format!("{error} {error:?}").contains(marker),
                "profile setup formatting disclosed marker",
            )?;
            let sqlx::Error::Configuration(cause) = error else {
                return Err("setup error did not retain a safe-display wrapper".into());
            };
            let retained = cause
                .downcast_ref::<SqlxFailure>()
                .ok_or("missing native cause wrapper")?;
            let native = retained
                .native()
                .as_database_error()
                .ok_or("native database error lost")?;
            require(
                native.code().as_deref() == Some(code),
                "native SQLSTATE changed",
            )?;
            require(
                native.message().contains(marker),
                "explicit native inspection lost marker",
            )?;
            require(
                std::error::Error::source(retained).is_some(),
                "native error source lost",
            )?;
            let previous: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&fixture.pool)
                .await?;
            let error = run_atomic_profiled(&fixture.pool, &profile, async |_| {
                panic!("failed complete profile must not invoke atomic work");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            })
            .await
            .unwrap_err();
            assert!(matches!(error, PgAtomicError::Begin(_)));
            super::replacement(&fixture.pool, previous).await?;
            let previous: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&fixture.pool)
                .await?;
            let error = PgReadOnlySnapshot::inspect_profiled(&fixture.pool, &profile, async |_| {
                panic!("failed complete profile must not invoke inspection");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            })
            .await
            .unwrap_err();
            assert!(matches!(error, PgSnapshotError::Transaction(_)));
            super::replacement(&fixture.pool, previous).await?;
        }
        policy(&login, &login)?
            .reset_and_apply(&mut fixture.observer)
            .await?;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn mixed_case_profile_settings_preserve_values() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        policy(&login, &login)?
            .with_setting("APP.TENANT", "Tenant-Mixed-Case")?
            .with_setting("TimeZone", "UTC")?
            .with_setting("APPLICATION_NAME", "Mixed-Case-Service")?
            .reset_and_apply(&mut fixture.observer).await?;
        let values: (String, String, String) = sqlx::query_as(
            "SELECT current_setting('app.tenant'), current_setting('timezone'), current_setting('application_name')"
        ).fetch_one(&mut fixture.observer).await?;
        require(values == ("Tenant-Mixed-Case".into(), "UTC".into(), "Mixed-Case-Service".into()), "profile altered setting values")?;
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[derive(Clone, Default)]
struct HookEvents(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for HookEvents {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target().contains("pool") {
            let mut fields = Fields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }
}
struct Fields(String);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        write!(self.0, " {}={value:?}", field.name()).unwrap();
    }
}

async fn hook_logging(before_acquire: bool) -> Result {
    let fixture = fixture().await?;
    let body = async {
        let options = fixture.pool.connect_options();
        let login = options.get_username();
        let profile = Arc::new(policy(login, login)?.with_setting("timezone", MARKER)?);
        let connections = Arc::new(AtomicUsize::new(0));
        let calls = connections.clone();
        let after_profile = profile.clone();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(Duration::from_millis(500))
            .after_connect(move |connection, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                let profile = after_profile.clone();
                Box::pin(async move {
                    if before_acquire {
                        Ok(())
                    } else {
                        profile.reset_and_apply(connection).await
                    }
                })
            })
            .before_acquire(move |connection, _| {
                let profile = profile.clone();
                Box::pin(async move { profile.reset_and_apply(connection).await.map(|()| true) })
            })
            .connect_lazy_with((*options).clone());
        if before_acquire {
            // SQLx only calls before_acquire for an idle connection, not a new one.
            drop(bounded(pool.acquire()).await??);
            bounded(async {
                while pool.num_idle() != 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
        }
        let events = HookEvents::default();
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(events.clone()));
        let acquire = tracing::dispatcher::with_default(&dispatch, || {
            batter_core::telemetry::with_current_dispatch(pool.acquire())
        });
        let outcome = bounded(acquire).await?;
        let expected_outcome = if before_acquire {
            outcome.is_ok() && connections.load(Ordering::SeqCst) == 2
        } else {
            matches!(&outcome, Err(sqlx::Error::PoolTimedOut))
        };
        drop(outcome);
        bounded(pool.close()).await?;
        require(
            expected_outcome,
            "SQLx hook rejection/replacement behavior changed",
        )?;
        let observed = events.0.lock().unwrap().join("\n");
        let hook = if before_acquire {
            "error from `before_acquire`"
        } else {
            "error returned from after_connect"
        };
        require(
            observed.contains(hook),
            "expected SQLx hook error was not logged",
        )?;
        require(
            !observed.contains(MARKER),
            "SQLx hook logging disclosed profile marker",
        )?;
        require(
            observed.contains("PostgreSQL operation failed"),
            "hook log omitted safe diagnostic",
        )?;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profile_after_connect_logging_redacts_setting_values() -> Result {
    hook_logging(false).await
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn profile_before_acquire_logging_redacts_setting_values() -> Result {
    hook_logging(true).await
}
