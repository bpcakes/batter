# Fresh atomic consumer exercise, 2026-09-21

Executed for `batter-ga1`, after the retained-poison/narrow-error implementation.
One agent received no conversation history or review/design hints. Its task was
to discover the current public API and implement an application audit write,
intent recording and enqueue in one transaction, returning a job ID only after
acknowledged commit, retaining failure/reconciliation information and not retrying.
It could inspect public documentation/source and applicable AGENTS guidance, but
not reviews, earlier plans/evidence or repository tests. Only a scratch consumer
under `/tmp/runledger-fresh-consumer.CE5FUS` was writable by that agent.

## Observed result

The unedited implementation is preserved in [consumer.rs](consumer.rs). Its first
choice was `runledger_postgres::run_atomic`, based on the public READMEs/rustdoc.
It used the initial intent phase, consumed it into queue phase, and exhaustively
matched Begin/Rejected/Uncertain. It retained the native narrow uncertainty rather
than inventing a completion protocol. Success constructed a private-field
`Committed` only after the runner returned. No raw transaction control or manual
completion appeared. The first compile succeeded without Rust fixes; formatting
then wrapped one import.

The integrator inspected the exact source and checked its behavior against the
task: all three writes share the runner, conflicted intents reject, original
operation/cleanup errors remain typed, and uncertain output is not returned as
success. Independent intent/request correspondence is deliberately not promised.

Executed commands in the scratch crate (Rust 1.98.1, Linux x86_64):

```sh
SQLX_OFFLINE=true cargo +1.98.1 check --offline --locked
cargo +1.98.1 fmt --check
```

Both passed. The scratch manifest used path dependencies on `runledger-postgres`
and `batter-sqlx`, plus SQLx 0.9 with runtime-tokio/postgres/uuid. To reproduce,
place consumer.rs at `src/lib.rs` of an isolated edition-2024 crate using these
sibling sources, generate its lockfile, and run the commands above. Application
table prerequisite (not executed by this exercise):

```sql
CREATE TABLE public.application_audit (id uuid PRIMARY KEY, action text NOT NULL);
```

## Discovered gap and limits

The agent found contradictory primary advice in Runledger's downstream guide:
the opening durable-handoff example still used pool.begin()/the native `_tx`
helper and obsolete outcome.status field. That guide now leads with run_atomic
and status(). The exercise therefore found and caused a concrete documentation
repair, despite selecting the protected API successfully.

Independent intent/request inputs cannot prove same-job correspondence. That is
application policy, not a transaction-disposition guarantee. Cancellation still
returns no result and requires externally retained reconciliation inputs.

This is one compile-only sample, using direct crates rather than the Batter
facade. It is not a usability-rate estimate, an independent code audit, database
execution, or evidence that arbitrary callback side effects are sandboxed. Live
atomicity/error coverage belongs to the separately executed PostgreSQL 18 suite.
