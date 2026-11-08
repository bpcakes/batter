//! The flags steps of the single-facade consumer: declared mode, then the
//! audited control plane, reached through `batter::flags` alone.

use std::time::Duration;

use batter::cleanup::CleanupBudget;
use batter::flags::control_plane::{
    ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, LoadPolicy, Presence,
    RefreshTiming, SnapshotStatus, register_in,
};
use batter::flags::core::{
    CatalogBuilder, CivilDate, Expiry, KillSwitch, Pinned, PinnedResolution, ReleaseFlag,
    SafeState, SettingsPrefix, StaleRule, SwitchState, Toggle,
};
use batter::flags::declared::{self, Resolved};
use batter::flags::low_level::capture_now;
use batter::flags::postgres::{
    Actor, Change, ChangeReason, ExplicitValue, PooledStore, Precondition, StoreTiming, Write,
    WriteOutcome,
};
use batter::lifecycle::{ShutdownBudget, Supervisor};
use batter::settings::SettingsSource;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::time::Instant;

use crate::Outcome;

const SECOND: Duration = Duration::from_secs(1);

struct Handles {
    express: ReleaseFlag<Toggle>,
    halt: KillSwitch,
}

/// One catalog: a pinnable release flag governed by a kill switch.
fn resolve(settings: &[(&str, &str)]) -> Outcome<(Resolved, Handles)> {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    let serving = catalog.process_kind("serving")?;
    let hosted = catalog.environment_class("hosted")?;
    let halt = catalog
        .kill_switch(
            "checkout_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .declare()?;
    let express = catalog
        .release_flag(
            "express_checkout",
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 6, 30)?),
        )
        .governed_by(&halt)
        .declare()?;
    let source = SettingsSource::from_pairs(
        settings
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    )?;
    let resolved = declared::resolve(
        catalog.build()?,
        &source,
        serving,
        hosted,
        CivilDate::new(2026, 11, 8)?,
    )?;
    Ok((resolved, Handles { express, halt }))
}

fn change() -> Outcome<Change> {
    Ok(Change::new(
        Actor::new("facade-consumer")?,
        ChangeReason::new("single-facade acceptance")?,
        Precondition::Unconditional,
    ))
}

/// Runs every flags step against the database `options` names.
pub(crate) async fn run(options: PgConnectOptions) -> Outcome {
    // Declared mode: resolve once, pin a decision, keep the pin in a row.
    let (resolved, handles) = resolve(&[("FLAG_EXPRESS_CHECKOUT", "on")])?;
    println!("facade consumer: declared catalog resolved");
    let declared = resolved.into_flags();
    let pin = capture_now(&declared).pin(&handles.express);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await?;
    sqlx::query("CREATE TABLE consumer_pins (id integer PRIMARY KEY, pin jsonb NOT NULL)")
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO consumer_pins (id, pin) VALUES (1, $1)")
        .bind(serde_json::to_value(&pin)?)
        .execute(&pool)
        .await?;
    println!("facade consumer: pinned decision stored in row");

    // The control plane: migrate as a finite step, then seed the store.
    let timing = StoreTiming::new(SECOND * 3, SECOND * 3, SECOND * 2, SECOND * 3)?;
    let store = PooledStore::new(pool.clone(), timing);
    store.migrate(Instant::now() + SECOND * 30).await?;
    let express = batter::flags::core::FlagKey::new("express_checkout")?;
    let halt = batter::flags::core::FlagKey::new("checkout_halt")?;
    let deadline = || Instant::now() + SECOND * 10;
    let seeded = store
        .write(
            Write::explicit(express, ExplicitValue::ReleaseToggle(Toggle::On), change()?),
            deadline(),
        )
        .await?;
    if !matches!(seeded, WriteOutcome::WrittenUndeclared { .. }) {
        return Err("the store did not record the seeded flag".into());
    }

    let (resolved, handles) = resolve(&[])?;
    let budget = CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5)?;
    let mut supervisor = Supervisor::new(ShutdownBudget::new(
        SECOND * 5,
        SECOND * 5,
        SECOND * 5,
        budget,
    )?);
    let login = options.get_username().to_owned();
    let profile = batter::sqlx::PgSessionProfile::new(
        &login,
        &login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    let shared = batter::sqlx::pool_in(
        supervisor.reserve_cleanup("database")?,
        PgPoolOptions::new().max_connections(4),
        options,
        profile,
    )?;
    let policy =
        LoadPolicy::unready_until_loaded(RefreshTiming::new(SECOND, SECOND * 20)?, SECOND * 60);
    let presence = Presence::new(
        ApplicationName::new("facade-consumer")?,
        BuildId::new("1.0.0+consumer")?,
        HeartbeatCadence::new(SECOND * 5, SECOND * 600)?,
    );
    let flags = register_in(
        &mut supervisor,
        ControlPlaneStore::new(&shared, timing),
        resolved,
        policy,
        presence,
    )?;
    let running = supervisor.start();
    // The steps; whatever they return, the shutdown that records the stop and
    // closes the pool is awaited, and both outcomes are reported.
    let steps = async {
        running
            .status()
            .wait_ready()
            .await
            .map_err(|readiness| format!("not ready: {readiness:?}"))?;
        if flags.status() != SnapshotStatus::Fresh
            || capture_now(&flags).evaluate(&handles.express).value() != Toggle::On
        {
            return Err("the control-plane snapshot was not published before readiness".into());
        }
        println!("facade consumer: control-plane snapshot loaded before readiness");

        // The pin from the row replays the declared decision.
        let stored: serde_json::Value =
            sqlx::query_scalar("SELECT pin FROM consumer_pins WHERE id = 1")
                .fetch_one(&pool)
                .await?;
        let pin: Pinned<Toggle> = serde_json::from_value(stored)?;
        match capture_now(&flags).resolve_pinned(&handles.express, &pin) {
            PinnedResolution::Honoured(decision) if decision.value() == Toggle::On => {}
            _ => return Err("the stored pin was not executed".into()),
        }
        println!("facade consumer: pinned decision executed");

        // Engaging the governing kill switch overrides the pin.
        let engaged = store
            .write(
                Write::explicit(halt, ExplicitValue::Switch(SwitchState::Engaged), change()?),
                deadline(),
            )
            .await?;
        if !matches!(
            engaged,
            WriteOutcome::Written { .. } | WriteOutcome::WrittenUndeclared { .. }
        ) {
            return Err("the store did not record the engaged switch".into());
        }
        let mut overridden = false;
        for _ in 0..50 {
            if let PinnedResolution::Overridden { decision, .. } =
                capture_now(&flags).resolve_pinned(&handles.express, &pin)
            {
                overridden = decision.value() == Toggle::Off;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !overridden || capture_now(&flags).switch(&handles.halt).value() != SwitchState::Engaged {
            return Err("the kill switch did not override the pin".into());
        }
        println!("facade consumer: kill switch overrode a pin");

        // The heartbeat names this build and the version it applied.
        let mut recorded = false;
        for _ in 0..100 {
            let row: Option<(String, Option<i64>)> = sqlx::query_as(
                "SELECT build, snapshot_version FROM flags.flag_processes WHERE process_id = $1 AND snapshot_source = 'control_plane'",
            )
            .bind(flags.process().as_uuid())
            .fetch_optional(&pool)
            .await?;
            if row.is_some_and(|(build, version)| build == "1.0.0+consumer" && version.is_some()) {
                recorded = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !recorded {
            return Err("no heartbeat recorded this build and its version".into());
        }
        println!("facade consumer: heartbeat recorded build and version");
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let stopped = running.shutdown_checked().await;
    pool.close().await;
    steps?;
    stopped?;
    Ok(())
}
