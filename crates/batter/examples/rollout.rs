//! A release flag and a kill switch: declared mode from settings, or the
//! audited PostgreSQL control plane when `DATABASE_URL` is set.
//!
//! Declared mode resolves `FLAG_NEW_CHECKOUT` and `FLAG_OUTBOUND_HALT` once
//! and changes them by restart. `rollout migrate` is the finite migration
//! command: run it with `DATABASE_URL` naming a login that owns the flags
//! schema, before the build that needs it serves. Serving with `DATABASE_URL`
//! never migrates: it registers the control plane over the pool `pool_in`
//! owns, under a serving login granted only `FlagsRole::ServingProcess`, with a
//! last-good file in `FLAGS_LAST_GOOD_DIR` (default: the system temporary
//! directory), and serves on defaults, or on its last-good file, through an
//! outage. In both modes a maintenance run is gated by the kill switch.
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "rollout/tests.rs"]
mod tests;

use std::{
    ffi::OsString,
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use batter::{
    BoxError,
    cleanup::CleanupBudget,
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
    periodic::PeriodicFailure,
    settings::SettingsSource,
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
    let days = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / 86_400)?;
    // Howard Hinnant's days-to-civil conversion.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Ok(CivilDate::new(
        u16::try_from(year)?,
        u8::try_from(month)?,
        u8::try_from(day)?,
    )?)
}

fn supervisor() -> Result<Supervisor, BoxError> {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second * 5, second * 5, second * 5)?;
    Ok(Supervisor::new(ShutdownBudget::new(
        second * 5,
        second * 5,
        second * 5,
        cleanup,
    )?))
}

/// Registers the maintenance run the kill switch gates. Core counts runs
/// that did their work as successes and the runs the switch kept out as skips.
pub(crate) fn gate_maintenance(
    flags: &Flags,
    supervisor: &mut Supervisor,
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
    let mut supervisor = supervisor()?;
    let maintenance = gate_maintenance(&flags, &mut supervisor, &handles)?;
    let running = supervisor.start();
    running
        .status()
        .wait_ready()
        .await
        .map_err(|readiness| format!("not ready: {readiness:?}"))?;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    running.shutdown_checked().await?;
    // Skipped runs did no work, so they are not successes.
    Ok(maintenance.snapshot().succeeded)
}

/// The finite migration command, under a login that owns the flags schema.
async fn migrate(url: &str) -> Result<(), BoxError> {
    let options: PgConnectOptions = url.parse()?;
    let second = Duration::from_secs(1);
    let timing = StoreTiming::new(second * 2, second * 2, second, second * 2)?;
    let migrator = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy_with(options);
    let migrated = PooledStore::new(migrator.clone(), timing)
        .migrate(tokio::time::Instant::now() + second * 30)
        .await;
    migrator.close().await;
    migrated?;
    println!("flags schema migrated");
    Ok(())
}

/// Control-plane mode: an audited store decides, refreshed every poll. It
/// never migrates and connects lazily, so a restart during an outage serves
/// its last-good file or the declared base instead of failing.
async fn run_control_plane(settings: &SettingsSource, url: &str) -> Result<(), BoxError> {
    let options: PgConnectOptions = url.parse()?;
    let second = Duration::from_secs(1);
    let timing = StoreTiming::new(second * 2, second * 2, second, second * 2)?;
    let (resolved, handles) = resolve(settings, today()?)?;
    let mut supervisor = supervisor()?;
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
        supervisor.reserve_cleanup("database")?,
        PgPoolOptions::new().max_connections(4),
        options,
        profile,
    )?;
    let directory = settings
        .text("FLAGS_LAST_GOOD_DIR")?
        .map_or_else(std::env::temp_dir, Into::into);
    let last_good = SnapshotDirectory::open(SnapshotFilePath::new(directory, "rollout.bfsn")?)?;
    let policy = LoadPolicy::start_on_defaults(RefreshTiming::new(second * 5, second * 60)?)
        .with_last_good(last_good);
    let presence = Presence::new(
        ApplicationName::new("rollout")?,
        BuildId::new(env!("CARGO_PKG_VERSION"))?,
        HeartbeatCadence::new(second * 5, second * 600)?,
    );
    let flags = register_in(
        &mut supervisor,
        ControlPlaneStore::new(&pool, timing),
        resolved,
        policy,
        presence,
    )?;
    println!("startup file: {:?}", flags.startup_file());
    gate_maintenance(&flags, &mut supervisor, &handles)?;
    let running = supervisor.start();
    // The serving work; whatever it returns, the shutdown that runs the stop
    // marker and closes the pool is awaited before it is reported.
    let served = async {
        running
            .status()
            .wait_ready()
            .await
            .map_err(|readiness| format!("not ready: {readiness:?}"))?;
        tokio::time::sleep(second * 3).await;
        println!("flags status: {:?}", flags.status());
        Ok::<(), BoxError>(())
    }
    .await;
    let stopped = running.shutdown_checked().await;
    served?;
    stopped?;
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
            (Some("migrate"), Some(url)) => migrate(url).await,
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
            eprintln!("rollout: {error}");
            ExitCode::FAILURE
        }
    }
}
