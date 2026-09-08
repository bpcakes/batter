# Separate subprocess startup and observation budgets

Owning Bead: `batter-fvz`. Baseline: `e5f2f04b2dbb349d08085caf177f662fcbc89812`
plus the existing staged and unstaged changes. The review found that startup
could consume the blocked-runtime fixture's two-second observation window.
This task changes only private tests and their documentation/tracker evidence.

## Progress

- Reproduced the missing observation event by delaying launch authority four seconds.
- Implemented five-second startup and three-second observation phases; retained
  the ten-second independent emergency exit and existing semantic assertions.
- Added delayed-startup, absent-startup and extended-deadline early-kill controls.
- Both Rust verification matrices and all three rebuilt HTTP smoke modes pass.
- Contracts, implemented status, test counts and validation evidence are updated.
- Jig required-gate receipts and final completion are recorded through this
  plan's work check/evidence/gates/finish commands.

## Surprises & Discoveries

The original watchdog killed the delayed fixture at 5.005940244 seconds before
`blocked-observation-elapsed`. A process exit record must retain its selected
deadline so the old five-second threshold cannot approve an early extended kill.

## Decision Log

Share the two-second observation duration from `non_yielding/fixture.rs` with
the private watchdog. Observe `drain-requested` within five seconds of spawn,
then allow that duration plus one second. Assert both phases precede the child
emergency limit. Keep ordinary scenarios and parent-death probe budgets intact.

## Outcomes & Retrospective

All 22 focused subprocess entries pass in 10.01 seconds. Both Rust matrices pass
128 foundation entries, 149 workspace entries and two doctests; all three HTTP
smoke modes pass. The extended-deadline control rejects a mutation restoring
the old five-second comparison (exit 101, 5.00 seconds); source was restored
byte-for-byte before verification. See `docs/validation.md` for exact versions,
commands and logs; use Jig receipts and plan resolution for final gate status.
No library/API/dependency or platform-scope changes. macOS remains unverified.

## Execution and validation

Change `crates/batter/tests/non_yielding/{fixture,watchdog,watchdog_tests}.rs`.
Update `docs/{guarantees,testing,status,validation}.md` and the owning Bead.
Focused verification is `cargo test -p batter --test non_yielding --locked`.
Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`,
rebuild `cargo build -p batter-axum --example http_service --locked`, then run
`python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in
default, `--signal SIGINT`, and `--deadline` modes. Run Jig work check/evidence/
gates for this plan, finish backend checks with `scripts/jig check test`, and
finish the plan only after required gates pass. Expected: 22 subprocess entries,
128 foundation entries, 149 workspace entries and two doctests pass on Linux.
Record actual results and macOS limits in validation. Logs live under ignored
`.agent/tmp/batter-fvz-startup/`. Commands are rerunnable; restore only temporary
mutation edits if interrupted, preserving all preexisting work. Do not commit.
