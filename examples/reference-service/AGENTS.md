# Reference compatibility package

## Purpose

Own the runnable generic delivery command, native SQLx/Runledger compatibility
probes, and reusable external-harness fixture acceptance. This unpublished
Unix-only application is not a reusable database framework.

## Key entrypoints

- `src/main.rs` and `src/runtime.rs` own the command-only process root.
- `src/delivery.rs` owns command identity, the one-transaction submission and
  durable owner-scoped projections.
- `src/http.rs` and `src/auth.rs` own authenticated command/reconciliation routes.
- `src/schema.rs` owns repeated migration/compatibility/producer-definition startup.
- `src/lib.rs` also preserves native SQLx type identity probes.
- `src/config.rs` and `src/config/` own the example settings schema, validated
  native constructor inputs, TCP URL subset and PG*/passfile restrictions.
- `tests/configuration.rs` covers source policy, bounds, redaction, native
  consumers and partial startup without PostgreSQL.
- `tests/reference_live.rs` owns explicitly ignored live probes.
- `examples/reference_preflight.rs` and `tests/support/live_endpoint.rs` share
  native preflight/fixture policy; Python owns scheduling and inventory only.
- `../../scripts/test_reference_live.sh` selects and verifies live execution.

## Edit here for X

Keep integration probes and application migrations here. Provisioning stays in
postgres-test-harness, supervision in Runledger, and generic test support remains
independent. Update `../../docs/reference-compatibility.md` with executed evidence.

## Invariants

Settings names/defaults/precedence and required passwords stay in this root.
Use shared `batter::settings` mechanics, retain concrete causes behind static
diagnostics, and pass validated outputs to native constructors without fallback.
Never use native from_env/builder_from_env or raw URL parsing after this validation.
The private live-endpoint handoff canonicalizes selected host/database/TLS values,
preserves explicit empty passwords and encodes query spaces as `%20` for the
external harness's required URL API. Reject IPv6 literals at this private live
boundary; SQLx's URL consumer cannot use their brackets for TCP lookup. Direct
root native IPv6 options remain supported. Native parser tests guard the handoff.
Use one native SQLx graph and explicit application-owned transactions. Delivery
submission uses one `PgLease` and one READ COMMITTED transaction for command,
delivery and Runledger rows. Exact replay never re-enqueues. Owner/key identity,
record generation, canonical payload and immutable enqueue fields stay retained;
uncertain commit/rollback acknowledgement is reconciled by owner/key and is never
automatically retried. The producer definition has no handler until the owning
worker/provider tasks add one. Close pools
before consuming leases; explicitly drain deferred cleanup. Startup witnesses
prove only their observed job path. Ordinary workspace tests never require a live
database. Native option tests cross the cleared-environment child boundary;
the matrix also tests hostile parent PG* state. The ordinary configuration target
requires enabled IPv6 loopback (`::1`) for its native protocol fixture, plus
IPv4 loopback and Unix subprocess permissions. This test is not skipped when
the host or container lacks IPv6. Never mutate process globals.
Explicit live invocation fails when prerequisites are missing.

## Common commands

From the root, run `cargo check -p batter-example-reference-service --all-targets
--all-features --locked` and `bash scripts/test_reference_live.sh`. Follow root
two-toolchain, HTTP smoke and Jig verification requirements.

The live inventory requires two distinct disposable local PostgreSQL 18 clusters:
`POSTGRES_TEST_ADMIN_URL` and `POSTGRES_TEST_OBSERVER_URL`. The primary uses a
superuser, SCRAM host authentication, track_counts and autovacuum with naptime <= 5s. The
startup negative control pauses a raw SCRAM handshake and must close its socket
before awaiting DROP completion. The autovacuum control disables further ordinary
vacuum launches before its explicit retry. The startup control must witness the
target DROP's ProcSignalBarrier wait, not only a short pending duration. Its
before/after PID identity requires no other connection producers on the dedicated
serial server; forwarded client ports need not match server-observed ports.
Keep these prerequisites and the strict
native preflight and Python inventory controls synchronized with docs/testing.md.

Identify the autovacuum worker with pg_stat_progress_vacuum and the prepared table's
actual relation OID, with substantial heap scanning remaining. Database-wide worker
presence alone can select a short-lived launcher visit and is not the test witness.

The minimal fixture uses the adapter SessionObserver on its own one-slot admin
pool; body/catalog diagnostics use another one-slot pool. Keep both outside
closing application capacity. Failed session observations retain leases until
explicit retry and remain errors after recovery. Real detached-session, deferred
failure and runtime-loss probes belong in this package. Keep their acknowledged
locks, independent identities and Python inventory/watchdog coverage intact.

Use `support/fixture_completion.rs` for bounded observed completion. Its pending
error retains the actual run/control/admin pools and prints only phase/count
summaries. Preserve typed recovery; never abort/drop leases to manufacture success.
The same bound covers admin closure after driver completion. A driver JoinError
closes both admin capacities before propagation; a blocked close returns the same
recoverable owner with its cached error. Successful completion keeps diagnostics
open for caller-owned catalog checks and awaited close.
Keep detached-session setup acknowledged before body return, and witness backend
exit before its short explicit retry. `fixture_retry_gate.rs` owns advisory-lock
gating of observer connection initialization and bounded completion recovery on
both Tokio runtime flavors; ordinary diagnostics must use a separate pool.

Temporary restricted logins use fresh random passwords with five-minute expiry
and belong to the outer test cleanup owner. Expiry limits password authentication;
it does not remove roles after process death. Join assertion failures before
closing their pools and dropping the role. Retained observer pools
close concurrently, so a blocked original checkout cannot delay initiating close
on later replacements. Both endpoints must allow pg_control_system for distinct
cluster identity preflight. PostgreSQL 18.4 permits this by default; if revoked,
the secondary login needs an EXECUTE grant on pg_catalog.pg_control_system().

Retain a caller clone of the diagnostic pool. Retry replacement preserves old pools
and does not repair their sessions; the pending owner exposes `observer_pools()`
for explicit native repair. For an observer inside a disposable database, close
its retained pool and witness backend exit before retrying;
fixture_sessions.rs exercises this through the recoverable completion helper.
Successful completion leaves the caller's diagnostic clone open for explicit close.
