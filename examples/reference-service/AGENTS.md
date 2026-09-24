# Reference compatibility package

## Purpose

Own the runnable generic delivery command, native SQLx/Runledger compatibility
probes, and reusable external-harness fixture acceptance. This unpublished
Unix-only application is not a reusable database framework.

## Key entrypoints

- `src/main.rs` and `src/runtime.rs` own the staged command/worker process root.
- `src/runtime.rs` uses protected startup with library-owned signals, reserves a
  cleanup slot before constructing the profiled database, and registers pool
  close before yielding. `src/database.rs` declares direct-login/public policy;
  native preparation and `batter::runledger` registration share that pool. It owns PostgreSQL
  health sampling, installs the delivery handler and approves ordinary readiness
  after registered components acknowledge initialization. Its startup failures downcast to
  `ProtectedRuntimeStartupFailure`; the earlier wrapper was removed in the
  coordinated hard cutover and must not be reintroduced.
- `tests/support/startup_process.rs` owns separately captured signal children and
  the production/test-fixture executable launchers;
  `tests/fixtures/signal_witness_fixture.rs` owns process-local signal
  acknowledgement outside the production entrypoint;
  `tests/support/protected_startup.rs` owns the test-only waiter/owner-loss
  composition. Case contracts are in
  `../../docs/testing.md#protected-startup-consumer-process-cases`.
- `../../crates/batter-runledger` owns native lifecycle translation; native
  descendant accounting remains in Runledger, not in this application.
- `src/delivery.rs` owns command identity, the one-transaction submission and
  durable owner-scoped projections.
- `src/delivery/worker.rs` owns provider admission, durable dispatch uncertainty,
  lookup-before-replay, generation fencing and terminal outcome projection;
  `src/delivery/worker/state.rs` owns its lease-fenced SQL state transitions.
- `src/provider.rs` owns the selected HTTP effect protocol. It is application
  code, not a generic provider adapter or an exactly-once claim.
- `src/http.rs` and `src/auth.rs` own authenticated command/reconciliation routes.
- `src/request.rs` owns direct-socket peer trust and combines it with the shared
  adapter correlation without owning application authority or request lifetime.
- `src/schema.rs` owns repeated migration/compatibility/producer-definition startup.
- `src/lib.rs` also preserves native SQLx type identity probes.
- `src/config.rs` and `src/config/` own the example settings schema, validated
  native constructor inputs, TCP URL subset and PG*/passfile restrictions.
- `tests/configuration.rs` covers source policy, bounds, redaction, native
  consumers and partial startup without PostgreSQL.
- `tests/reference_live.rs` owns explicitly ignored live probes.
- `src/retirement.rs` and `examples/retire_startup_controls.rs` own offline legacy
  maintenance through finite commands. Readback is separate from the retained
  primary report. `src/retirement/session/tests.rs` owns the ignored SQLx physical
  replacement regression, executed explicitly by the live runner.
- `examples/reference_preflight.rs` and `tests/support/live_endpoint.rs` share
  native preflight/fixture policy; Python owns scheduling and inventory only.
- `../../scripts/test_reference_live.sh` selects and verifies live execution.

## Edit here for X

Keep integration probes and application migrations here. Provisioning stays in
postgres-test-harness, supervision in Runledger, and generic test support remains
independent. Update `../../docs/reference-compatibility.md` with executed evidence.

## Invariants

Settings names/defaults/precedence and required passwords stay in this root.
Keep `ServingSettings` and database-only `MaintenanceSettings` concrete and
separate; do not restore a shared mode enum, optional serving capability, or
conversion from maintenance into serving. `runtime::run` accepts only the inert,
non-cloneable `PreparedServing` owner, and canonical `http::register_in` consumes
only `PreparedHttp` while inseparably selecting native peer registration.
`http::in_process_client` is the lower-level test seam. Its opaque
`InProcessRequestClient` cannot be served or expose the inner router, and each
request requires an exact synthetic peer. Maintenance ignores known serving-only names from captured
environment without parsing them, but dedicated files/overrides reject those
names and unknown reserved or PG* names still fail. Use shared `batter::settings` mechanics, retain concrete causes
behind static diagnostics, and pass validated outputs to native constructors
without fallback. Worker settings return validated configuration data. Keep
native preparation and managed registration in the composition root; do not add
a settings convenience method that exposes immediate worker launch.
Never use native from_env/builder_from_env or raw URL parsing after this validation.
The private live-endpoint handoff canonicalizes selected host/database/TLS values,
preserves explicit empty passwords and encodes query spaces as `%20` for the
external harness's required URL API. Reject IPv6 literals at this private live
boundary; SQLx's URL consumer cannot use their brackets for TCP lookup. Direct
root native IPv6 options remain supported. Native parser tests guard the handoff.
Use one native SQLx graph and explicit application-owned transactions. Delivery
submission uses `run_atomic` and one READ COMMITTED transaction for command,
delivery and Runledger rows. Exact replay never re-enqueues. Owner/key identity,
record generation, canonical payload and immutable enqueue fields stay retained;
uncertain commit/rollback acknowledgement is reconciled by owner/key and is never
automatically retried. The delivery producer definition and handler use the same
locked type identity. The production registry contains exactly that handler.
Use native `prepare()` and
transfer the owned launch value to the adapter; do not construct a live supervisor
or reintroduce an application termination gate, settlement driver, control job,
advisory lease or reconciliation loop. Native initialization, fresh database
health and application approval are separate readiness inputs.
Application readiness approval follows handler registration; fresh dependency
health and native initialization remain independent inputs. Durable
execution witnesses belong only to isolated tests. Always await a started process
before returning a live probe failure. Native reports must retain unjoined
callbacks and prevent dependent cleanup. Close pools before consuming fixture
leases and explicitly drain deferred cleanup.
Retain the applied startup-control migration and sequence. Legacy retirement is
an offline maintenance responsibility under `batter-gi4`, using native definition
disable and cancellation after verified producer/session/transaction quiescence;
never make production startup perform it. Ordinary workspace tests require no live
database. Native option tests cross the cleared-environment child boundary;
the matrix also tests hostile parent PG* state. The ordinary configuration target
requires enabled IPv6 loopback (`::1`) for its native protocol fixture, plus
IPv4 loopback and Unix subprocess permissions. This test is not skipped when
the host or container lacks IPv6. Never mutate process globals.
Explicit live invocation fails when prerequisites are missing.

The provider effect key and canonical request are created in the submission
transaction. Load and classify that retained effect before provider admission:
terminal states perform no admission, and `RECONCILE_NEEDED` must reconcile
before generation denial or replay. Provider admission waits on the dedicated
`BATTER_PROVIDER_CAPACITY` inside the existing work deadline. Its waiter set is
bounded by Runledger's validated global handler concurrency; do not replace it
with request-style immediate rejection or another claim engine. Runledger has
already claimed the native attempt and has no attempt-neutral defer/refund.
Commit `RECONCILE_NEEDED` before polling the POST, and constrain every effect
write by its legal durable source state. Every effect mutation must use the
private live-effect transaction: lock the exact `job_queue` row, recheck the
unexpired job/run/attempt/worker lease after lock acquisition, mutate one legal
effect state, revalidate before commit after any later lock waits, and commit.
Time-dependent authority must be checked in a statement after row locking, not
only in the locking SELECT's filter. Do not duplicate lease predicates in caller-owned SQL
or hold the transaction across provider I/O. Cancellation, timeout, lost response and
process death after dispatch became possible remain uncertain until the selected
lookup protocol resolves them. Only exact structured response codes may establish
known non-dispatch or business denial; text does not. POST and GET acceptance
both require the exact canonical request echo. Only a typed reqwest connector
failure proves known non-dispatch; timeout and post-connect/body errors remain
uncertain. Recheck the authoritative
owner/record/generation before confirmation. Confirmation locks the record FOR SHARE
before locking the native job and effect rows, then evaluates generation in a
fresh statement and holds the record lock through commit. A replacement that
wins the record lock makes acceptance stale; a later replacement waits for
confirmation. Keep record waits outside the heartbeat's job-row lock and retain
post-lock/precommit lease validation. Expiry, mismatch or an accepted
stale generation becomes manual resolution. The local protocol retains keys for
24 hours; bind SQL resolution deadlines from that code-selected duration rather
than duplicating it in SQL. After lookup proves retained-window absence, renew
that deadline immediately before polling the newly authorized POST; absence does
not make the old request's clock valid for a new remote-effect boundary. No claim
extends beyond that window or to another provider.
`EXHAUSTED` remains distinct from `MANUAL_RESOLUTION`: the former records a spent
native attempt budget and can retain possible acceptance, while the latter means
automatic action is unsafe because retained truth expired, conflicted or became
stale.

The owner-scoped read boundary projects unresolved terminal jobs even when the
handler could not write after storage failure or lease loss. Both routes and
exact replay share one command loader and preserve acceptance facts; do not add
best-effort writes that require lost authority. Provider exchange methods consume
the permit, so post-response SQL cannot retain provider capacity. Keep send/body
errors distinct, and map transport failures to bounded codes without error text.
The 24-hour accepted retry lower bound is a separate application policy from
retention; never clamp it down. Cleartext provider URLs require literal loopback.
Provider non-dispatch persistence takes the retry delay and constructs the native
scheduling result itself. The durable absolute eligibility timestamp commits with
the outcome and every dispatch authorization checks it under the effect-row lock.
Admission failure has a separate narrow transition and cannot classify an
uncertain provider result. A recovered attempt may consume native retry budget
while deferring; there is no refund or replacement scheduler. A response never
successfully persisted can still be lost on process death; no local type removes
that external acknowledgement gap.

Real production-child probes bind `127.0.0.1:0` in the child and consume its
acknowledged listener report through an explicitly configured Unix datagram
receiver owned by the harness before child spawn. No stdout discovery protocol
or blocking writer belongs in startup. Do not reintroduce parent-selected ephemeral ports:
an address is not reserved after its listener is dropped. Settlement permits only
empty stdout/stderr and retains the existing bounded capture, panic, exit-status
and secret-disclosure checks. Socket publication acknowledges bind, not readiness.

Keep request metadata outside authority and operation lifetime. The production
root must use application-owned `http::register_in`, which alone selects
`register_http_with_connect_info_in`; only its accepted socket peer
may populate `TrustedPeer`. Ignore forwarding, trace and client request-ID
headers until a separately validated proxy policy is implemented. The bearer
credential alone selects `OwnerId`, replacing any prior extension. Reuse
`operational_http`, `CorrelationId`, `request_admission` and
`render_infrastructure_failure`; do not add another ID generator, observation
layer, response-header setter or infrastructure renderer. Pass metadata,
authority and `OperationContext` explicitly. No arbitrary spawned task inherits
request metadata.

## Common commands

From the root, run `cargo check -p batter-example-reference-service --all-targets
--all-features --locked` and `bash scripts/test_reference_live.sh`. Follow root
pinned-toolchain, HTTP smoke and Jig verification requirements locally; MSRV
verification belongs in CI.

The live inventory requires two distinct disposable local PostgreSQL 18 clusters:
`POSTGRES_TEST_ADMIN_URL` and `POSTGRES_TEST_OBSERVER_URL`. The primary uses a
superuser, SCRAM host authentication, `max_prepared_transactions > 0`, track_counts
and autovacuum with naptime <= 5s. The
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
