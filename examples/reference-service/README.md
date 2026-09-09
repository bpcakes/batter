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

For live probes, select a **disposable local PostgreSQL 18 server** whose role can
create and drop databases, then run from the workspace root:

```sh
POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:5432/postgres?sslmode=disable' \
  bash scripts/test_reference_live.sh
```

The external harness owns four kinds of lease cleanup, and each application pool
closes before lease disposal. The runner checks prerequisites, requires all four
named ignored cases to exist and run, and uses the existing bounded Unix process
owner. Ordinary workspace tests report these cases as ignored and require no
database. A failed or watchdog-terminated run does not establish cleanup.

The migration fixtures and witness job are minimal probes. They do not implement
the later business command, durable provider, HTTP host or reusable lease adapter.
