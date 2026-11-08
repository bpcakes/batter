//! The operator command's commit, signal and archive contracts against a
//! disposable PostgreSQL 18 database.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![allow(clippy::expect_used, reason = "a panic fails the test")]

#[path = "live/support.rs"]
mod support;

use support::*;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn an_interrupt_before_commit_rolls_back_and_one_during_commit_is_deferred() {
    let db = Database::seeded().await;
    // Hold the counter so the write waits before COMMIT, then interrupt it.
    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT version FROM flags.flag_control FOR UPDATE")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let child = db.spawn(&[
        "--output",
        "json",
        "--timeout",
        "20",
        "kill",
        "halt",
        "engage",
        "--actor",
        "ops",
        "--reason",
        "x",
        "--yes",
    ]);
    db.wait_for_lock_wait().await;
    signal(&child, "-INT");
    let run = finish(child);
    holder.rollback().await.unwrap();
    assert_eq!(run.code, 1, "{run:?}");
    assert_eq!(run.json()["outcome"], "interrupted", "{run:?}");
    assert_eq!(db.audits().await, 0);

    // A COMMIT that a deferred trigger holds: the signal waits for it.
    db.slow_commit(2).await;
    let child = db.spawn(&[
        "--output",
        "json",
        "--timeout",
        "20",
        "kill",
        "halt",
        "engage",
        "--actor",
        "ops",
        "--reason",
        "x",
        "--yes",
    ]);
    db.wait_for_sleep().await;
    signal(&child, "-INT");
    let run = finish(child);
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(run.json()["outcome"], "written");
    assert_eq!(db.audits().await, 1);
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_commit_the_database_rejects_exits_1_with_nothing_written() {
    let db = Database::seeded().await;
    db.failing_commit().await;
    let run = db.cli_args(&[
        "kill", "halt", "engage", "--actor", "ops", "--reason", "x", "--yes",
    ]);
    assert_eq!(run.code, 1, "{run:?}");
    assert_eq!(run.json()["data"]["sqlstate"], "P0001");
    assert!(
        run.json()["message"]
            .as_str()
            .unwrap()
            .contains("nothing written")
    );
    assert_eq!(db.audits().await, 0);
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_lost_commit_acknowledgement_exits_4_with_re_run_guidance() {
    let db = Database::seeded().await;
    db.lost_commit().await;
    let run = db.cli_args(&[
        "kill", "halt", "engage", "--actor", "ops", "--reason", "x", "--yes",
    ]);
    assert_eq!(run.code, 4, "{run:?}");
    assert_eq!(run.stdout.trim().lines().count(), 1);
    let report = run.json();
    assert_eq!(report["outcome"], "uncertain");
    assert!(
        report["message"]
            .as_str()
            .unwrap()
            .contains("--if-revision")
    );
    // The session ended inside COMMIT, so here the transaction rolled back;
    // the command could not know that, and `history` is how to find out.
    assert_eq!(db.audits().await, 0);
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn archive_and_restore_follow_the_declaration_gate_and_silent_blocks_name_prune() {
    let db = Database::seeded().await;
    db.cli_args(&[
        "set",
        "fast_path",
        "on",
        "--actor",
        "ops",
        "--reason",
        "x",
        "--yes",
    ]);
    let archived = db.cli_args(&[
        "archive",
        "fast_path",
        "--actor",
        "ops",
        "--reason",
        "x",
        "--yes",
    ]);
    assert_eq!(archived.code, 3, "{archived:?}");
    assert_eq!(archived.json()["data"]["refusal"], "still_declared");
    let restored = db.cli_args(&[
        "restore",
        "fast_path",
        "--actor",
        "ops",
        "--reason",
        "x",
        "--yes",
    ]);
    assert_eq!(restored.code, 0);
    assert_eq!(restored.json()["outcome"], "unchanged");

    // A build that only a Silent row keeps declared blocks an undecodable
    // write until prune deletes it.
    sqlx::query("UPDATE flags.flag_processes SET stopped_at = clock_timestamp()")
        .execute(&db.pool)
        .await
        .unwrap();
    db.silent("build-silent").await;
    let blocked = db.cli_args(&[
        "set",
        "fast_path",
        "express",
        "--actor",
        "ops",
        "--reason",
        "x",
        "--yes",
    ]);
    assert_eq!(blocked.code, 3, "{blocked:?}");
    let report = blocked.json();
    assert_eq!(report["data"]["refusal"], "undecodable_by_live_declaration");
    assert_eq!(report["data"]["builds"][0]["presence"], "silent");
    assert!(
        report["message"]
            .as_str()
            .unwrap()
            .contains("fleet --flag fast_path")
    );
    assert!(report["message"].as_str().unwrap().contains("prune"));
    db.close().await;
}
