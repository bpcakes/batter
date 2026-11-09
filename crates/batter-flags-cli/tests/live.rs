//! The operator command against a disposable PostgreSQL 18 database, spawned
//! as a binary over catalogs that real heartbeats published.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![allow(clippy::expect_used, reason = "a panic fails the test")]

#[path = "live/support.rs"]
mod support;

use serde_json::Value;
use support::*;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn refused_writes_exit_3_and_write_nothing() {
    let db = Database::seeded().await;
    let refused = |args: &[&str]| {
        let run = db.cli(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
        assert_eq!(run.code, 3, "{run:?}");
        run.json()["data"]["refusal"].as_str().unwrap().to_owned()
    };
    assert_eq!(
        refused(&[
            "set",
            "never_declared",
            "on",
            "--actor",
            "ops",
            "--reason",
            "test",
            "--yes"
        ]),
        "undeclared"
    );
    assert_eq!(
        refused(&[
            "set", "halt", "on", "--actor", "ops", "--reason", "test", "--yes"
        ]),
        "wrong_kind"
    );
    assert_eq!(
        refused(&[
            "kill",
            "fast_path",
            "engage",
            "--actor",
            "ops",
            "--reason",
            "test",
            "--yes"
        ]),
        "wrong_kind"
    );
    assert_eq!(
        refused(&[
            "set", "rollout", "on", "--actor", "ops", "--reason", "test", "--yes"
        ]),
        "coordinated"
    );
    assert_eq!(db.head().await, 0);
    assert_eq!(db.audits().await, 0);

    let accepted = db.cli_args(&[
        "set",
        "rollout",
        "on",
        "--actor",
        "ops",
        "--reason",
        "test",
        "--yes",
        "--accept-rolling-flip",
    ]);
    assert_eq!(accepted.code, 0, "{accepted:?}");
    let accepted = accepted.json();
    assert_eq!(accepted["outcome"], "written");
    // An accepted coordinated flip is named in the outcome and warned about.
    assert_eq!(accepted["data"]["deferral"]["flip"], "coordinated");
    let warnings = accepted["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().unwrap().starts_with(
                "rollout is declared coordinated: processes apply this write poll by poll"
            )),
        "{warnings:?}"
    );
    let stale = db.cli_args(&[
        "set",
        "fast_path",
        "on",
        "--actor",
        "ops",
        "--reason",
        "test",
        "--if-revision",
        "5",
    ]);
    assert_eq!(stale.code, 3);
    assert_eq!(stale.json()["data"]["refusal"], "revision_above_head");
    let unchanged = db.cli_args(&[
        "set",
        "rollout",
        "on",
        "--actor",
        "ops",
        "--reason",
        "test",
        "--if-revision",
        "1",
        "--accept-rolling-flip",
    ]);
    assert_eq!(unchanged.code, 0);
    assert_eq!(unchanged.json()["outcome"], "unchanged");
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn releasing_a_switch_a_settings_floor_holds_warns_with_the_count() {
    let db = Database::seeded().await;
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
    let warnings = released.json()["warnings"].clone();
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .starts_with("1 Live processes keep halt engaged"),
        "{warnings}"
    );
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn the_read_commands_report_revisions_filters_history_and_the_fleet() {
    let db = Database::seeded().await;
    let show = db.cli_args(&["show", "fast_path"]);
    assert_eq!(show.code, 0);
    assert_eq!(
        show.json()["data"]["revision"],
        0,
        "an absent key is revision 0"
    );
    let rolling = db
        .cli_args(&[
            "set",
            "fast_path",
            "on",
            "--actor",
            "ops",
            "--reason",
            "go",
            "--if-revision",
            "0",
        ])
        .json();
    assert_eq!(rolling["data"]["deferral"]["flip"], "rolling");
    assert!(
        !rolling.to_string().contains("declared coordinated"),
        "{rolling}"
    );
    assert_eq!(
        db.cli_args(&["show", "fast_path"]).json()["data"]["revision"],
        1
    );

    let keys = |filter: &str| -> Vec<String> {
        db.cli_args(&["list", filter]).json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["key"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(keys("--expired"), ["old_path"]);
    assert_eq!(keys("--orphaned"), ["orphan_path"]);
    assert_eq!(
        keys("--declared-only"),
        ["halt", "old_path", "orphan_path", "rollout"]
    );
    let history = db.cli_args(&["history", "fast_path"]).json();
    assert_eq!(history["data"][0]["actor"], "ops");
    assert_eq!(history["data"][0]["version"], 1);

    let fleet = db.cli_args(&["fleet", "--version", "1"]).json();
    assert_eq!(fleet["data"]["live_groups"].as_array().unwrap().len(), 1);
    assert_eq!(fleet["data"]["live_groups"][0]["processes"], 2);
    assert_eq!(
        fleet["data"]["not_live"].as_array().unwrap().len(),
        1,
        "the stopped process stands apart"
    );
    assert_eq!(fleet["data"]["rollout"]["reached"], false);
    let declarers = db.cli_args(&["fleet", "--flag", "fast_path"]).json();
    let presence: Vec<&str> = declarers["data"]["declarers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["presence"].as_str().unwrap())
        .collect();
    assert_eq!(presence, ["live", "live", "stopped"]);
    db.close().await;
}

/// The store returns history a page at a time; the command reads every page,
/// so the newest event, the one an uncertain write points to, is shown.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn history_reads_every_page_so_the_newest_event_is_shown() {
    let db = Database::seeded().await;
    let count = batter_flags_postgres::HISTORY_PAGE + 1;
    db.audited_writes("fast_path", count).await;
    let history = db.cli_args(&["history", "fast_path"]);
    assert_eq!(history.code, 0, "{history:?}");
    let history = history.json();
    let entries = history["data"].as_array().unwrap();
    assert_eq!(entries.len(), count);
    assert_eq!(entries.last().unwrap()["version"], db.head().await);
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn json_output_is_one_document_for_every_exit_but_a_parser_usage_error() {
    let db = Database::seeded().await;
    for (args, code) in [
        (vec!["show", "fast_path"], 0),
        (
            vec![
                "set",
                "never_declared",
                "on",
                "--actor",
                "ops",
                "--reason",
                "x",
                "--yes",
            ],
            3,
        ),
    ] {
        let run = db.cli_args(&args);
        assert_eq!(run.code, code);
        assert_eq!(run.stdout.trim().lines().count(), 1, "{run:?}");
        let _: Value = serde_json::from_str(run.stdout.trim()).unwrap();
    }
    let missing = run_cli(&[], &["--output", "json", "show", "fast_path"]);
    assert_eq!(missing.code, 2);
    assert_eq!(missing.json()["outcome"], "usage");
    let refused = run_cli(
        &[(
            "BATTER_FLAGS_DATABASE_URL",
            "postgres://ops@127.0.0.1:1/flags?sslmode=disable",
        )],
        &["--output", "json", "--timeout", "2", "show", "fast_path"],
    );
    assert_eq!(refused.code, 1);
    assert_eq!(refused.json()["outcome"], "failed");
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn no_sentinel_reaches_either_stream() {
    const SENTINEL: &str = "SENTINEL-7f3a";
    let db = Database::seeded().await;
    let cases: Vec<(String, Vec<&str>)> = vec![
        // Connection refused.
        (
            format!("postgres://ops:{SENTINEL}@127.0.0.1:1/flags?sslmode=disable"),
            vec!["show", "fast_path"],
        ),
        // URL policy refusal.
        (
            format!("postgres://ops:{SENTINEL}@db.example.invalid/flags?sslmode=disable"),
            vec!["show", "fast_path"],
        ),
        // A guard refusal over a live connection, with the sentinel in the reason.
        (
            db.url_with_password(SENTINEL),
            vec![
                "set", "halt", "on", "--actor", "ops", "--reason", SENTINEL, "--yes",
            ],
        ),
    ];
    for (url, args) in cases {
        for output in ["text", "json"] {
            let mut full = vec!["--output", output, "--timeout", "3"];
            full.extend(args.iter().copied());
            let run = run_cli(&[("BATTER_FLAGS_DATABASE_URL", &url)], &full);
            assert!(
                !run.stdout.contains(SENTINEL) && !run.stderr.contains(SENTINEL),
                "{run:?}"
            );
            assert_ne!(run.code, 0);
        }
    }
    db.close().await;
}

#[test]
fn arguments_are_validated_before_any_connection() {
    let unreachable = [(
        "BATTER_FLAGS_DATABASE_URL",
        "postgres://ops@127.0.0.1:1/flags?sslmode=disable",
    )];
    for args in [
        vec![
            "--output",
            "json",
            "set",
            "fast_path",
            "on",
            "--actor",
            "ops",
            "--reason",
            "x",
        ],
        vec![
            "--output",
            "json",
            "set",
            "fast_path",
            "on",
            "--actor",
            "ops",
            "--reason",
            "x",
            "--yes",
            "--if-revision",
            "1",
        ],
        vec!["--output", "json", "show", "Not A Key"],
    ] {
        let run = run_cli(&unreachable, &args);
        assert_eq!(run.code, 2, "{run:?}");
        assert_eq!(run.json()["outcome"], "usage");
    }
}
