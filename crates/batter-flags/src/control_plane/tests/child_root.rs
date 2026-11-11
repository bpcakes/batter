//! A whole root with its database unreachable at boot, run as a child of
//! the test binary (re-executed with a marker, so no executable root is
//! added): the protected path from `pool_in` to readiness.

use std::{process::Command, time::Duration};

use batter_flags_core::SwitchState;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use super::super::{
    ControlPlaneStore, Gate, LoadPolicy, Served, SnapshotDirectory, SnapshotFilePath, register_in,
};
use super::support::*;
use crate::low_level::capture_now;

const MODE: &str = "BATTER_FLAGS_CHILD_ROOT";
const DIRECTORY: &str = "BATTER_FLAGS_CHILD_DIRECTORY";
const CHILD: &str = "control_plane::tests::child_root::child";

/// Runs only as the child: registers through `register_in` with a store no
/// connection can reach, starts the supervisor, and prints what it serves.
#[test]
fn child() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let (input, handles) = resolved(&[("FLAG_HALT", "engaged")]);
        let mut supervisor = supervisor();
        let options = PgConnectOptions::new_without_pgpass()
            .host("127.0.0.1")
            .port(1)
            .username("flags")
            .database("flags");
        let profile = batter_sqlx::PgSessionProfile::new(
            "flags",
            "flags",
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        let pool = batter_sqlx::pool_in(
            supervisor.reserve_cleanup("database").unwrap(),
            PgPoolOptions::new().max_connections(2),
            options,
            profile,
        )
        .unwrap();
        let policy = match mode.as_str() {
            "file" => {
                let directory = std::env::var(DIRECTORY).unwrap();
                let path = SnapshotFilePath::new(directory, "last-good.bfsn").unwrap();
                LoadPolicy::start_on_defaults(file_timing())
                    .with_last_good(SnapshotDirectory::open(path).unwrap())
            }
            _ => LoadPolicy::start_on_defaults(timing()),
        };
        let flags = register_in(
            &mut supervisor,
            ControlPlaneStore::new(&pool, store_timing()),
            input,
            policy,
            presence(),
        )
        .unwrap();
        let running = supervisor.start();
        running.status().wait_ready().await.unwrap();
        // The probe a root installs: lifecycle readiness narrowed by the
        // flags condition, asked as an HTTP readiness probe asks it.
        let (condition, satisfied) = flags.readiness_condition(Gate::Published);
        let probe = batter_core::readiness::ReadinessEvaluator::lifecycle_only(running.status())
            .with_condition(condition, satisfied);
        let published = || probe.decision().is_ready();
        let view = capture_now(&flags);
        println!(
            "child: ready=true published={} halt={} stale_halt={} persisted={}",
            published(),
            view.switch(&handles.halt).value().as_str(),
            view.switch(&handles.stale_halt).value().as_str(),
            matches!(flags.status().served, Served::PersistedFile { .. }),
        );
        let _ = running.shutdown().await;
    });
}

fn run_child(mode: &str, directory: Option<&std::path::Path>) -> String {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
        .env(MODE, mode);
    if let Some(directory) = directory {
        command.env(DIRECTORY, directory);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn a_boot_time_outage_without_a_file_serves_the_base_with_the_floor_and_is_not_published() {
    let stdout = run_child("base", None);
    // Ready on defaults, the settings floor engaged, and `Gate::Published`
    // declining the bare base.
    assert!(
        stdout.contains(
            "child: ready=true published=false halt=engaged stale_halt=released persisted=false"
        ),
        "{stdout}"
    );
    let _ = SwitchState::Engaged;
}

#[test]
fn an_outage_restart_with_a_usable_file_serves_it_and_is_published() {
    let directory =
        std::env::temp_dir().join(format!("batter-flags-child-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&directory).unwrap();
    let (input, _) = resolved(&[]);
    let (base, _) = input.into_parts();
    let entry = |key: &str, state: &str| crate::last_good::format::FileEntry {
        key: key.into(),
        kind: "switch".into(),
        value_type: "switch".into(),
        fingerprint: *base
            .catalog()
            .declaration(key)
            .unwrap()
            .fingerprint()
            .as_bytes(),
        updated_version: 40,
        value: crate::last_good::format::EntryValue::State(format!(
            "{{\"encoding\":1,\"switch\":\"{state}\"}}"
        )),
    };
    let file = crate::last_good::format::FileSnapshot {
        scope: crate::last_good::scope_label(&base),
        catalog_digest: *base.catalog().digest().as_bytes(),
        manifest_digest: *base.catalog().manifest_digest().as_bytes(),
        version: 40,
        entries: vec![entry("halt", "released"), entry("stale_halt", "engaged")],
    };
    std::fs::write(
        directory.join("last-good.bfsn"),
        crate::last_good::format::encode(&file).unwrap(),
    )
    .unwrap();
    let stdout = run_child("file", Some(&directory));
    let _ = std::fs::remove_dir_all(&directory);
    // The file's engaged switch stays engaged, the floor still engages halt,
    // and a published file satisfies `Gate::Published`.
    assert!(
        stdout.contains(
            "child: ready=true published=true halt=engaged stale_halt=engaged persisted=true"
        ),
        "{stdout}"
    );
}
