//! PostgreSQL 18 fleet contracts: which process states each write rule and
//! the rollout answer count, over rows written by real heartbeats.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent contract assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
#[path = "support/presence.rs"]
mod support;
use batter_flags_core::Version;
use batter_flags_postgres::*;
use fixture::*;
use std::num::NonZeroUsize;
use support::*;

async fn beat(process: &mut ProcessPresence, snapshot: SnapshotReport) {
    process
        .heartbeat(&report(snapshot), deadline())
        .await
        .unwrap();
}

fn archive(name: &str) -> Archive {
    Archive::new(key(name), change(None))
}

fn builds(blocking: &[BlockingBuild]) -> Vec<(&str, Presence)> {
    blocking
        .iter()
        .map(|b| (b.build.as_str(), b.presence))
        .collect()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn only_a_live_control_plane_report_above_the_head_moves_the_next_version() {
    let f = Fixture::migrated().await;
    let ahead = || SnapshotReport::ControlPlane(Version::new(50));
    let mut silent = process(&f.store, Build::Base, "silent", 1, 600);
    let mut stopped = process(&f.store, Build::Base, "stopped", 60, 600);
    beat(&mut silent, ahead()).await;
    beat(&mut stopped, ahead()).await;
    stopped.stop(deadline()).await.unwrap();
    let mut file = process(&f.store, Build::Base, "file", 60, 600);
    let mut base = process(&f.store, Build::Base, "base", 60, 600);
    beat(&mut file, SnapshotReport::PersistedFile(Version::new(50))).await;
    beat(&mut base, SnapshotReport::NotLoaded).await;
    until_silent().await;

    // A persisted file, the declared base, a Silent and a Stopped process
    // above the head never move the next version.
    let written = f
        .store
        .write(set("fast_path", "on", None), deadline())
        .await
        .unwrap();
    assert!(
        matches!(
            written,
            WriteOutcome::Written {
                live_declaring: 2,
                ..
            }
        ),
        "{written:?}"
    );

    let mut live = process(&f.store, Build::Base, "live", 60, 600);
    beat(&mut live, ahead()).await;
    assert!(
        matches!(
            f.store
                .write(set("fast_path", "off", None), deadline())
                .await
                .unwrap(),
            WriteOutcome::Written { version, .. } if version == Version::new(51)
        ),
        "a Live process above the head moves the next version above itself"
    );
    // At or below the head, the same Live process moves nothing.
    beat(&mut live, SnapshotReport::ControlPlane(Version::new(51))).await;
    assert!(matches!(
        f.store.write(set("fast_path", "on", None), deadline()).await.unwrap(),
        WriteOutcome::Written { version, .. } if version == Version::new(52)
    ));
    f.close().await;
}

/// A report is the serving role's own claim: at or beyond the jump window
/// above the head it neither refuses a write nor moves its number, and just
/// inside the window it moves the write to the window's edge.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_report_at_or_beyond_the_jump_window_never_moves_or_refuses_a_write() {
    const WINDOW: u64 = MAX_VERSION_JUMP as u64;
    let f = Fixture::migrated().await;
    let mut wild = process(&f.store, Build::Base, "wild", 60, 600);
    let written = async |variant: &str| match f
        .store
        .write(set("fast_path", variant, None), deadline())
        .await
        .unwrap()
    {
        WriteOutcome::Written { version, .. } => version.get(),
        other => panic!("{other:?}"),
    };
    // The largest storable version: no write is refused and none jumps.
    beat(
        &mut wild,
        SnapshotReport::ControlPlane(Version::new(i64::MAX as u64)),
    )
    .await;
    assert_eq!(written("on").await, 1);
    // Exactly the window above the head is still outside it.
    beat(
        &mut wild,
        SnapshotReport::ControlPlane(Version::new(1 + WINDOW)),
    )
    .await;
    assert_eq!(written("off").await, 2);
    // One inside the window: the write lands at the window's edge, which is
    // the widest step the counter guard accepts.
    beat(
        &mut wild,
        SnapshotReport::ControlPlane(Version::new(2 + WINDOW - 1)),
    )
    .await;
    assert_eq!(written("on").await, 2 + WINDOW);
    assert_eq!(f.head().await, (2 + WINDOW) as i64);
    // A wild report never hides a genuine one inside the window.
    let mut genuine = process(&f.store, Build::Base, "genuine", 60, 600);
    beat(
        &mut wild,
        SnapshotReport::ControlPlane(Version::new(i64::MAX as u64)),
    )
    .await;
    beat(
        &mut genuine,
        SnapshotReport::ControlPlane(Version::new(2 + WINDOW + 50)),
    )
    .await;
    assert_eq!(written("off").await, 2 + WINDOW + 51);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn decoding_and_the_archive_gate_count_live_and_silent_declarers_only() {
    let f = Fixture::migrated().await;
    let mut live = process(&f.store, Build::Base, "live", 60, 600);
    let mut silent = process(&f.store, Build::Base, "silent", 1, 600);
    let mut stopped = process(&f.store, Build::Base, "stopped", 60, 600);
    for process in [&mut live, &mut silent, &mut stopped] {
        beat(process, SnapshotReport::NotLoaded).await;
    }
    stopped.stop(deadline()).await.unwrap();
    until_silent().await;

    let written = f
        .store
        .write(set("fast_path", "on", None), deadline())
        .await
        .unwrap();
    assert!(
        matches!(
            written,
            WriteOutcome::Written {
                live_declaring: 1,
                ..
            }
        ),
        "{written:?}"
    );
    // A declaration of another kind cannot decode the row: here the build's
    // catalog entry says mode where the stored row says release.
    sqlx::query("UPDATE flags.flag_catalog_entries SET kind = 'mode' WHERE key = 'fast_path'")
        .execute(&f.pool)
        .await
        .unwrap();
    let WriteOutcome::Refused(Refusal::UndecodableByLiveDeclaration { builds: blocking }) = f
        .store
        .write(set("fast_path", "off", None), deadline())
        .await
        .unwrap()
    else {
        panic!("an undecodable write is refused");
    };
    assert_eq!(
        builds(&blocking),
        [("live", Presence::Live), ("silent", Presence::Silent)]
    );
    sqlx::query("UPDATE flags.flag_catalog_entries SET kind = 'release' WHERE key = 'fast_path'")
        .execute(&f.pool)
        .await
        .unwrap();

    let WriteOutcome::Refused(Refusal::StillDeclared { builds: blocking }) = f
        .store
        .archive(archive("fast_path"), deadline())
        .await
        .unwrap()
    else {
        panic!("a declared key is not archived");
    };
    assert_eq!(
        builds(&blocking),
        [("live", Presence::Live), ("silent", Presence::Silent)]
    );

    // Once only Stopped rows declare it, the key is archived, and the write
    // that only Stopped and Silent rows declared is no longer Live-declared.
    live.stop(deadline()).await.unwrap();
    silent.stop(deadline()).await.unwrap();
    assert!(matches!(
        f.store
            .write(set("fast_path", "off", None), deadline())
            .await
            .unwrap(),
        WriteOutcome::WrittenUndeclared {
            basis: UndeclaredBasis::NoLiveDeclarer,
            ..
        }
    ));
    assert!(matches!(
        f.store
            .archive(archive("fast_path"), deadline())
            .await
            .unwrap(),
        WriteOutcome::WrittenUndeclared { .. }
    ));
    // A catalog that no process row references declares the key as orphaned.
    sqlx::query("DELETE FROM flags.flag_processes")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .write(
                Write::kill(
                    key("halt"),
                    batter_flags_core::SwitchState::Engaged,
                    change(None)
                ),
                deadline()
            )
            .await
            .unwrap(),
        WriteOutcome::WrittenUndeclared {
            basis: UndeclaredBasis::Orphaned,
            ..
        }
    ));
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_rollout_counts_only_live_control_plane_publications() {
    let f = Fixture::migrated().await;
    let v = Version::new;
    assert_eq!(
        f.store.fleet(deadline()).await.unwrap().rollout(v(1)),
        Rollout::NoLiveProcess
    );

    let mut early = process(&f.store, Build::Base, "a-early", 60, 600);
    let mut late = process(&f.store, Build::Extended, "b-late", 60, 600);
    let mut file = process(&f.store, Build::Base, "c-file", 60, 600);
    let mut silent = process(&f.store, Build::Base, "d-silent", 1, 600);
    let mut stopped = process(&f.store, Build::Base, "e-stopped", 60, 600);
    for process in [&mut early, &mut late, &mut file, &mut silent, &mut stopped] {
        beat(process, SnapshotReport::NotLoaded).await;
    }
    stopped.stop(deadline()).await.unwrap();
    // Five writes give the store a head every report below stays within.
    for variant in ["on", "off", "on", "off", "on"] {
        let written = f
            .store
            .write(set("fast_path", variant, None), deadline())
            .await
            .unwrap();
        assert!(
            matches!(written, WriteOutcome::Written { .. }),
            "{written:?}"
        );
    }
    beat(&mut early, SnapshotReport::ControlPlane(v(3))).await;
    beat(&mut late, SnapshotReport::ControlPlane(v(5))).await;
    beat(&mut file, SnapshotReport::PersistedFile(v(5))).await;
    until_silent().await;

    let fleet = f.store.fleet(deadline()).await.unwrap();
    assert_eq!(fleet.head(), v(5));
    assert_eq!(fleet.processes().len(), 5);
    assert_eq!(
        fleet.rollout(v(3)),
        Rollout::Pending {
            reached: 2,
            behind: vec![file.process()]
        },
        "a persisted file never counts as applied"
    );
    assert_eq!(
        fleet.rollout(v(4)),
        Rollout::Pending {
            reached: 1,
            behind: vec![early.process(), file.process()]
        }
    );
    beat(&mut early, SnapshotReport::ControlPlane(v(5))).await;
    beat(&mut file, SnapshotReport::ControlPlane(v(5))).await;
    let fleet = f.store.fleet(deadline()).await.unwrap();
    assert_eq!(
        fleet.rollout(v(5)),
        Rollout::Reached {
            live: NonZeroUsize::new(3).unwrap()
        }
    );

    let declaring = f
        .store
        .fleet_declaring(&key("checkout"), deadline())
        .await
        .unwrap();
    let names: Vec<_> = declaring
        .processes()
        .iter()
        .map(FleetProcess::build)
        .collect();
    assert_eq!(names, ["b-late"]);
    let states: Vec<_> = fleet.processes().iter().map(FleetProcess::state).collect();
    assert_eq!(
        states,
        [
            ProcessState::Live,
            ProcessState::Live,
            ProcessState::Live,
            ProcessState::Silent,
            ProcessState::Stopped
        ]
    );
    f.close().await;
}

/// A process reporting a version above the head applied a store this one is
/// not, such as the store before a restore: a report beyond the jump window
/// moves no write, the process refuses the write as a regression, and the
/// rollout lists it as behind instead of counting it as reached.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_report_above_the_head_never_counts_as_reached() {
    let f = Fixture::migrated().await;
    let mut wild = process(&f.store, Build::Base, "a-wild", 60, 600);
    let mut applied = process(&f.store, Build::Base, "b-applied", 60, 600);
    beat(
        &mut wild,
        SnapshotReport::ControlPlane(Version::new(i64::MAX as u64)),
    )
    .await;
    let WriteOutcome::Written { version, .. } = f
        .store
        .write(set("fast_path", "on", None), deadline())
        .await
        .unwrap()
    else {
        panic!("the write lands");
    };
    assert_eq!(version, Version::new(1), "a wild report moves no write");
    beat(&mut applied, SnapshotReport::ControlPlane(version)).await;
    let fleet = f.store.fleet(deadline()).await.unwrap();
    assert_eq!(fleet.head(), version);
    assert_eq!(
        fleet.rollout(version),
        Rollout::Pending {
            reached: 1,
            behind: vec![wild.process()]
        }
    );
    f.close().await;
}

/// A key with no row takes its shape from a running declarer, not from the
/// most recently referenced catalog: after a rollback that catalog can be a
/// stopped build's, and its shape would be refused by the running one.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_new_row_takes_the_running_declarers_shape_over_a_stopped_builds() {
    let f = Fixture::migrated().await;
    let mut running = process(&f.store, Build::Base, "running", 60, 600);
    let mut rolled_back = process(&f.store, Build::Extended, "rolled-back", 60, 600);
    beat(&mut running, SnapshotReport::NotLoaded).await;
    beat(&mut rolled_back, SnapshotReport::NotLoaded).await;
    rolled_back.stop(deadline()).await.unwrap();
    // The stopped build declared `fast_path` as a mode enum, and its catalog
    // is the most recently referenced one.
    for statement in [
        "UPDATE flags.flag_catalog_entries SET kind = 'mode', value_type = 'enum' \
         WHERE key = 'fast_path' AND manifest_digest = \
         (SELECT manifest_digest FROM flags.flag_processes WHERE build = 'rolled-back')",
        "UPDATE flags.flag_catalogs SET last_referenced_at = \
         CASE WHEN manifest_digest = (SELECT manifest_digest FROM flags.flag_processes \
         WHERE build = 'rolled-back') THEN clock_timestamp() ELSE clock_timestamp() - interval '1 hour' END",
    ] {
        sqlx::query(statement).execute(&f.pool).await.unwrap();
    }
    let written = f
        .store
        .write(set("fast_path", "on", None), deadline())
        .await
        .unwrap();
    assert!(
        matches!(
            written,
            WriteOutcome::Written {
                live_declaring: 1,
                ..
            }
        ),
        "{written:?}"
    );
    let (kind, value_type): (String, String) =
        sqlx::query_as("SELECT kind, value_type FROM flags.flag_state WHERE name = 'fast_path'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), value_type.as_str()), ("release", "toggle"));
    f.close().await;
}
