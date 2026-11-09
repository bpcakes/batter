//! PostgreSQL 18 prune contracts, over rows written by real heartbeats.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent contract assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
#[path = "support/presence.rs"]
mod support;
use batter_flags_postgres::*;
use fixture::*;
use std::time::Duration;
use support::*;

async fn count(f: &Fixture, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&f.pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn each_row_is_pruned_by_its_own_expiry_and_retention() {
    let f = Fixture::migrated().await;
    let mut brief = process(&f.store, Build::Base, "brief", 1, 1);
    let mut kept = process(&f.store, Build::Base, "kept", 1, 3600);
    let mut live = process(&f.store, Build::Base, "live", 3600, 1);
    for process in [&mut brief, &mut kept, &mut live] {
        process
            .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let states: Vec<_> = f
        .store
        .fleet(deadline())
        .await
        .unwrap()
        .processes()
        .iter()
        .map(|p| (p.build().to_owned(), p.state()))
        .collect();
    assert_eq!(
        states,
        [
            ("brief".into(), ProcessState::Silent),
            ("kept".into(), ProcessState::Silent),
            ("live".into(), ProcessState::Live),
        ]
    );
    let limit = PruneLimit::new(100).unwrap();
    let pruned = f.store.prune(limit, deadline()).await.unwrap();
    assert_eq!(
        pruned,
        Pruned {
            processes: 1,
            catalogs: 0
        }
    );
    let left: Vec<String> =
        sqlx::query_scalar("SELECT build FROM flags.flag_processes ORDER BY build")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(left, ["kept", "live"]);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn prune_deletes_only_catalogs_unreferenced_for_thirty_days() {
    let f = Fixture::migrated().await;
    let mut old = process(&f.store, Build::Base, "old", 60, 600);
    let mut current = process(&f.store, Build::Extended, "current", 60, 600);
    for process in [&mut old, &mut current] {
        process
            .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM flags.flag_processes WHERE build = 'old'")
        .execute(&f.pool)
        .await
        .unwrap();
    let limit = PruneLimit::new(100).unwrap();
    sqlx::query("UPDATE flags.flag_catalogs SET last_referenced_at = clock_timestamp() - interval '29 days'")
        .execute(&f.pool).await.unwrap();
    assert_eq!(
        f.store.prune(limit, deadline()).await.unwrap(),
        Pruned::default()
    );

    sqlx::query("UPDATE flags.flag_catalogs SET last_referenced_at = clock_timestamp() - interval '31 days'")
        .execute(&f.pool).await.unwrap();
    let pruned = f.store.prune(limit, deadline()).await.unwrap();
    assert_eq!(
        pruned,
        Pruned {
            processes: 0,
            catalogs: 1
        },
        "a referenced catalog stays"
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_catalogs").await,
        1
    );
    // The pruned catalog's entries went with it; the kept one has three.
    assert_eq!(
        count(&f, "SELECT count(*) FROM flags.flag_catalog_entries").await,
        3
    );

    // A heartbeat refreshes a reference older than a day.
    current
        .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
        .await
        .unwrap();
    assert_eq!(count(&f, "SELECT count(*) FROM flags.flag_catalogs WHERE last_referenced_at > clock_timestamp() - interval '1 minute'").await, 1);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn prune_deletes_at_most_its_limit_and_passes_locked_rows_by() {
    let f = Fixture::migrated().await;
    for n in 0..3 {
        let mut brief = process(&f.store, Build::Base, &format!("brief-{n}"), 1, 1);
        brief
            .heartbeat(&report(SnapshotReport::NotLoaded), deadline())
            .await
            .unwrap();
    }
    for n in 0..3_u8 {
        sqlx::query("INSERT INTO flags.flag_catalogs(manifest_digest, catalog_digest, manifest, last_referenced_at) VALUES ($1, $1, '{}', clock_timestamp() - interval '40 days')")
            .bind(vec![n; 32]).execute(&f.pool).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let two = PruneLimit::new(2).unwrap();
    // The heartbeats' own catalog is still referenced, so only the three
    // unreferenced ones are due, and a limit of two leaves one of each.
    assert_eq!(
        f.store.prune(two, deadline()).await.unwrap(),
        Pruned {
            processes: 2,
            catalogs: 2
        }
    );

    // The last due rows, held by another transaction, are passed by at once.
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT process_id FROM flags.flag_processes FOR UPDATE")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    sqlx::query(
        "SELECT manifest_digest FROM flags.flag_catalogs WHERE manifest = '{}' FOR KEY SHARE",
    )
    .fetch_one(&mut *holder)
    .await
    .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        f.store.prune(two, deadline()).await.unwrap(),
        Pruned::default()
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a prune never waits on a held row"
    );
    holder.commit().await.unwrap();
    // Released, the held rows are due as before.
    assert_eq!(
        f.store.prune(two, deadline()).await.unwrap(),
        Pruned {
            processes: 1,
            catalogs: 1
        }
    );
    f.close().await;
}
