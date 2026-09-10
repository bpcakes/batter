# Acknowledge native graceful shutdown in HTTP lifetime tests

Owning Bead: batter-mhp, follow-up to batter-u0m. Baseline master 486e0b0 with
reconciled local observations suite present. Preserve existing uncommitted work.

## Progress

- [x] Read fixture, adapter and resolved Axum 0.8.9 connection shutdown source.
- [x] Added shared test-only native milestone observation and held-work checkpoints in both suites.
- [x] Four unmodified-Axum cases pass; all four reject premature connection closure and all four reject producer-only evidence in a private build directory.
- [x] Updated contracts/status/references/validation, executed both toolchains and ten smokes; final Jig gates passed. api:test receipt_01M25N9GTTYFE6CBDSPGCKDNVE. Minimum no-default core failure remains batter-gg4.

## Surprises & Discoveries

The signal future's completion precedes separately scheduled connection handling.
Axum's connection-task trace immediately precedes synchronous graceful_shutdown;
these fixtures run exclusively on one runtime thread. Observing the native event
from a subsequently polled test proves that call has returned. An INFO-only or
changed native trace must fail closed rather than falling back to the producer
marker. This is a version-specific compatibility observation, not an adapter API.

## Decision Log

Keep production code unchanged. Enable only axum::serve TRACE alongside existing
INFO capture. Share the observation helper between the two HTTP targets. Rename
the producer event to graceful-signal-ready to avoid acknowledgement terminology.
Hold cooperative handler/body work after the native event, check liveness and
unfinished completion for a bounded observation interval, then explicitly release.
Use a disposable snapshot plus copied Axum source for mutation evidence: replace
the native graceful_shutdown call with immediate connection-loop exit. Never edit
the registry cache or the working Cargo.lock. The independent tracing issue
batter-gg4 remains separate; record any recurrence without hiding it.

## Outcomes & Retrospective

The repaired suites have passed focused runs and all four cooperative cases have
passed the unmodified native implementation in an isolated copy. Both adverse
variants are rejected at their intended assertions. Rust 1.98 full verification
passed 668 executions and all ten HTTP smokes passed. Minimum-toolchain workspace
tests and separately executed doctests/Clippy/rustdoc passed, but its no-default
core pass reproduced unchanged subscriber failure batter-gg4. This qualifies the
full verification result; it does not invalidate the independently passing HTTP
cases. Final archival Jig gates passed, with api:test receipt_01M25N9GTTYFE6CBDSPGCKDNVE; no later source/test/environment changes.

## Concrete steps and acceptance

Edit tests/support/http_graceful.rs, both HTTP root targets, and their scenario/
fixture files. A shared helper waits with a deadline for the exact native target
and event; assert CurrentThread runtime. Require native event before held-work
checkpoint before explicit release. Existing framing/drop/report/cleanup assertions
remain. Run cargo test -p batter-axum --test http_lifetime --test
http_lifetime_observations --locked. In an isolated snapshot with a local patched
Axum 0.8.9, each cooperative handler/stream case must fail on missing live resource
before release, rather than timeout/compilation failure. Record reproducible steps.

Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh; build
http_service and execute all five documented smoke profiles under each compiler.
Update docs/guarantees.md, docs/testing.md, docs/status.md, docs/references.md and
docs/validation.md, then owning Bead. Inspect Jig evidence/gates before final work
check and finish. No commits, publishing or new platform claims.

## Executed mutation evidence

`python3 /tmp/batter-http-graceful-mutation.py` produced final isolated evidence at
`/tmp/batter-http-graceful-mutation-xzq44i7b/evidence.json`. An exact copy of that
runner is retained alongside this plan as `plan_01M25MJHFHKRKTZXF2PNHCG6DG-mutation.py`.
It uses this Linux checkout/cache's absolute paths; adjust the two source paths
when reproducing on another checkout. It copies the workspace and Axum 0.8.9,
adds a path patch only in the disposable manifest, uses a private target directory,
and verifies that the copied lock differs only by Axum's path source identity.
It never edits the working lock or registry source.

The original variant passes all four exact Cargo parent cases. The premature-close
variant replaces the single `conn.as_mut().graceful_shutdown();` statement with
`break;`, preserving the native event. All four fail for dropped resources or
non-pending wire, after the acknowledgement. The producer-only variant restores
graceful behavior but changes only the native connection event message. All four
fail for missing native acknowledgement despite the producer marker. Builds must
succeed, exact tests must execute, and each adverse exit must be 101 with the
specific expected assertion. Compilation failure or watchdog expiry is not success.

Initial verification passed runtime tests but failed Clippy complexity checks;
named helpers preserve the assertions, and final focused Clippy passes. A repeated
mutation setup initially reused a stale path-patch artifact in a shared target;
that baseline failed and was discarded. The final twelve-run evidence uses a fresh
private build directory. Earlier logs remain under `/tmp` and are not final evidence.
