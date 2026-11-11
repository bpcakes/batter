# Flags evaluation 2 (control plane), 2026-11-08

Owning task: `batter-jkw6.24`. A fresh agent built a flag-aware service through
Batter's public documentation, rustdoc and examples, and the result was scored
against an oracle frozen before launch.

## Method

- Oracle: [`oracle.md`](oracle.md), committed before launch at `2c71d847`
  (blob `29fd9b54a165e4937409e6c5123ec16584958e73`). The agent was told not to
  open `docs/evidence/`.
- Task: [`task.md`](task.md), product requirements; it names no Batter type
  the oracle checks. The starting project already supplied the one `batter`
  dependency with its three features and the panic hook as the first statement
  of `main`, so items 1 and 9 show that the agent kept that scaffolding, not that
  it discovered it.
- Agent: one fresh general-purpose agent with no other context, the same model
  family as the maintainer that wrote the oracle; 186 tool uses, about 26
  minutes. Its final report is [`report.md`](report.md); the full transcript is
  not archived.
- Library: the flags integration branch at `63fe812a`.
- The consumer is archived in [`consumer/`](consumer/) as the agent left it,
  except an empty `[workspace]` table added so the copy builds standalone inside
  this repository. It path-depends on `crates/batter` by absolute path.
- The consumer builds against `63fe812a`. `master`'s `f6e076d4`, merged into
  the branch at `29c0c0d2`, made `batter::runledger::job_phases` private, so
  `consumer/src/receipts.rs` does not type-check against any revision that
  includes that merge.
  To rebuild it, check out `63fe812a` in a separate worktree and point the
  path dependency there; item 10's recheck ran before the merge, against that
  library source.

## Scores

| # | Oracle item | Score | Evidence |
| --- | --- | --- | --- |
| 1 | only `batter`, with the three flags features | met | `consumer/Cargo.toml` |
| 2 | `RegisteredFlags` from `control_plane::register_in` over `declared::resolve`'s `Resolved` from the root's `SettingsSource` | met | `src/main.rs:46-51`, `src/flags.rs:72-78`, `src/serve.rs:104` |
| 3 | the kill switch declares `SafeState` and `StaleRule` | met | `src/flags.rs:50` |
| 4 | `flags.http_gate` installed with `GroupPolicy::gated`, handlers read `AdmittedFlags` | met | `src/serve.rs:131`, `src/http.rs:80`, `:150`, `:177` |
| 5 | a `GatedHandler` from `gated_direct` with an explicit tripped policy and awaiting recheck | met | `src/receipts.rs:133-151` |
| 6 | `view.pin` into the application's row in the `run_atomic` that enqueues the job; `resolve_pinned` matching every arm with no wildcard | met | `src/http.rs:91`, `src/orders.rs:86-106`, `src/receipts.rs:237` and `:93-122` |
| 7 | after exit 4, read the revision and re-run with `--if-revision`, never a blind `--yes` | partly met | every write keeps the original `--if-revision` and none uses `--yes` (`ops/engage-halt.sh:106-141`, `:270`); but when an exit-4 report carries no `proposed_version`, `committed_at` returns before reading `history` (`:169-170`) and the loop re-issues without a read (`:271`) |
| 8 | no process-environment mutation; tests inject a `SettingsSource` | met | no `set_var` or `remove_var`; `src/flags.rs:102-103`; the script tests set the child's environment |
| 9 | `redacting_panic_hook` first in `main` | met | `src/main.rs:61` |
| 10 | `SQLX_OFFLINE=true cargo check --offline --all-targets` | met | rechecked by the maintainer on the archived copy: check passes, 12 unit and 11 script tests pass |

Nine items are met and one, item 7, is partly met. The service never ran
against PostgreSQL: its
migrations, schema verification, flags registration, refresh and LISTEN, the
worker and the operator command against a real store are unexercised.

## Friction the agent reported

This summary is selective; [`report.md`](report.md) has the complete list. The
items are what the agent reported in this run, under its offline setup, not
independently established API limits. Design signals for the canonical path
(ADR-010), opened as follow-ups `batter-qrqy`, `batter-alk2`, `batter-31p9`,
`batter-owtx`, `batter-xuv6` and, for the smaller items, `batter-aalq`:

1. The flags example, `crates/batter/examples/rollout.rs`, starts its
   supervisor directly and migrates with a hand-built pool, while the facade's
   and `control_plane`'s rustdoc point to `Startup::protected`; no example
   combines the HTTP gate, the job gate and a pin.
2. `JobFlags` has no test constructor, and in this offline setup the
   worker-free handler drive's `runledger-test-support` dependency, with its
   Docker crates, did not resolve, so the agent did not drive the gated handler
   itself in a test.
3. The agent found that `job_phases(...)?.into_operation(work, finalize)`, which
   takes owned callbacks, did not fit its flag-aware handler, which borrows
   `JobFlags` and reads its pin from the row first, and fell back to the manual
   sequencing the adapter labels "not the protected path"; whether a protected
   composition exists is the follow-up's question (`batter-31p9`). `master`
   has since removed public `job_phases` and added `PhasedJob` with a
   `PhasedJobHandler` whose work and finalization borrow the invocation; that
   path postdates this run and is unevaluated here.
4. Before the first publication the job gate always consumes an attempt, so a
   long first-boot outage of the flags store can dead-letter jobs whatever the
   tripped policy says (disclosed in ADR-016).
5. The operator command's JSON payloads are undocumented, "re-run with
   `--if-revision`" does not say which revision (the safe one is the original,
   which relies on the store answering an already-satisfied write `unchanged`
   before its revision check), and reads that fail report "nothing written",
   with a refused connection shown as a timeout.

Smaller items: the operator command is unpublished; `flags-postgres` selects no
TLS provider; there is no documented way to refuse ambient `PG*` variables;
there is no helper for today's `CivilDate`; sharing SQLx's migration ledger with
the application's migrator is undocumented; narrowing settings before
`declared::resolve` silently resets flags to their defaults;
`SharedCommandReport` has no `Display`.

## Limits

One run of one agent from the same model family is a usability signal, not a
measurement. The agent also read the operator command's and the store's
source, beyond the documentation the task named, to answer its friction items
5 and 6. Passing offline checks prove the consumer type-checks and its offline
tests pass, not that the service works against a database.
