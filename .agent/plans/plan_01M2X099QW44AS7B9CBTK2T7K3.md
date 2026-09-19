# Upgrade Batter to Runlimit e91da419

Owning Bead: `batter-tbn`. Baseline: `d0e1f6fc31542311054019504c07680605170f9d`.
This plan grants no commit, push, publication or deployment.

## Progress

- [x] Verify the primary upstream Git head and inspect every change since the old pin.
- [x] Pin every active core, memory and PostgreSQL dependency and generated consumer to `e91da419216e77e8c83d0bb80c70c297d286d341`.
- [x] Adopt native `Allowance`, validated batch sizes, exhaustive consumption states and new PostgreSQL commit errors.
- [x] Update the facade example, failure-path tests, integration/testing contracts, primary-source references and implemented-status row.
- [x] Run focused tests, Clippy, eight direct feature combinations and facade identity checks.
- [x] Run full verification, the quota example and all five HTTP smoke profiles on Rust 1.98.1 and 1.94.0.
- [x] Record the final Jig receipt, close the Bead and finish this plan.

## Discoveries and decisions

The upstream `master` head on 2026-09-19 was `e91da419`. Core and memory remain
0.3.0, PostgreSQL remains 0.3.1 and the minimum remains Rust 1.94. Upstream now
uses `Allowance` wherever admission is known to have consumed quota and includes
a validated nonzero batch size with enforced and shadow batch denials.

Batter retains its named `AllowedBatch` result but stores native `Allowance`
values directly and yields them without an impossible denial match. Rejected and
shadow-admitted results retain the upstream `NonZeroUsize`. The HTTP consumption
projection now names all three exhaustive `ConsumptionStatus` variants. The
PostgreSQL bridge explicitly classifies `CommitOutcomeUnknown` and
`CommitTimedOut` as possibly consumed while keeping its wildcard for future
variants of the still-non-exhaustive error enum.

The first full Rust 1.98.1 run correctly failed because the facade identity
fixture still generated direct native dependencies at the old revision. Updating
that fixture removed the duplicate crate identities; its focused rerun and both
complete toolchain matrices then passed.

## Validation evidence

- `cargo test -p batter-runlimit --all-features --locked`: 1 unit, 36 HTTP, 20 quota and 11 doctest/compile-fail cases passed.
- `cargo clippy -p batter-runlimit --all-targets --all-features --locked -- -D warnings`: passed.
- `python3 scripts/check_runlimit_features.py`: all eight feature subsets passed.
- `python3 scripts/check_facade_features.py`: 14 positive graphs, eight negative gates and all-features type identity passed after the stale fixture pin was repaired.
- `bash scripts/verify.sh`: passed on Rust 1.98.1 after the expected earlier failed run exposed the stale generated-consumer pin.
- `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`: passed.
- `cargo run -p batter --features runlimit-memory,runlimit-axum --example quota_service --locked`: passed on both toolchains with native admission and HTTP `[200, 429]`.
- `scripts/smoke_http.py` default, SIGINT, deadline, WARN-filter and WARN-filter-plus-deadline profiles: all ten executions passed across the two toolchains.

Live PostgreSQL and Linux behavior were not executed and no new claim is made for
either. The supported local evidence is macOS arm64 on the two pinned toolchains.
