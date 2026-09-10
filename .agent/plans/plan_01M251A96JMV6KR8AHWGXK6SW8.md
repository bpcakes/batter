# Integrate upstream and validate the complete workspace

Owning Bead: batter-s03. User requested pulling and recording upstream merges and ensuring the full suite passes. Preserve uncommitted fixture work and both local commits; publication and push are not authorized. Baseline: 662f2ba7271ba332f6ffdab0114d45ab59688569.

## Progress

- [x] Fetch origin and inspect both histories and dependency pins.
- [x] Back up/stash uncommitted work, merge origin/master, restore fixture work and tracker history.
- [x] Regenerate workspace lock metadata through Cargo and check all targets/features.
- [x] Run both toolchain matrices, all live database cases and HTTP/PostgreSQL process smokes.
- [x] Record executed evidence, close the Bead and pass final Jig gates; Jig records plan/session closure.

## Surprises & Discoveries

Origin/master advanced by three commits while the local branch had two commits. The merge required retaining both lifecycle tasks and Unix signal modules, both testing sections, and complete validation histories at the top and bottom of the document. An initial conflict-resolution assertion incorrectly expected both validation tails to match; the local tail additionally held SQLx/startup/health evidence. The premature local merge commit was immediately repaired and amended before restoring fixture work. No conflict markers remain, and no incomplete merge was pushed. Restoring the stash required another validation-history reconciliation. All saved files and prior append-only records were retained. The external harness and job-runtime pins already match their remote master heads.

## Decision Log

Use a merge rather than rewriting local commit history. The completed merge is fa18aeab96cb84a94b03ba04fd4408222de670eb with parents 662f2ba7271ba332f6ffdab0114d45ab59688569 and 0f486036ec13d48d760665a8e41997ccd14cf7cd. Existing fixture work remains uncommitted. Cargo update --workspace --offline reported zero package-version changes. Import merged tracker JSONL without rebuilding or deleting records; three upstream issues were added. Keep the safety stash and backup until restoration and verification are confirmed.

## Outcomes & Retrospective

Merge, restoration and complete two-toolchain verification are finished. docs/validation.md records 575 executions per toolchain, all 29 live cases and seven process profiles each, with no failure. All five final Jig targets passed with fresh receipts.

## Context, steps and acceptance

Upstream adds filtered tracing-parent retention, private lifecycle task ownership, private HTTP observation and a daily scan configuration. Local changes add startup, health, SQLx and fixture composition. No scheduled scan is launched by this integration.

From /home/aa/Documents/batter run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Provision a dedicated local UTF-8 PostgreSQL 18 cluster outside repository code; run scripts/test_reference_live.sh (16 cases), scripts/test_sqlx_live.sh (10), and the postgres_lifecycle tests::live:: selection (3) on both toolchains. Rebuild and execute all five HTTP smoke profiles and PostgreSQL SIGTERM/SIGINT profiles on both toolchains. DATABASE_URL must be scoped to live commands, not ambient during compilation. Inspect independent catalog state and stop the test-owned server afterward. Record versions, commands, counts, failures/repairs and limitations in docs/validation.md, without broadening earlier macOS evidence to this Linux run.

Inspect Jig work evidence and gates, then run scripts/jig work check with this plan ID after final metadata. Require all five targets passed and fresh, including api:test. Preserve semantic assertions. If verification fails, repair the actual integration and repeat affected checks; do not relax assertions or claim the earlier snapshots validate changed code.

## Recovery

Backup files, manifest, patches and tracker database are under ignored .agent/tmp/batter-upstream-sync-20260910/. Safety stash is 7dff29f740177ae2d12820fb90b6a58c1b5f0ebc. Do not reset or overwrite the restored work. PostgreSQL provisioning is local test infrastructure only; no application database is changed. Completed plans retain historical evidence and Beads owns any further delivery scope.


The complete validation script exited 0. Independent catalog snapshots after both
toolchains retained the same three compatibility templates and no disposable
resources. The dedicated PostgreSQL 18.6 server was stopped with awaited shutdown.
No code/test assertion changed during verification. The safety stash remains as
an extra recovery copy; fixture changes and this validation record are uncommitted.


Final Jig api:test receipt: receipt_01M251SV7NNFGWXWQ5A6QP4D8Y. Work evidence
and gates both report passed/fresh for all five targets. The owning Bead is closed.
HEAD remains fa18aeab96cb84a94b03ba04fd4408222de670eb. During final verification,
origin/master also advanced to that same commit (remote-tracking reflog says
update by push); this task did not invoke a push. No local source input changed
with that remote-ref update. All requested integration and verification work is
complete; preserved fixture changes remain uncommitted.
