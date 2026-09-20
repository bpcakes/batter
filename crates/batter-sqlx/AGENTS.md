# PostgreSQL operation adapter guide

## Purpose

Optional SQLx 0.9 PostgreSQL connection disposition; native operations and
transactions remain application-owned. Follow the root Unix-only policy.

## Key entrypoints

- `src/lib.rs`: default-retiring lease, consuming native migration execution, bounded probe, pool cleanup registration.
- `src/session.rs`: opaque session/transaction execution capabilities and consuming completion.
- `src/failure.rs`: redacted native causes and conservative classifications.
- `src/verification.rs` and `src/verification/`: owned read-only migration and
  serving-authority inspection; validated policy construction and canonical
  limits are in the README.
- `tests/postgres_live.rs`: explicitly selected external-database contracts.

## Edit here for X

Keep connection ownership mechanics here and migration, replay, SQL contents,
transaction completion and provisioning policy in applications. Generic read-only
verification owns its lease, snapshot and disposition. Non-empty protected plans can
check exact SQLx 0.9 ledger shape/history, scoped definer `search_path`, and
coarse reachable current-database ownership; migration selection, routine
protocols and grant-manifest contents remain downstream. The pure
exact-role compiler and inert grant renderer live here; they must not acquire a
connection, execute SQL or become a provisioner. The runnable
consumer is `examples/postgres-lifecycle`.

## Invariants

Dropping an unsuccessful lease detaches and drops the client connection.
Ordinary pool return requires a private proof created only after application
success, zero unacknowledged typed transactions and a successful same-connection
`ROLLBACK` round trip. This synchronizes raw open or failed transactions that the
native executor can create but does not reset arbitrary session state. Failure or
cancellation before that proof retires the connection. Local capacity release and
Pool::close do not acknowledge server-session termination, rollback or remote
cancellation; detached sessions can exceed max_connections.
Public lease closures never receive a native connection or transaction and the
opaque wrappers never implement native `DerefMut`/`AsMut`; disposition must apply
to the same physical connection that was acquired.
Never log native error contents automatically. Core and generic support remain
independent of SQLx. Never provision PostgreSQL in this package. Verifier discovery preserves selected
object identities separately from effective ACL sources. Evaluation shares one
cooperative work/report budget; exhaustion cannot produce a partial pass.
Classify selected namespace semantics before ledger or authority inspection;
temporary namespaces are unsupported, not ordinary ACL objects. Required and
excess checks share captured catalog state: native privilege functions can see
newer grants through catalog caches. Use them as stable-fixture test references.
PUBLIC relation/column default precedence belongs to the shared policy index.
Relation and sequence targets share PostgreSQL's `pg_class` namespace: reject
cross-kind identities in pure authority compilation and when a migration ledger
is composed with authority, regardless of plan construction order. A valid
declaration that meets the opposite catalog kind is database drift: retain its
ordinary missing-object or missing-privilege finding and apply discovery defaults to the observed kind through
the separate catalog-evaluation policy; do not merge both kinds into one policy.
Construct that private evaluation policy incrementally inside the shared
checkpointed traversal. Do not route catalog-generated defaults back through the
external draft compiler or add a synchronous whole-policy sort/index phase.
Ledger inheritance is unsupported; standalone ledger reads use ONLY so planner
inheritance refresh cannot mix relation membership with older snapshot rows.
High-level SQLx ledgers additionally require the exact six-column and primary-key
shape. Schema-setting checks remain independent of serving-role authority.
New supported semantics require primary PostgreSQL18 evidence and native tests.

## Common commands

Run `cargo test -p batter-sqlx --features test-support --locked` for offline contracts and
`bash scripts/test_sqlx_live.sh` with `DATABASE_URL` plus a known-good password
endpoint in `BATTER_SQLX_AUTH_ACCEPT_URL` for the ignored live cases. The explicit
invocation must fail when prerequisites are missing. Also run the root
two-toolchain verification, HTTP smoke profiles and required Jig gates.

## Optional fixture support

Read [the canonical fixture contract](README.md#optional-fixture-support) before
changing `src/test_support.rs`, `src/test_support/` or fixture tests. Keep that
contract in the README rather than duplicating it here. Session observation and
retry ownership live in `src/test_support/sessions.rs`; retained outcomes and
redacted summaries live in `src/test_support/report.rs`. Reference composition
and live failure controls belong to `examples/reference-service`, not the generic
`batter-test-support` crate.
