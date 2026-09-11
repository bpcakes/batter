# Reference compatibility probes

Unpublished, Unix-only application package for `batter-4t6`. Its native SQLx
pool/connection/transaction seams compile against pinned Runledger Git sources.
The [compatibility manifest](../../docs/reference-compatibility.md) records exact
versions, API contracts, executed evidence and limits.

Run offline compilation and the native seam doctest:

```sh
cargo check -p batter-example-reference-service --all-targets --all-features --locked
cargo test -p batter-example-reference-service --doc --locked
```

For live probes, select **two dedicated disposable local PostgreSQL 18 servers**.
The primary requires superuser authority, SCRAM host authentication and autovacuum
with `track_counts` enabled and `autovacuum_naptime <= 5s`
(use `postgres -c autovacuum_naptime=1s`). The
secondary must be a different cluster for the wrong-server observation control
and permit `pg_control_system()`; preflight rejects equal cluster identities.
PostgreSQL 18.4 permits this by default; if access was revoked, grant
`EXECUTE ON FUNCTION pg_catalog.pg_control_system()` to the secondary login in
the selected administrative database.
Then run from the workspace root:

```sh
POSTGRES_TEST_ADMIN_URL='postgres://postgres:fixture@127.0.0.1:5432/postgres?sslmode=disable' \
POSTGRES_TEST_OBSERVER_URL='postgres://postgres:fixture@127.0.0.1:5433/postgres?sslmode=disable' \
  bash scripts/test_reference_live.sh
```

The external harness owns four kinds of lease cleanup, and each application pool
closes before lease disposal. The runner checks prerequisites, requires all thirty-seven
named ignored cases to exist and run, and uses the existing bounded Unix process
owner. Ordinary workspace tests report these cases as ignored and require no
database. The failure injection locks a shared system catalog, so keep other
workloads off the endpoint; the runner executes cases serially. A failed or watchdog-terminated run does not establish cleanup.

The migration fixtures and witness job are minimal probes. They do not implement
the later business command, durable provider, HTTP host. Reusable native pool/lease ownership comes from the optional
`batter-sqlx/test-support` feature; template isolation and lock-operation probes
exercise it without importing application policy into the adapter.

The minimal fixture installs a separately budgeted session observer before
acquisition. Bounded observation failure retains the database until explicit retry;
all attempt errors remain in its final report. Native detach and adapter retirement
controls demonstrate why pool close alone is insufficient. Separate tests retain
body, handled pool, consuming-cleanup and deferred-drain errors, and distinguish
live waiter loss from actual runtime destruction and native lease Drop.
