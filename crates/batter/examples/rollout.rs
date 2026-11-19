//! A release flag and a kill switch: declared mode from settings, or the
//! audited PostgreSQL control plane when `DATABASE_URL` is set.
//!
//! Declared mode resolves `FLAG_NEW_CHECKOUT` and `FLAG_OUTBOUND_HALT` once
//! and changes them by restart. `rollout migrate` is the finite migration
//! command: run it with `DATABASE_URL` naming a login that owns the flags
//! schema, before the build that needs it serves. Serving with `DATABASE_URL`
//! never migrates: it registers the control plane over the pool `pool_in`
//! owns, under a serving login granted only `FlagsRole::ServingProcess`, with a
//! last-good file in the required `FLAGS_LAST_GOOD_DIR`, a directory this
//! process owns, and serves on defaults, or on its last-good file, through an
//! outage. In both modes a maintenance run is gated by the kill switch.
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "rollout/tests.rs"]
mod tests;

use std::{
    ffi::OsString,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use batter::{
    BoxError,
    cleanup::CleanupBudget,
    command::{Command, CommandCause, CommandFailure},
    flags::{
        Flags,
        control_plane::{
            ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, LoadPolicy, Presence,
            RefreshTiming, SnapshotDirectory, SnapshotFilePath, register_in,
        },
        core::{
            CatalogBuilder, CivilDate, Expiry, KillSwitch, ReleaseFlag, SafeState, SettingsPrefix,
            StaleRule, Toggle,
        },
        declared::{self, Resolved},
        periodic::{GatedPeriodicPolicy, GatedPeriodicReader},
        postgres::{PooledStore, StoreTiming},
    },
    lifecycle::{ShutdownBudget, Supervisor},
    operation::OperationOwner,
    periodic::PeriodicFailure,
    registration::RegistrationTarget,
    service::{self, NoDiagnostics, ServiceCompletion, ServiceOutcome},
    settings::SettingsSource,
    startup::{Initializer, Startup},
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// The handles the application evaluates through.
pub(crate) struct Handles {
    pub(crate) new_checkout: ReleaseFlag<Toggle>,
    pub(crate) outbound_halt: KillSwitch,
}

/// The one catalog of this application, resolved against `settings`.
pub(crate) fn resolve(
    settings: &SettingsSource,
    today: CivilDate,
) -> Result<(Resolved, Handles), BoxError> {
    let (mut catalog, serving, hosted) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
    let new_checkout = catalog
        .release_flag(
            "new_checkout",
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 6, 30)?),
        )
        .declare()?;
    let outbound_halt = catalog
        .kill_switch(
            "outbound_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .declare()?;
    let resolved = declared::resolve(catalog.build(), settings, serving, hosted, today)?;
    Ok((
        resolved,
        Handles {
            new_checkout,
            outbound_halt,
        },
    ))
}

/// Today's civil date in UTC, from the system clock.
pub(crate) fn today() -> Result<CivilDate, BoxError> {
    Ok(CivilDate::from_unix_time(
        SystemTime::now().duration_since(UNIX_EPOCH)?,
    )?)
}

const SECOND: Duration = Duration::from_secs(1);

fn cleanup() -> Result<CleanupBudget, batter::ConfigurationError> {
    CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5)
}

struct ServiceFailure(ServiceCompletion<BoxError, ()>);

impl std::fmt::Display for ServiceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Err(readiness) = self.0.readiness() {
            write!(f, "{readiness}. ")?;
        }
        match self.0.service() {
            ServiceOutcome::StartupFailed(error) => write!(f, "{error}"),
            ServiceOutcome::Shutdown(Err(error)) => write!(f, "{error}"),
            ServiceOutcome::Coordinator(_) => f.write_str("service coordinator failed"),
            ServiceOutcome::Shutdown(Ok(_)) => f.write_str("service stopped"),
        }
    }
}

impl std::fmt::Debug for ServiceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ServiceFailure {}

/// Both modes transfer their initialized readers only after readiness. A failed
/// handoff retains the service's full completion, including cleanup failures.
async fn serve_for<T>(
    initialize: impl AsyncFnOnce(&mut batter::startup::ProtectedStartupScope) -> Result<T, BoxError>
    + Initializer<BoxError, Output = T>,
    window: Duration,
    observe: impl FnOnce(&T),
) -> Result<T, BoxError> {
    let cleanup = cleanup()?;
    let supervisor = Supervisor::new(ShutdownBudget::new(
        SECOND * 5,
        SECOND * 5,
        SECOND * 5,
        cleanup,
    )?);
    let startup = Startup::protected(
        supervisor,
        OperationOwner::new(SECOND * 30)?.into_context(),
        cleanup,
        initialize,
    );
    let (owner, output) = service::start_with_readiness(startup, NoDiagnostics, None)
        .into_ready()
        .await
        .map_err(|failure| ServiceFailure(failure.completion().clone()))?;
    tokio::time::sleep(window).await;
    observe(&output);
    owner.request_shutdown();
    let completion = owner.wait().await;
    if matches!(completion.service(), ServiceOutcome::Shutdown(Ok(_))) {
        Ok(output)
    } else {
        Err(Box::new(ServiceFailure(completion)))
    }
}

/// Registers the maintenance run the kill switch gates. Core counts runs
/// that did their work as successes and the runs the switch kept out as skips.
pub(crate) fn gate_maintenance(
    flags: &Flags,
    supervisor: &mut impl RegistrationTarget,
    handles: &Handles,
) -> Result<GatedPeriodicReader, BoxError> {
    let new_checkout = handles.new_checkout.clone();
    flags
        .gate_periodic_in(
            supervisor,
            "rollout.maintenance",
            GatedPeriodicPolicy::new(Duration::from_secs(1), Duration::from_secs(1))?,
            &handles.outbound_halt,
            move |_, view| {
                // The run's one view: the switch let it run, and the flag it reads
                // cannot change under it.
                let checkout = view.evaluate(&new_checkout).value();
                println!(
                    "maintenance ran with new_checkout={}",
                    checkout == Toggle::On
                );
                std::future::ready(Ok::<(), PeriodicFailure<std::io::Error>>(()))
            },
        )
        .map_err(Into::into)
}

/// Declared mode: the settings decide until the next restart.
pub(crate) async fn run_declared(settings: &SettingsSource) -> Result<u64, BoxError> {
    let (resolved, handles) = resolve(settings, today()?)?;
    println!("{}", resolved.report());
    let flags = resolved.into_flags();
    let maintenance = serve_for(
        async move |scope: &mut batter::startup::ProtectedStartupScope| {
            gate_maintenance(&flags, scope, &handles)
        },
        Duration::from_millis(2_500),
        |_| {},
    )
    .await?;
    Ok(maintenance.snapshot().succeeded)
}

#[derive(Debug, thiserror::Error)]
enum MigrationError {
    #[error("migration pool registration failed")]
    Registration(#[from] batter::RegistrationError),
    #[error("flags migration failed")]
    Store(#[from] batter::flags::postgres::StoreError),
}

/// Prepare a finite command. The root selects the Unix signal policy.
fn migration(
    url: &str,
) -> Result<
    Command<
        impl for<'a> FnOnce(
            &'a mut batter::command::CommandScope,
        ) -> batter::command::CommandFuture<'a, (), MigrationError>
        + Send
        + 'static,
    >,
    BoxError,
> {
    let options: PgConnectOptions = url.parse()?;
    let timing = StoreTiming::new(SECOND * 2, SECOND * 2, SECOND, SECOND * 2)?;
    Ok(Command::new(
        OperationOwner::new(SECOND * 30)?.into_context(),
        cleanup()?,
        move |scope| {
            Box::pin(async move {
                scope.stage("flags.migrate")?;
                let slot = scope.reserve_cleanup("migration.pool")?;
                let pool = PgPoolOptions::new()
                    .max_connections(1)
                    .connect_lazy_with(options);
                let store = PooledStore::new(pool.clone(), timing);
                slot.register(move || async move {
                    pool.close().await;
                    Ok(())
                });
                store.migrate(scope.context().deadline()).await?;
                Ok(())
            })
        },
    ))
}

/// Control-plane mode: an audited store decides, refreshed every poll. It
/// never migrates and connects lazily, so a restart during an outage serves
/// its last-good file or the declared base instead of failing.
async fn run_control_plane(settings: &SettingsSource, url: &str) -> Result<(), BoxError> {
    let options: PgConnectOptions = url.parse()?;
    let second = Duration::from_secs(1);
    let timing = StoreTiming::new(second * 2, second * 2, second, second * 2)?;
    let (resolved, handles) = resolve(settings, today()?)?;
    let directory = PathBuf::from(settings.required("FLAGS_LAST_GOOD_DIR")?);
    serve_for(
        async move |scope: &mut batter::startup::ProtectedStartupScope| {
            let login = options.get_username().to_owned();
            let profile = batter::sqlx::PgSessionProfile::new(
                &login,
                &login,
                vec!["public".into()],
                Duration::ZERO,
                Duration::ZERO,
            )?;
            // Registered first, so the pool closes after the flags stop marker.
            let pool = batter::sqlx::pool_in(
                scope.reserve_cleanup("database")?,
                PgPoolOptions::new().max_connections(4),
                options,
                profile,
            )?;
            // Required: the file restores kill-switch states at startup, so it lives
            // in a directory this process owns, never a shared temporary one.
            let last_good =
                SnapshotDirectory::open(SnapshotFilePath::new(directory, "rollout.bfsn")?)?;
            let policy =
                LoadPolicy::start_on_defaults(RefreshTiming::new(second * 5, second * 60)?)
                    .with_last_good(last_good);
            let presence = Presence::new(
                ApplicationName::new("rollout")?,
                BuildId::new(env!("CARGO_PKG_VERSION"))?,
                HeartbeatCadence::new(second * 5, second * 600)?,
            );
            let flags = register_in(
                scope,
                ControlPlaneStore::new(&pool, timing),
                resolved,
                policy,
                presence,
            )?;
            println!("startup file: {:?}", flags.startup_file());
            gate_maintenance(&flags, scope, &handles)?;
            Ok(flags)
        },
        second * 3,
        |flags| println!("flags status: {:?}", flags.status()),
    )
    .await?;
    Ok(())
}

fn main() -> ExitCode {
    std::panic::set_hook(batter::redacting_panic_hook());
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        eprintln!("rollout: the runtime could not start");
        return ExitCode::FAILURE;
    };
    let outcome = runtime.block_on(async {
        let environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        let settings = SettingsSource::from_pairs(environment)?;
        let command = std::env::args().nth(1);
        match (command.as_deref(), settings.text("DATABASE_URL")?) {
            (Some("migrate"), Some(url)) => {
                migration(url)?
                    .start_with_unix_signals()?
                    .finish()
                    .await?
                    .into_value();
                println!("flags schema migrated");
                Ok(())
            }
            (Some("migrate"), None) => Err("rollout migrate needs DATABASE_URL".into()),
            (Some(_), _) => Err("usage: rollout [migrate]".into()),
            (None, Some(url)) => run_control_plane(&settings, url).await,
            (None, None) => run_declared(&settings).await.map(|_| ()),
        }
    });
    // A persist the drain abandoned may still be writing; give it a bound.
    runtime.shutdown_timeout(Duration::from_secs(5));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(CommandFailure::Report(report)) =
                error.downcast_ref::<CommandFailure<(), MigrationError>>()
                && (matches!(report.work, Err(CommandCause::Interrupted(_)))
                    || report.interruption_after_work.is_some())
            {
                eprintln!("rollout: migration interrupted. {error}");
            } else {
                eprintln!("rollout: {error}");
            }
            ExitCode::FAILURE
        }
    }
}
