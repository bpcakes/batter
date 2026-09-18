# Update the native Runlimit pin

Owning Bead: `batter-9ms`. Baseline: `521f0e3`. This plan records the source
update and validation; it grants no commit, push, publication or deployment.

## Progress

- [x] Compare upstream `f147fb7b..346dc5e` against the current adapter.
- [x] Pin core, memory and PostgreSQL crates to one exact revision through Cargo.
- [x] Replace the obsolete eager-memory control with single and batch laziness checks.
- [x] Refresh integration, testing, source and status contracts.
- [x] Run both Rust verification matrices, HTTP smokes, quota examples and Jig gates.
- [x] Close the Bead and finish this Jig plan after reviewing the final diff.

## Surprises & Discoveries

Upstream kept core/memory at 0.3.0 and PostgreSQL at 0.3.1 while changing
unpolled memory futures, shadow-denial representation, and decision aliases.
Batter's adapter already uses `BatchDecisionView` and `.await`, so no public API
or execution-path edit was needed. The old negative control would reject the
fixed upstream behavior and had to be reversed.

## Decision Log

Pin Git commit `346dc5e6233995a8e2d8ad2d5d56a2d26ed3e664`, the upstream
`master` head verified during this task, instead of a moving branch. Test the
memory trait's single and batch paths locally; record GCRA as source-inspected.
Keep the previous `f147fb7b` validation section as historical evidence.

## Outcomes & Retrospective

The focused adapter suite, both full Rust toolchain matrices, ten HTTP process
profiles, both quota example runs, and the Jig verify gate passed on macOS arm64.
Live PostgreSQL, Linux and GCRA runtime behavior were not executed in this task.

## Context and orientation

`crates/batter-runlimit/Cargo.toml` owns the exact native Git revision.
`crates/batter-runlimit/tests/quota.rs` owns the local lazy-future regression.
`docs/integrations.md`, `docs/references.md`, `docs/status.md`, `docs/testing.md`
and `docs/validation.md` own the consumer contract and evidence. Native quota
algorithms, policy validation and storage remain in Runlimit.

## Concrete steps and validation

1. Inspect `git diff f147fb7b..346dc5e` in the upstream checkout and check the
   current remote HEAD.
2. Edit the three dependency pins and let Cargo regenerate `Cargo.lock`.
3. Update the native single/batch test and the affected contract text.
4. Run `cargo test -p batter-runlimit --all-features --locked`,
   `bash scripts/verify.sh`, and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
5. On each toolchain, build `batter-axum`'s `http_service`, run all five smoke
   profiles, and run the `quota_service` example.
6. Run `scripts/jig work check --plan-id plan_01M2TPS5T7GB26MJ5W1SG5QG09`;
   inspect `work evidence` and `work gates` before finishing the plan.

## Idempotence and recovery

The three pins are one revision; rerunning Cargo produces the same lockfile.
The tests and smoke commands are read-only apart from ignored build outputs.
If a check fails, repair the cause and rerun the affected gate before closing
the Bead. No database provisioning or external mutation is part of this work.
