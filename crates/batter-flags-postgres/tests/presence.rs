//! PostgreSQL 18 presence and prune contracts, over rows written by real
//! heartbeats.
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
use sqlx::Row;
use std::time::Duration;
use support::*;

async fn count(f: &Fixture, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&f.pool).await.unwrap()
}

/// Waits, for at most five seconds, until another session is blocked by the
/// transaction `holder` runs.
async fn blocked_by(f: &Fixture, holder: &mut sqlx::Transaction<'_, sqlx::Postgres>) {
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut **holder)
        .await
        .unwrap();
    for _ in 0..100 {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT FROM pg_stat_activity WHERE $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(pid)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        if blocked {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no session waited on the held lock");
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_first_heartbeat_publishes_the_catalog_and_later_ones_only_refresh() {
    let f = Fixture::migrated().await;
    let mut first = process(&f.store, Build::Base, "build-1", 60, 600);
    first
        .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();

    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_catalogs").await,
        1
    );
    let entries = sqlx::query("SELECT key, kind, value_type, variants, expiry::text AS expiry, flip_discipline FROM flags.flag_catalog_entries ORDER BY key")
        .fetch_all(&f.pool).await.unwrap();
    let fast = &entries[0];
    assert_eq!(fast.get::<String, _>("key"), "fast_path");
    assert_eq!(fast.get::<String, _>("kind"), "release");
    assert_eq!(fast.get::<String, _>("value_type"), "toggle");
    assert_eq!(fast.get::<Vec<String>, _>("variants"), ["off", "on"]);
    assert_eq!(
        fast.get::<Option<String>, _>("expiry").as_deref(),
        Some("2027-03-31")
    );
    let halt = &entries[1];
    assert_eq!(halt.get::<String, _>("kind"), "switch");
    assert!(halt.get::<Vec<String>, _>("variants").is_empty());

    let row = sqlx::query("SELECT snapshot_source, snapshot_version, liveness_seconds, retention_seconds, row_encoding, expires_at > clock_timestamp() AS live FROM flags.flag_processes")
        .fetch_one(&f.pool).await.unwrap();
    assert_eq!(row.get::<String, _>("snapshot_source"), "not_loaded");
    assert_eq!(row.get::<Option<i64>, _>("snapshot_version"), None);
    assert_eq!(row.get::<i32, _>("liveness_seconds"), 60);
    assert_eq!(row.get::<i32, _>("retention_seconds"), 600);
    assert_eq!(row.get::<i16, _>("row_encoding"), 1);
    assert!(row.get::<bool, _>("live"));

    // A later heartbeat of the same build, and another process of it, send
    // no catalog: the stored one is found and only referenced.
    let seen: String = sqlx::query_scalar("SELECT first_seen::text FROM flags.flag_catalogs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let mut beat = report(SnapshotReport::ControlPlane(Version::new(3)));
    beat.held = KeySample::of([key("fast_path")]);
    first.heartbeat(&beat, deadline()).await.unwrap();
    let mut second = process(&f.store, Build::Base, "build-1", 60, 600);
    second
        .heartbeat(
            &report(SnapshotReport::PersistedFile(Version::new(2))),
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_catalogs").await,
        1
    );
    let again: String = sqlx::query_scalar("SELECT first_seen::text FROM flags.flag_catalogs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(seen, again);
    let fleet = f.store.fleet(deadline()).await.unwrap();
    let mine = fleet
        .processes()
        .iter()
        .find(|p| p.process() == first.process())
        .unwrap();
    assert_eq!(
        mine.snapshot(),
        SnapshotReport::ControlPlane(Version::new(3))
    );
    assert_eq!(mine.held().keys(), [key("fast_path")]);
    assert_eq!(fleet.processes().len(), 2);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_stop_consumes_the_presence_keeps_the_expiry_and_survives_heartbeats() {
    let f = Fixture::migrated().await;
    let never = process(&f.store, Build::Base, "never", 60, 600);
    assert_eq!(
        never.stop(deadline()).await.unwrap(),
        StopOutcome::NotRegistered
    );

    let mut stopping = process(&f.store, Build::Base, "stopping", 60, 600);
    stopping
        .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    let expiry = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT expires_at::text FROM flags.flag_processes WHERE build = 'stopping'",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap()
    };
    let before = expiry().await;
    assert_eq!(
        stopping.stop(deadline()).await.unwrap(),
        StopOutcome::Stopped
    );
    assert_eq!(expiry().await, before, "a stop leaves the expiry alone");

    // A stop recorded some other way is reported, and a heartbeat never
    // clears it.
    let mut marked = process(&f.store, Build::Base, "marked", 60, 600);
    marked
        .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE flags.flag_processes SET stopped_at = clock_timestamp() WHERE build = 'marked'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    marked
        .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    assert_eq!(
        marked.stop(deadline()).await.unwrap(),
        StopOutcome::AlreadyStopped
    );
    let states: Vec<_> = f
        .store
        .fleet(deadline())
        .await
        .unwrap()
        .processes()
        .iter()
        .map(FleetProcess::state)
        .collect();
    assert_eq!(states, [ProcessState::Stopped, ProcessState::Stopped]);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_heartbeat_stamps_its_expiry_after_waiting_for_its_row() {
    let f = Fixture::migrated().await;
    let mut slow = process(&f.store, Build::Base, "slow", 1, 600);
    slow.heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    // Hold the row past its one-second liveness while the next heartbeat waits.
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT process_id FROM flags.flag_processes FOR UPDATE")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let beat = tokio::spawn(async move {
        slow.heartbeat(&report(SnapshotReport::NotLoaded), deadline())
            .await
    });
    blocked_by(&f, &mut holder).await;
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    holder.commit().await.unwrap();
    beat.await.unwrap().unwrap();
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM flags.flag_processes WHERE expires_at > clock_timestamp()"
        )
        .await,
        1,
        "the row is Live after the heartbeat that waited for it"
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_heartbeat_outlasts_a_prune_that_deletes_its_catalog() {
    let f = Fixture::migrated().await;
    let mut gone = process(&f.store, Build::Base, "build-1", 60, 600);
    gone.heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    sqlx::query("DELETE FROM flags.flag_processes")
        .execute(&f.pool)
        .await
        .unwrap();

    // A prune holds the catalog row for update and deletes it.
    let mut prune = f.pool.begin().await.unwrap();
    sqlx::query("SELECT manifest_digest FROM flags.flag_catalogs FOR UPDATE")
        .fetch_one(&mut *prune)
        .await
        .unwrap();
    sqlx::query("DELETE FROM flags.flag_catalogs")
        .execute(&mut *prune)
        .await
        .unwrap();

    // A heartbeat of the same build waits on that lock...
    let mut next = process(&f.store, Build::Base, "build-1", 60, 600);
    let beat = tokio::spawn(async move {
        next.heartbeat(&report(SnapshotReport::NotLoaded), deadline())
            .await
    });
    blocked_by(&f, &mut prune).await;
    assert!(!beat.is_finished(), "the heartbeat waits for the prune");

    // ...and once the delete commits it publishes the catalog again.
    prune.commit().await.unwrap();
    beat.await.unwrap().unwrap();
    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_catalogs").await,
        1
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_catalog_entries").await,
        2
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_processes").await,
        1
    );
    f.close().await;
}

#[test]
fn presence_inputs_are_validated_at_construction() {
    assert!(ProcessLabel::new("build-1.2+rc_3").is_ok());
    for bad in ["", "white space", "slash/", &"x".repeat(97)] {
        assert!(ProcessLabel::new(bad).is_err(), "{bad:?}");
    }
    let s = Duration::from_secs;
    assert!(Cadence::new(s(1), s(86_400)).is_ok());
    assert!(Cadence::new(s(0), s(10)).is_err());
    assert!(Cadence::new(s(10), s(86_401)).is_err());
    assert!(Cadence::new(Duration::from_millis(1_500), s(10)).is_err());
    assert!(PruneLimit::new(0).is_err());
    assert!(PruneLimit::new(10_001).is_err());
    let sample = KeySample::of(
        (0..70)
            .map(|n| key(&format!("k{n:02}")))
            .chain([key("k00")]),
    );
    assert_eq!(sample.keys().len(), 64);
    assert_eq!(sample.overflow(), 6);
    assert_eq!(sample.keys()[0], key("k00"));
}
