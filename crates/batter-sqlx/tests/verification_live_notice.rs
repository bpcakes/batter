use super::support::{Result, require};
use super::{AuthorityFixture, RolePolicy, names_policy};
use std::sync::{Arc, Mutex};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

#[derive(Clone)]
struct Notices(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for Notices {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() == "sqlx::postgres::notice"
            && *event.metadata().level() == tracing::Level::WARN
        {
            event.record(&mut Capture(&mut self.0.lock().unwrap()));
        }
    }
}

struct Capture<'a>(&'a mut Vec<String>);
impl Visit for Capture<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }
}

#[tokio::test]
#[ignore = "external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn verification_idle_reset_has_one_expected_native_notice() -> Result {
    AuthorityFixture::create()
        .await?
        .run(|fixture| {
            Box::pin(async move {
                let names = fixture.names.clone();
                let pool = fixture.login(&names.login_a).await?;
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
                let authority = policy.authority.clone().build()?;
                let notices = Notices(Arc::new(Mutex::new(Vec::new())));
                let dispatch =
                    tracing::Dispatch::new(tracing_subscriber::registry().with(notices.clone()));
                for mode in 0..3 {
                    notices.0.lock().unwrap().clear();
                    let context = batter::operation::OperationContext::new(
                        std::time::Duration::from_secs(10),
                    )?;
                    let operation = tracing::dispatcher::with_default(&dispatch, || {
                        batter::telemetry::with_current_dispatch(async {
                            match mode {
                                0 => {
                                    let plan =
                                        batter_sqlx::verification::VerificationPlan::migrations(
                                            &policy.migration,
                                        )
                                        .with_authority(&authority)
                                        .expect("notice plan has one authority component");
                                    batter_sqlx::verification::verify(&pool, &context, plan).await
                                }
                                1 => {
                                    batter_sqlx::verification::verify_migrations(
                                        &pool,
                                        &context,
                                        &policy.migration,
                                    )
                                    .await
                                }
                                _ => {
                                    batter_sqlx::verification::verify_authority(
                                        &pool, &context, &authority,
                                    )
                                    .await
                                }
                            }
                        })
                    });
                    let report = operation.await?;
                    require(
                        report.supported().contains(
                            &batter_sqlx::verification::SupportedSurface::MigrationLedger,
                        ) == (mode != 2),
                        "verification mode reported the wrong ledger coverage",
                    )?;
                    if mode == 1 {
                        require(
                            report.is_within_declared_policy(),
                            "migration-only verification rejected the fixture ledger",
                        )?;
                    }
                    let observed = notices.0.lock().unwrap().clone();
                    require(
                        observed.len() == 1
                            && observed[0].contains("there is no transaction in progress"),
                        "idle reset did not retain exactly the expected PostgreSQL warning",
                    )?;
                }
                pool.close().await;
                Ok(())
            })
        })
        .await
}
