//! `ops/engage-halt.sh` against a scripted stand-in for `batter-flags`.
//!
//! Each case lists the reports the stand-in returns, in order, and asserts
//! the procedure's exit status and every command it ran. The stand-in fails
//! any call beyond its script, so an unexpected extra write fails the case.
//! The child gets its own environment; this process's is never changed.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

const STUB: &str = r#"#!/usr/bin/env bash
dir="$(cd "$(dirname "$0")" && pwd)"
printf '%s\n' "$*" >> "$dir/calls.log"
n=$(( $(cat "$dir/counter" 2>/dev/null || echo 0) + 1 ))
printf '%s' "$n" > "$dir/counter"
f="$dir/responses/$n"
[ -f "$f" ] || { echo "stub: unscripted call $n: $*" >&2; exit 99; }
code="$(head -n 1 "$f")"
tail -n +2 "$f"
exit "$code"
"#;

static CASE: AtomicUsize = AtomicUsize::new(0);

struct Outcome {
    status: i32,
    calls: Vec<String>,
    stderr: String,
}

impl Outcome {
    fn kills(&self) -> Vec<&String> {
        self.calls
            .iter()
            .filter(|call| call.contains(" kill "))
            .collect()
    }
}

/// Runs the procedure with `responses` as the stand-in's (exit code, stdout).
fn run(responses: &[(i32, &str)]) -> Outcome {
    let n = CASE.fetch_add(1, Ordering::SeqCst);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("engage-halt-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("responses")).unwrap();
    let stub = dir.join("batter-flags");
    fs::write(&stub, STUB).unwrap();
    make_executable(&stub);
    for (index, (code, body)) in responses.iter().enumerate() {
        fs::write(
            dir.join("responses").join((index + 1).to_string()),
            format!("{code}\n{body}"),
        )
        .unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("ops/engage-halt.sh");
    let output = Command::new("bash")
        .arg(script)
        .args([
            "--actor",
            "ops",
            "--reason",
            "contain duplicate charges",
            "--wait",
            "0",
        ])
        .env_clear()
        .env(
            "PATH",
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin",
        )
        .env("BATTER_FLAGS_BIN", &stub)
        .env(
            "BATTER_FLAGS_DATABASE_URL",
            "postgres://operator@127.0.0.1/checkout",
        )
        .env("ENGAGE_HALT_POLL_SECONDS", "0")
        .output()
        .unwrap();
    let calls = fs::read_to_string(dir.join("calls.log")).unwrap_or_default();
    Outcome {
        status: output.status.code().unwrap_or(-1),
        calls: calls.lines().map(str::to_owned).collect(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn show(revision: u64, state: Option<&str>) -> String {
    let state = state.map_or("null".to_owned(), |s| {
        format!(r#"{{"encoding":1,"switch":"{s}"}}"#)
    });
    format!(
        r#"{{"command":"show","exit":0,"outcome":"ok","message":"revision {revision}","data":{{"key":"checkout_halt","kind":"switch","value_type":"switch","revision":{revision},"stored":"active","state":{state},"declaration":{{"declared":true,"coordinated":false,"expiry":null,"expired":false,"orphaned":false}}}}}}"#
    )
}

const WRITTEN_8: &str = r#"{"command":"kill","exit":0,"outcome":"written","message":"written at version 8","data":{"version":8,"live_declaring":2,"deferral":{"unmet":[],"overridden_by":[],"unjudged_catalogs":0}}}"#;
const UNCERTAIN_8: &str = r#"{"command":"kill","exit":4,"outcome":"uncertain","message":"the commit's outcome is unknown; run `batter-flags history checkout_halt` and re-run with --if-revision","data":{"proposed_version":8}}"#;
const UNCHANGED_8: &str = r#"{"command":"kill","exit":0,"outcome":"unchanged","message":"already in that state at revision 8","data":{"revision":8}}"#;
const MISMATCH: &str = r#"{"command":"kill","exit":3,"outcome":"revision_mismatch","message":"nothing written: the key is at revision 9, the store head is 9","data":{"current":9,"head":9}}"#;
const REACHED_2: &str = r#"{"command":"fleet","exit":0,"outcome":"ok","message":"version 8 reached all 2 Live processes","data":{"head":8,"rollout":{"reached":true,"live":2},"live_groups":[],"not_live":[]}}"#;
const PENDING: &str = r#"{"command":"fleet","exit":0,"outcome":"ok","message":"version 8 reached 1 Live processes; 1 have not applied it","data":{"head":8,"rollout":{"reached":false,"applied":1,"behind":["0190-behind"]},"live_groups":[],"not_live":[]}}"#;

fn history(entries: &str) -> String {
    format!(
        r#"{{"command":"history","exit":0,"outcome":"ok","message":"entries","data":[{entries}]}}"#
    )
}

const OUR_WRITE_AT_8: &str = r#"{"version":8,"operation":"set","actor":"ops","reason":"contain duplicate charges","guard":"cas","recorded_by":"operator","after":{"name":"checkout_halt","state":{"encoding":1,"switch":"engaged"}}}"#;

#[test]
fn a_guarded_write_is_confirmed_across_the_fleet() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (0, WRITTEN_8),
        (0, &show(8, Some("engaged"))),
        (0, REACHED_2),
    ]);
    assert_eq!(outcome.status, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.calls,
        [
            "--timeout 10 --output json show checkout_halt",
            "--timeout 10 --output json kill checkout_halt engage --actor ops --reason contain duplicate charges --if-revision 7",
            "--timeout 10 --output json show checkout_halt",
            "--timeout 10 --output json fleet --version 8",
        ]
    );
}

#[test]
fn an_already_engaged_switch_is_not_written_again() {
    let outcome = run(&[
        (0, &show(8, Some("engaged"))),
        (0, &show(8, Some("engaged"))),
        (0, REACHED_2),
    ]);
    assert_eq!(outcome.status, 0, "{}", outcome.stderr);
    assert!(outcome.kills().is_empty());
}

#[test]
fn an_unknown_commit_that_history_shows_committed_is_not_reissued() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (4, UNCERTAIN_8),
        (0, &history(OUR_WRITE_AT_8)),
        (0, &show(8, Some("engaged"))),
        (0, REACHED_2),
    ]);
    assert_eq!(outcome.status, 0, "{}", outcome.stderr);
    assert_eq!(outcome.kills().len(), 1);
}

#[test]
fn an_unknown_commit_absent_from_history_is_reissued_with_the_same_guard() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (4, UNCERTAIN_8),
        (0, &history("")),
        // The first attempt had in fact committed: the store answers `unchanged`.
        (0, UNCHANGED_8),
        (0, &show(8, Some("engaged"))),
        (0, REACHED_2),
    ]);
    assert_eq!(outcome.status, 0, "{}", outcome.stderr);
    let kills = outcome.kills();
    assert_eq!(kills.len(), 2);
    assert!(
        kills
            .iter()
            .all(|call| call.ends_with("--if-revision 7") && !call.contains("--yes"))
    );
}

#[test]
fn a_panic_without_a_report_is_reconciled_like_an_unknown_commit() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (101, ""),
        (0, WRITTEN_8),
        (0, &show(8, Some("engaged"))),
        (0, REACHED_2),
    ]);
    assert_eq!(outcome.status, 0, "{}", outcome.stderr);
    assert_eq!(outcome.kills().len(), 2);
}

#[test]
fn a_write_that_stays_unknown_stops_without_an_unguarded_retry() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (4, UNCERTAIN_8),
        (
            1,
            r#"{"command":"history","exit":1,"outcome":"failed","message":"the timeout passed before COMMIT; nothing written"}"#,
        ),
        (4, UNCERTAIN_8),
        (0, &history("")),
        (4, UNCERTAIN_8),
        (0, &history("")),
        (4, UNCERTAIN_8),
        (0, &history("")),
    ]);
    assert_eq!(outcome.status, 30, "{}", outcome.stderr);
    let kills = outcome.kills();
    assert_eq!(kills.len(), 4);
    assert!(kills.iter().all(|call| call.ends_with("--if-revision 7")));
}

#[test]
fn a_concurrent_release_is_not_overwritten() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (3, MISMATCH),
        (0, &show(9, Some("released"))),
    ]);
    assert_eq!(outcome.status, 20, "{}", outcome.stderr);
    assert_eq!(outcome.kills().len(), 1);
}

#[test]
fn a_refusal_points_at_the_settings_floor() {
    let refused = r#"{"command":"kill","exit":3,"outcome":"refused","message":"nothing written: a Live process applied a version above the store head; see the restore runbook","data":{"refusal":"store_behind_fleet"}}"#;
    let outcome = run(&[(0, &show(7, None)), (3, refused)]);
    assert_eq!(outcome.status, 20, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("FLAG_CHECKOUT_HALT=engaged"),
        "{}",
        outcome.stderr
    );
    assert!(outcome.kills()[0].ends_with("--if-revision 7"));
}

#[test]
fn a_certain_failure_writes_nothing_and_says_so() {
    let failed = r#"{"command":"kill","exit":1,"outcome":"failed","message":"the database refused the operation (SQLSTATE 54000); nothing written","data":{"sqlstate":"54000"}}"#;
    let outcome = run(&[(0, &show(7, Some("released"))), (1, failed)]);
    assert_eq!(outcome.status, 20, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("NOTIFY queue is full"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_pending_rollout_is_reported_as_unconfirmed() {
    let outcome = run(&[
        (0, &show(7, Some("released"))),
        (0, WRITTEN_8),
        (0, &show(8, Some("engaged"))),
        (0, PENDING),
    ]);
    assert_eq!(outcome.status, 10, "{}", outcome.stderr);
    assert!(outcome.stderr.contains("0190-behind"), "{}", outcome.stderr);
}

#[test]
fn a_usage_error_from_the_command_stops_before_any_write() {
    let outcome = run(&[(2, "")]);
    assert_eq!(outcome.status, 2, "{}", outcome.stderr);
    assert!(outcome.kills().is_empty());
}
