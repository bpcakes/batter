//! The NOTIFY wake against a disposable PostgreSQL 18 database: a committed
//! write reaches every listening process within the wake spacing, with the
//! poll as the backstop.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "listen_live/support.rs"]
mod support;

use std::time::Duration;

use batter_flags::{
    control_plane::RegisteredFlags,
    core::{KillSwitch, SwitchState},
    low_level::capture_now,
};
use support::*;
use tokio::time::Instant;

/// A wake follows a commit within the spacing; a load then takes at most
/// this long on a local server.
const LOAD: Duration = Duration::from_millis(1_500);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_committed_write_reaches_every_listening_process_within_the_spacing() {
    let database = Database::migrated().await;
    let (first, first_running, first_halt) = process(&database, Listening::Yes).await;
    let (second, second_running, second_halt) = process(&database, Listening::Yes).await;
    listening_count(&database, 2).await;
    // Let both resync wakes run, so only the write's notification is left.
    tokio::time::sleep(SECOND * 2).await;
    database.write(SwitchState::Engaged).await;
    let committed = Instant::now();
    let reached = |flags: &RegisteredFlags, halt: &KillSwitch| {
        capture_now(flags).switch(halt).value() == SwitchState::Engaged
    };
    while !(reached(&first, &first_halt) && reached(&second, &second_halt)) {
        assert!(
            committed.elapsed() < SECOND + LOAD,
            "both processes within the spacing and one load"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // A burst of notifications is coalesced: no refresh starts sooner than
    // the spacing after the previous one.
    let channel: String = sqlx::query_scalar("SELECT channel FROM flags.flag_control")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    let before = first.refresh().invocations;
    let burst = Instant::now();
    for _ in 0..20 {
        sqlx::query("SELECT pg_notify($1, '1')")
            .bind(&channel)
            .execute(&database.pool)
            .await
            .unwrap();
    }
    tokio::time::sleep(SECOND * 3).await;
    let runs = first.refresh().invocations - before;
    let allowed = burst.elapsed().as_secs() + 1;
    assert!(
        runs >= 1 && runs <= allowed,
        "{runs} refreshes in {:?}",
        burst.elapsed()
    );
    first_running.shutdown_checked().await.unwrap();
    second_running.shutdown_checked().await.unwrap();
    database.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_notification_inside_an_open_transaction_wakes_nothing_until_it_commits() {
    let database = Database::migrated().await;
    let (flags, running, _) = process(&database, Listening::Yes).await;
    listening(&database).await;
    tokio::time::sleep(SECOND * 2).await;
    let before = flags.refresh().invocations;
    let channel: String = sqlx::query_scalar("SELECT channel FROM flags.flag_control")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    let mut transaction = database.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_notify($1, '1')")
        .bind(&channel)
        .execute(&mut *transaction)
        .await
        .unwrap();
    tokio::time::sleep(SECOND * 2).await;
    assert_eq!(
        flags.refresh().invocations,
        before,
        "an uncommitted notification is not delivered"
    );
    transaction.commit().await.unwrap();
    tokio::time::sleep(SECOND * 2).await;
    assert!(
        flags.refresh().invocations > before,
        "the committed notification woke a refresh"
    );
    running.shutdown_checked().await.unwrap();
    database.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_terminated_listener_rebuilds_and_its_resync_catches_the_gap() {
    let database = Database::migrated().await;
    let (flags, running, halt) = process(&database, Listening::Yes).await;
    listening(&database).await;
    tokio::time::sleep(SECOND * 2).await;
    let lost = listener_pids(&database).await;
    for pid in &lost {
        sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .execute(&database.pool)
            .await
            .unwrap();
    }
    // Written while the session is gone; the loss itself or the rebuilt
    // session's resync delivers it well before the minute-long poll.
    database.write(SwitchState::Engaged).await;
    assert!(
        engaged_within(&flags, &halt, SECOND * 8).await,
        "the gap was caught"
    );
    // A new session, with another backend, listens again and delivers the
    // next write within the spacing.
    let mut rebuilt = Vec::new();
    for _ in 0..100 {
        rebuilt = listener_pids(&database).await;
        if !rebuilt.is_empty() && rebuilt.iter().all(|pid| !lost.contains(pid)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !rebuilt.is_empty() && rebuilt.iter().all(|pid| !lost.contains(pid)),
        "a replacement session listens"
    );
    tokio::time::sleep(SECOND * 2).await;
    database.write(SwitchState::Released).await;
    let written = Instant::now();
    while capture_now(&flags).switch(&halt).value() != SwitchState::Released {
        assert!(
            written.elapsed() < SECOND + LOAD,
            "the rebuilt session delivered the next write"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    running.shutdown_checked().await.unwrap();
    database.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires disposable PostgreSQL 18"]
async fn without_a_listener_the_poll_still_converges() {
    let database = Database::migrated().await;
    let (quiet, quiet_running, quiet_halt) = process(&database, Listening::No).await;
    let (polled, polled_running, polled_halt) = process(&database, Listening::PollFast).await;
    database.write(SwitchState::Engaged).await;
    assert!(
        engaged_within(&polled, &polled_halt, SECOND * 5).await,
        "the poll delivered it"
    );
    assert!(
        !engaged_within(&quiet, &quiet_halt, SECOND * 2).await,
        "nothing woke the minute-long poll"
    );
    quiet_running.shutdown_checked().await.unwrap();
    polled_running.shutdown_checked().await.unwrap();
    database.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires disposable PostgreSQL 18"]
async fn drain_unlistens_closes_the_listener_sessions_and_records_the_stop() {
    let database = Database::migrated().await;
    let (flags, running, _) = process(&database, Listening::Yes).await;
    listening(&database).await;
    let sessions: Vec<i32> = sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname = current_database() AND application_name = 'batter-flags-listener'")
        .fetch_all(&database.pool)
        .await
        .unwrap();
    assert!(!sessions.is_empty());
    let started = Instant::now();
    running.shutdown_checked().await.unwrap();
    assert!(
        started.elapsed() < SECOND * 5,
        "drain does not wait on a receive"
    );
    let alive: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid = ANY($1)")
            .bind(&sessions)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(alive, 0, "every listener backend ended");
    let stopped: bool = sqlx::query_scalar(
        "SELECT stopped_at IS NOT NULL FROM flags.flag_processes WHERE process_id = $1",
    )
    .bind(flags.process().as_uuid())
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert!(stopped, "the stop marker ran before the pools closed");
    database.close().await;
}
