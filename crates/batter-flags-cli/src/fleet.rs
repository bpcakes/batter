//! The fleet reads: every process, the declarers of a key, and whether a
//! version reached every Live process.

use std::collections::BTreeMap;

use batter_flags_core::{FlagKey, Version};
use batter_flags_postgres::{FleetProcess, PooledStore, ProcessState, Rollout};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::{
    commands::{snapshot_text, state_text},
    exit::Exit,
    output::Report,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn process(process: &FleetProcess) -> Value {
    let (source, version) = snapshot_text(process.snapshot());
    json!({
        "process": process.process().as_uuid().to_string(),
        "application": process.application(),
        "environment": process.environment(),
        "build": process.build(),
        "state": state_text(process.state()),
        "snapshot_source": source,
        "snapshot_version": version,
        "declared_revision": hex(process.declared_revision()),
        "held": process.held().keys().iter().map(FlagKey::as_str).collect::<Vec<_>>(),
        "held_overflow": process.held().overflow(),
        "last_seen_at": process.last_seen_at().to_rfc3339(),
    })
}

pub(crate) async fn report<F: From<batter_flags_postgres::StoreError>>(
    store: &PooledStore,
    deadline: Instant,
    version: Option<u64>,
    flag: Option<&FlagKey>,
) -> Result<Report, F> {
    if let Some(key) = flag {
        let fleet = store.fleet_declaring(key, deadline).await?;
        let declarers: Vec<Value> = fleet
            .processes()
            .iter()
            .map(|p| json!({"build": p.build(), "presence": state_text(p.state()), "process": p.process().as_uuid().to_string()}))
            .collect();
        return Ok(Report::new(
            "fleet",
            Exit::Ok,
            "ok",
            format!("{} processes declare {}", declarers.len(), key.as_str()),
        )
        .with_data(json!({"head": fleet.head().get(), "declarers": declarers})));
    }
    let fleet = store.fleet(deadline).await?;
    let Some(version) = version else {
        let processes: Vec<Value> = fleet.processes().iter().map(process).collect();
        return Ok(Report::new(
            "fleet",
            Exit::Ok,
            "ok",
            format!("{} processes", processes.len()),
        )
        .with_data(json!({"head": fleet.head().get(), "processes": processes})));
    };
    // Live processes grouped by the rule-set triple; Silent and Stopped apart.
    let mut groups: BTreeMap<(String, Option<u64>, String), usize> = BTreeMap::new();
    let mut others = Vec::new();
    for p in fleet.processes() {
        match p.state() {
            ProcessState::Live => {
                let (_, applied) = snapshot_text(p.snapshot());
                *groups
                    .entry((p.build().to_owned(), applied, hex(p.declared_revision())))
                    .or_default() += 1;
            }
            ProcessState::Silent | ProcessState::Stopped => others.push(process(p)),
        }
    }
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|((build, applied, revision), count)| json!({"build": build, "snapshot_version": applied, "declared_revision": revision, "processes": count}))
        .collect();
    let (message, reached) = match fleet.rollout(Version::new(version)) {
        Rollout::Reached { live } => (
            format!("version {version} reached all {live} Live processes"),
            json!({"reached": true, "live": live.get()}),
        ),
        Rollout::Pending { reached, behind } => (
            format!(
                "version {version} reached {reached} Live processes; {} have not applied it",
                behind.len()
            ),
            json!({"reached": false, "applied": reached, "behind": behind.iter().map(|p| p.as_uuid().to_string()).collect::<Vec<_>>()}),
        ),
        Rollout::NoLiveProcess => (
            format!("no process is Live, so no process applied version {version}"),
            json!({"reached": false, "live": 0}),
        ),
    };
    Ok(Report::new("fleet", Exit::Ok, "ok", message)
        .with_data(json!({"head": fleet.head().get(), "rollout": reached, "live_groups": groups, "not_live": others})))
}

/// Adds the warning a released switch needs: Live processes whose settings
/// floor holds it engaged stay engaged. A process whose list overflowed
/// counts as possibly engaged.
pub(crate) async fn floor_warning(
    store: &PooledStore,
    deadline: Instant,
    switch: &FlagKey,
    report: Report,
) -> Report {
    let Ok(fleet) = store.fleet(deadline).await else {
        return report.warn("the fleet could not be read to count processes whose settings floor keeps the switch engaged");
    };
    let floored = fleet
        .processes()
        .iter()
        .filter(|p| p.state() == ProcessState::Live)
        .filter(|p| p.floor_engaged().keys().contains(switch) || p.floor_engaged().overflow() > 0)
        .count();
    if floored == 0 {
        return report;
    }
    report.warn(format!(
        "{floored} Live processes keep {} engaged by their settings floor",
        switch.as_str()
    ))
}
