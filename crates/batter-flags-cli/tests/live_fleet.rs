//! The operator command's fleet reports against a disposable PostgreSQL 18
//! database: what a rollout groups apart, and what an overflowing settings
//! floor sample can and cannot say.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![allow(clippy::expect_used, reason = "a panic fails the test")]

#[path = "live/support.rs"]
mod support;

use support::*;

/// A version restored from the last-good file is not one the control plane
/// applied: a process serving it shares build, revision and version with one
/// that applied it, but stands in its own group, and the rollout lists it as
/// behind.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_restored_file_version_is_grouped_apart_from_an_applied_one() {
    let db = Database::seeded().await;
    let written = db.cli_args(&[
        "set",
        "fast_path",
        "on",
        "--actor",
        "ops",
        "--reason",
        "go",
        "--yes",
    ]);
    assert_eq!(written.code, 0, "{written:?}");
    let fleet = db.cli_args(&["fleet", "--version", "1"]).json();
    assert_eq!(
        fleet["data"]["live_groups"][0]["snapshot_source"],
        "control_plane"
    );
    // A process that restored version 1 from its last-good file has not
    // applied it from the control plane: same build, revision and version,
    // but its own group, and the rollout lists it as behind.
    sqlx::query(
        "UPDATE flags.flag_processes SET snapshot_source = 'persisted_file', snapshot_version = 1 WHERE process_id = (SELECT process_id FROM flags.flag_processes WHERE stopped_at IS NULL ORDER BY process_id LIMIT 1)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE flags.flag_processes SET snapshot_version = 1 WHERE stopped_at IS NULL AND snapshot_source = 'control_plane'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let split = db.cli_args(&["fleet", "--version", "1"]).json();
    let mut sources: Vec<(String, u64)> = split["data"]["live_groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| {
            assert_eq!(group["snapshot_version"], 1, "{split}");
            (
                group["snapshot_source"].as_str().unwrap().to_owned(),
                group["processes"].as_u64().unwrap(),
            )
        })
        .collect();
    sources.sort();
    assert_eq!(
        sources,
        [
            ("control_plane".to_owned(), 1),
            ("persisted_file".to_owned(), 1)
        ],
        "{split}"
    );
    assert_eq!(split["data"]["rollout"]["reached"], false, "{split}");
    assert_eq!(
        split["data"]["rollout"]["behind"].as_array().unwrap().len(),
        1,
        "{split}"
    );
    assert_eq!(
        fleet["data"]["not_live"].as_array().unwrap().len(),
        1,
        "the stopped process stands apart"
    );
    assert_eq!(fleet["data"]["rollout"]["reached"], false);
    db.close().await;
}

/// An overflowing floor sample omits keys without naming them: a declarer
/// whose sample overflowed without listing the switch may keep it engaged,
/// and is reported apart from one whose sample lists it; a process whose
/// catalog does not declare the switch is not counted at all.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn an_overflowing_floor_sample_is_reported_as_unknown_not_engaged() {
    let db = Database::seeded().await;
    db.live(&["fast_path", "halt"], "build-overflow").await;
    db.live(&["fast_path"], "build-undeclaring").await;
    sqlx::query(
        "UPDATE flags.flag_processes SET floor_overflow = 2 WHERE build IN ('build-overflow', 'build-undeclaring')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let engaged = db.cli_args(&[
        "kill", "halt", "engage", "--actor", "ops", "--reason", "contain", "--yes",
    ]);
    assert_eq!(engaged.code, 0, "{engaged:?}");
    let released = db.cli_args(&[
        "kill",
        "halt",
        "release",
        "--actor",
        "ops",
        "--reason",
        "recovered",
        "--if-revision",
        "1",
    ]);
    assert_eq!(released.code, 0, "{released:?}");
    let warnings: Vec<String> = released.json()["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].starts_with("1 Live processes keep halt engaged"),
        "{warnings:?}"
    );
    assert!(
        warnings[1]
            .starts_with("1 Live processes declaring halt report more floor-engaged switches")
            && warnings[1].ends_with("is unknown"),
        "{warnings:?}"
    );
    db.close().await;
}

/// A release the store already holds is unchanged, yet a Live process's
/// settings floor can still keep the switch engaged: the operator sees the
/// same warning that explains why it is still tripped.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn releasing_an_already_released_switch_still_warns_about_floors() {
    let db = Database::seeded().await;
    let release = || {
        db.cli_args(&[
            "kill",
            "halt",
            "release",
            "--actor",
            "ops",
            "--reason",
            "recovered",
            "--yes",
        ])
    };
    let first = release();
    assert_eq!(first.code, 0, "{first:?}");
    assert_eq!(first.json()["outcome"], "written");
    let again = release();
    assert_eq!(again.code, 0, "{again:?}");
    let again = again.json();
    assert_eq!(again["outcome"], "unchanged", "{again}");
    let warnings = again["warnings"].as_array().expect("a floor warning");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .starts_with("1 Live processes keep halt engaged"),
        "{warnings:?}"
    );
    db.close().await;
}
