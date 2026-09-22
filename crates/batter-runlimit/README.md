# batter-runlimit

Optional native atomic quota-before-work execution for Batter. No default
features; `memory`, `postgres` and `axum` can be selected independently.

```sh
cargo run -p batter --features runlimit-memory,runlimit-axum --example quota_service --locked
cargo test -p batter-runlimit --all-features --locked
python3 scripts/check_runlimit_features.py
```

`Quota::run(&context, Checks::new(&checks)?, work_factory)` keeps native quota
decisions separate from the work result. Checks run once; work shares the same
total budget. Quota denial/backend failure prevents factory invocation. Cancellation
after a grant does not refund or erase consumption. Native algorithms, atomicity,
policy validation, opaque keys and persistence remain in the native packages
under `runlimit/`. See [import provenance](../../runlimit/IMPORT.md).
Allowed batches yield native validated `Allowance` values. Enforced and shadow
denials retain the native index and nonzero evaluated batch size as well as the
typed denial details.

`HttpQuota::new(quota, policies, authenticate, subject)?.prepare(policy, routes)`
guards every supplied route. It rejects empty and mixed-mode policy sets at
construction. Call `.with_public_probes(PublicProbes::new().get("/live", handler)?)`
before `prepare` to deliberately add a public GET/HEAD probe. Probes bypass
lifecycle/deadline admission, authentication and quota, and receive no protected
`OperationContext`. The builder accepts only literal GET paths and
returns a typed error for capture or wildcard patterns. It cannot accept an
arbitrary Router or fallback. A public and protected GET at the same path
panic during `prepare`, before serving. Use distinct GET paths.
Handler principals arrive through the
`Authenticated<P>` extractor, not `Extension<P>`; its value cannot be constructed
outside the adapter. Authentication receives no request body, and subject
selection uses its result and the actual peer, not forwarded headers. Closure
signatures are checked at `new`. Prepared serving installs peer metadata and one
retained HTTP observer; the alternative test transport requires an explicit
synthetic peer. See the
[facade compiling example](../batter/examples/quota_service.rs), [API docs](src/http.rs) and
[integration contract](../../docs/integrations.md#runlimit-optional-protected-native-quota-adapter).

## Outcome-aware attempts

The optional `postgres` feature also exposes `attempts::AttemptRunner` and the
exact pinned `native`/`postgres` namespaces. The runner owns one native attempt
reservation, credential verification outside a transaction, and an owned SQLx
transaction that claims the receipt, invokes final application checks/writes,
and completes retry state. Its module rustdoc is a compiling consumer example.

Construct it with `AttemptRunner::new(database)?`, where `database` is a
`batter_sqlx::PgProfiledPool`. Declare login/effective roles, one authoritative
schema, baseline timeouts and any custom settings in `PgSessionProfile` before
connecting. Arbitrary pools and fallback schema lists are not accepted. The
shared foundation owns connect/acquire/release normalization; completion applies
the same profile after reset and revalidates it. Native admission still owns its
transaction budget and may tighten timeouts. Provisioning/grants remain external.

The application callback returns `Authentication::Accepted(value)` or
`Authentication::Rejected(reason)`. Both are committed domain outcomes. Rejection
therefore preserves failure audit together with the incremented failure state.
An operational callback error rolls everything back. The final decision belongs
inside this transaction because replay and account-status checks can invalidate
credentials that passed expensive verification. A stale claim prevents callback
invocation. A claim acquired before lease expiry holds its native row lock through
completion and commit, so verification lease expiry does not invalidate an already
claimed transaction. No raw connection, attempt receipt, or separate completion
call escapes the canonical runner.

Admission, verification and completion share the supplied `OperationContext`
deadline. The SQLx `run_atomic_profiled_in` helper retains acknowledged output and native
uncertainty before deadline resolution. No automatic retry follows an uncertain
commit. Interrupted verification leaves a native lease for conservative expiry;
it does not reset failures. Observations report completion only after commit.
Observer panics are isolated, and default result/error formatting redacts payloads.

Peer/global traffic checks remain `Quota::run` with native fixed-window or GCRA
backends. Nest the attempt runner in its work factory using the provided child
context to share the total budget. Successful authentication resets only native
attempt state; it never refunds traffic quota. `HttpQuota` remains an authenticated
principal quota boundary; it is not the pre-authentication runner.

Consumers explicitly install Runlimit's additive attempts/GCRA migrations or
vendor their SQL into forward-only application migrations. Batter does not
provision, migrate, prepare or maintain a database. Existing `Quota`/`HttpQuota`
workflow semantics stay unchanged. Updating the older Runlimit pin requires source
adaptation for native types (`CheckAllError`, bound subjects, exhaustive `Denial`,
validated capacities and batch errors); the attempt feature itself is additive.

The explicit PostgreSQL 18 acceptance command is:

```sh
DATABASE_URL=... cargo test -p batter-runlimit --features postgres --test attempts_live -- --ignored
```

Tests cover failure audit, successful reset without audit deletion, final replay
rejection, stale claims, operational rollback, one total budget, cancelled
verification, acknowledged commit followed by cancellation, lease expiry during
a claimed transaction, and unconfirmed commit without replay. They do not prove
runtime-death survival, termination of arbitrary detached work, or rollback of
external effects. Fresh-agent consumer evaluation remains unexecuted.

## Invalid-state review (ADR-010)

- One constructor binds native admission and atomic completion to the same pool.
- Library-owned ordering prevents forgotten completion, work before admission,
  success publication before commit, and stale receipt application callbacks.
- The final decision enum keeps durable rejection separate from transaction error;
  credentials and business meaning remain application policy.
- The opaque SQL capability prevents replacing the retained connection. Runlimit
  owns receipt and transaction fencing; SQLx owns transaction-boundary detection.
- The canonical runner never exposes a staged transition as confirmed. Uncertainty
  retains its provisional output under the native atomic error, with no replay.
- Arbitrary application SQL, captured pools, exported values, and external effects
  are not sandboxed. The API does not claim to prove those remote behaviors.

Response-body streaming remains outside HTTP response construction. Version
0.0.1 targets crates.io together with native Runlimit 0.4.0.
