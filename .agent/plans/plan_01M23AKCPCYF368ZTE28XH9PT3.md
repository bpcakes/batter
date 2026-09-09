# Reconcile finite shutdown causes and shared reports

Owning Bead: `batter-bzr`. Baseline: `fbe77addaafc8709c95d7ecf4982dd3ceea3f41d`,
fast-forwarded from `61a025f5ed3699405034c1407942cdb4c280297c`.
Recovery stash: `7bbb97f910fc24ee9d5356a77d5fb18c2d36480c`.

## Progress

- Upstream is integrated; the local edits are restored and unstaged.
- The moved concrete-source test also asserts `FiniteTaskExit("shared-error")`.
- Both status descriptions and validation histories are retained. Original
  Beads records, append-only work records and untracked files were checked
  against the upstream commit and recovery stash.
- Focused lifecycle/process/cause tests and four PostgreSQL smoke controls pass.
- Both Rust 1.98.1 and 1.94.0 full matrices passed 467 executions with zero
  failures and three explicitly live database tests ignored. Both rebuilt HTTP
  examples passed all five smoke profiles. Validation evidence is recorded.
- All five Jig gates and the final `scripts/jig check test` passed. Gate and
  backend logs are `/tmp/batter-finite-reconcile-jig-check.json` and
  `/tmp/batter-finite-reconcile-final-backend.log`.

## Surprises & Discoveries

The upstream test edit conflicted with its local replacement in
`tests/process_ownership/error_sources.rs`; choosing either side alone would
lose an assertion. The combined `lifecycle.rs` additions also exceeded the
existing file-size limit (811 lines). The direct budget check failed before
the report extraction and passed afterward (746 lines).

## Decision Log

Preserve both error-source identity and finite-cause classification in the
expanded regression. Move the unchanged `ShutdownReport` definition and
implementations to private `src/lifecycle/report.rs` and re-export it at its
existing public path. Preserve compile-fail rustdoc and report behavior.
Use additive Beads reconciliation: one upstream issue was imported, with no
existing issue updates or deletions. Do not commit or publish.

## Outcomes & Retrospective

Conflict resolution, both full verification matrices, HTTP smokes, all Jig
gates and the final backend check are complete. Existing macOS and PostgreSQL
evidence retains its original scope. Cargo.lock is unchanged, no conflicts or
staged changes remain, and no application commit was created. The recovery
stash retains the complete original working changes.

## Validation and recovery

From the repository root, run `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Rebuild the HTTP example on
each toolchain and execute all five `scripts/smoke_http.py` profiles from
`docs/testing.md`. Logs use `/tmp/batter-finite-reconcile-*`.
Update `docs/validation.md` with actual results, then run `scripts/jig work check`,
`scripts/jig check test`, `scripts/jig work evidence`, `scripts/jig work gates`,
and `scripts/jig work finish` for this plan. All required gates must pass.

Retain the recovery stash until the user decides it is unnecessary. Do not
reapply it over the reconciled edits; inspect its original tracked and untracked
snapshots if recovery is needed. No dependency or database migration changes
are intended.
