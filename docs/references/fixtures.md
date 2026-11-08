# References: Database fixtures and live probes

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Fixture login pools (2026-10-31)

Owning Bead: `batter-7r3.8`. Checked the locked SQLx 0.9.0 registry sources,
postgres-test-harness revision `3d525e6fc5745ce2e2437c7997de5cccdecff4ac` and
PostgreSQL 18.6.

- SQLx [`PoolOptions::connect_lazy`](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/pool/options.rs)
  is `connect_lazy_with(url.parse()?)`, so parsing the lease URL once and cloning
  the result preserves plain lease-login pools. Lazy construction
  ([`PoolInner::new_arc`](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/pool/inner.rs))
  opens no connection unless `min_connections > 0` with both idle timeout and
  maximum lifetime disabled; then a spawned task connects at once with a
  300-second internal deadline. Construction panics on the child-pool
  assertions (a larger `max_connections` or different `fair` than its parent),
  on `max_connections == 0` (crossbeam-queue's `ArrayQueue::new` asserts a
  non-zero capacity) and outside a Tokio context (maintenance spawning).
  `PlanDeclaration::finish` rejects zero maxima and fixtures construct inside
  Tokio, so only a caller-supplied `PoolOptions::parent` can reach a panic. `acquire` runs connect and `after_connect` inline inside the
  caller's future under the acquire timeout, so cancelling the waiter drops an
  in-progress connection. A failing `after_connect` closes the connection and
  retries until `PoolTimedOut`; a non-transient connect error such as `28P01`
  returns at once, while transient ones such as `53300` are retried.
  `Pool::close` waits for every permit.
- SQLx [PostgreSQL URL parsing](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-postgres/src/options/parse.rs)
  starts from `PgConnectOptions::new_without_pgpass` (reading `PG*` variables),
  applies the path, then lets `host`, `dbname` and other query keys override it,
  and applies a passfile only for the parsed username when no password is set.
  Setters cannot clear a password or socket, and the
  [options type](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-postgres/src/options/mod.rs)
  derives Debug over the password. `Pool::set_connect_options` replaces the
  options used for later connections. `Error::Configuration` is the native
  configuration variant in [sqlx-core error.rs](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/error.rs).
- The harness [admin URL](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs)
  builds each lease URL by replacing only the path, keeping the query, creates
  databases with `CREATE DATABASE ... TEMPLATE` and drops them with
  `DROP DATABASE ... WITH (FORCE)`.
- PostgreSQL 18 [session information functions](https://www.postgresql.org/docs/18/functions-info.html):
  `session_user` is the role that started the session (a superuser can change
  it with `SET SESSION AUTHORIZATION`), while `system_user` is the authentication
  method and identity presented before a role was assigned, NULL under trust.
  Live cases therefore require `system_user` of `scram-sha-256:<login>`.
- PostgreSQL 18 [schema privileges](https://www.postgresql.org/docs/18/ddl-schemas.html#DDL-SCHEMAS-PRIV):
  since PostgreSQL 15, `public` is owned by `pg_database_owner` and PUBLIC has
  USAGE but not CREATE. [Database privileges](https://www.postgresql.org/docs/18/ddl-priv.html)
  give PUBLIC CONNECT and TEMPORARY on new databases, so a restricted login can
  connect to a fresh lease and its profile can use `public` before any grant.
- PostgreSQL 18 [`DROP DATABASE`](https://www.postgresql.org/docs/18/sql-dropdatabase.html)
  `FORCE` fails when the current user may not terminate the other connections,
  with the same permission rules as
  [`pg_terminate_backend`](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADMIN-SIGNAL).
  [`DROP ROLE`](https://www.postgresql.org/docs/18/sql-droprole.html) fails while
  the role has privileges on objects in any database, so grants copied from a
  retained template block removal; grants inside a dropped lease do not.
- PostgreSQL 18 [error codes](https://www.postgresql.org/docs/18/errcodes-appendix.html):
  `28P01` invalid_password and `42501` insufficient_privilege, both observed on
  PostgreSQL 18.6 by the live cases.

## Native live URL compatibility, 2026-09-10

The same pinned SQLx URL parser retains IPv6 authority brackets. Its PostgreSQL
connection stream passes that host to
[`sqlx-core` 0.9.0 TCP connection](https://docs.rs/crate/sqlx-core/0.9.0/source/src/net/socket/mod.rs),
which uses Tokio's `(host, port)` address form. A native reproduction confirmed
that `[::1]` fails lookup there while bare `::1` resolves. The private live
fixture policy now rejects IPv6 literals before connection work; the root's
native-options IPv6 wire test remains supported. This avoids applying a private
live-runner workaround throughout the generic adapter's native URL consumers.

Related comparisons of the locked native parsers found two further scalar
differences: SQLx accepts case-insensitive TLS modes, while tokio-postgres 0.7.18
requires `disable` exactly; SQLx trims all leading path slashes before decoding,
while the harness client removes one. The live handoff serializes the validated
database as one encoded path component and the selected disabled TLS mode in
canonical spelling. Credential/query values retain their selected bytes.

SQLx's URL parser also calls `apply_pgpass` after parsing. An absent URL password
therefore differs from the root's explicit empty Setup password. A regression
using the existing private fake passfile failed against the earlier handoff.
The handoff now supplies an explicit empty query password when necessary; the
same real native parser then preserves the selected empty value. These checks
do not read a real credential file or establish live server authentication.
