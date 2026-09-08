# Derive subprocess synchronization from its timing policy

Owning Bead: `batter-fvz`. Follow `.agent/PLANS.md`. This follow-up repairs the
private test harness after the independent Claude/Codex/Cursor review. It does
not change library behavior, public interfaces, dependencies or platform scope.

## Progress

- [x] 2026-09-08: Read review reports, current code, prior plans and primary sources.
- [x] 2026-09-08: Confirm the authorized macOS host remains reachable.
- [x] 2026-09-08: Derive synchronization bounds, close final-output race and add failure controls.
- [x] 2026-09-08: Three targeted mutations rejected; both toolchain matrices and HTTP modes pass on both hosts.
- [x] 2026-09-08: Recorded research/evidence, closed Bead, passed fresh Jig gates and final backend test.

## Surprises & Discoveries

The panic synchronization guard repeats the five-second startup budget even
though the fixture waits two seconds after drain before panicking. The existing
pure policy already supports five seconds of startup plus three of observation.
The environment control protects the removed ambient launch mechanism; it is
an intentional regression test, not a current environment input. The macOS job
was deliberately focused; existing full host validation is distinct from hosted
CI. Only the emergency fallback control deliberately consumes ten seconds;
the other parent-death probes exit as soon as their evidence is available.

## Decision Log

Derive the maximum synchronization allowance from the existing WaitPolicy,
rather than adding configurable deadlines or more timing layers. Retain the
real ten-second emergency test and the two-second observation: they establish
the actual fallback and exceed the simulated lifecycle budget. No OS scheduling
margin is a guarantee under starvation; retain fail-closed status validation.

When observing a child exit during an event wait, join capture and re-evaluate
complete evidence before declaring it missing. Use explicit overflow causes so
a live event-overflow control can prove continued draining independently of a
later byte overflow. Keep fixed bounds and private helpers; add no framework.

Python's single owner-PID/fixture-ready deadline bounds one startup phase, just
like Rust's spawn-to-drain deadline. Keep it. SIGALRM is a last-resort hard stop,
not cleanup; do not introduce asynchronous exception handlers and imply stronger
cleanup. Document this limit. Hosted CI cannot validate uncommitted source;
execute the full matrix on the previously authorized SSH host instead.

## Outcomes & Retrospective

Implementation and host validation are complete. Linux passes 48 focused / 175
workspace entries plus two doctests; macOS passes 46 focused / 173 workspace
entries plus two doctests. The original 44-entry Linux review passed; new tests
cover the policy boundary, exited-child event presence/absence and live metadata
overflow. Three mutations reject shortened synchronization, stopped draining,
and delayed actual kills. No scheduling flake was reproduced on either host;
the exact-instant control proves the policy mismatch without near-deadline sleeps.
All 67 source/build hashes match. Full commands, snapshot hashes, research answers
and remaining hosted-CI limits are recorded in docs/references.md and
docs/validation.md. All five Jig targets and the fresh required verify gate pass;
the final `scripts/jig check test` also exits 0. Bead `batter-fvz` is closed and
the export refreshed. The worktree remains uncommitted with the user's index
preserved. Hosted macOS CI remains unexecuted; no input was needed for the
authorized local/SSH work.

## Context and orientation

`crates/batter/tests/non_yielding/timing.rs` owns spawn-relative deadlines;
`evidence.rs` records bounded complete event lines and their capture instants;
`capture.rs` continuously drains pipes on a joined OS thread. `watchdog.rs`
owns child kill/reap and the actual wait deadline. `watchdog_tests.rs` drives
live children; `fixture.rs` supplies deliberately blocked and invalid controls.
The Python probe owns Linux-only adoption/reaping. No database work is involved.

## Plan of work and milestones

First expose the maximum duration from the existing closed wait policy and use
it for panic synchronization and emergency ordering. Exercise exact late-startup
instants deterministically. Then make event resolution recheck joined final
capture on child exit, with successful and missing-event controls. Add a fixture
that exceeds the event bound, floods the pipe, and exits; require ordinary exit
and rejected evidence with the event cause retained. Preserve byte-overflow tests.
Check kill-request upper bounds against the existing observation allowance in the
live wiring control, without claiming a hard OS scheduling guarantee.

Second verify these changes with focused tests and bounded mutations, restoring
exact files after each mutation. Update guarantees, testing, status and research
answers. Finally validate identical source on Linux and macOS and finish tracker
and workflow evidence.

## Concrete steps and validation

Run from `/home/aa/Documents/batter`:

    cargo test -p batter --test non_yielding --locked
    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline

Repeat full verification and rebuilt smoke modes on `aa@torque.local` in a fresh
temporary source directory. Reuse only its existing target cache, compare source
hashes, and retain logs in `.agent/tmp/batter-fvz-review`. Run `scripts/jig work
check`, `work evidence`, `work gates`, and `work finish` for this plan. Finish
backend verification with `scripts/jig check test`. All commands must pass;
negative controls must fail for their intended diagnostic. No skipped platform
check or configured hosted job is runtime evidence.

## Idempotence, recovery and interfaces

The checkout has staged user changes. Preserve the index; apply this follow-up
only as working-tree edits. Back up exact touched bytes before mutations and
restore them even on failure. Keep logs and snapshots ignored. No commits,
pushes, publication, dependency changes or new public APIs are authorized.

Completion note, 2026-09-08: updated outcomes and progress after the complete
two-host matrix, three rejected mutations, source-hash comparison and fresh Jig
gates. Current evidence is named explicitly in docs/validation.md to distinguish
older logs already present in the shared ignored evidence directory.
