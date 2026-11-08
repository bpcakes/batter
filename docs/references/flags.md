# References: Typed flags control plane

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support the flags packages of
[ADR-016](../adr/016-typed-flags-control-plane.md). Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Flags control plane, 2026-11-08

`batter-jkw6.26` records the external semantics the flags packages of
[ADR-016](../adr/016-typed-flags-control-plane.md) rely on. The pages below were
fetched on 2026-11-08 unless marked otherwise; package facts were read from the
sources Cargo.lock resolves.

PostgreSQL 18 notifications
([`NOTIFY`](https://www.postgresql.org/docs/18/sql-notify.html),
[`LISTEN`](https://www.postgresql.org/docs/18/sql-listen.html),
[hot standby](https://www.postgresql.org/docs/18/hot-standby.html)): a
transaction's notifications are delivered only if it commits and only between a
listener's transactions; identical channel and payload within one transaction
are delivered once; the payload must be shorter than 8000 bytes in the default
configuration; the queue is 8 GB in a standard installation, and when it fills
"transactions calling `NOTIFY` will fail at commit". `LISTEN` takes effect at
commit, so a listener commits `LISTEN`, then reads state in a new transaction,
then relies on notifications, and the first ones may repeat what it read. Hot
standby refuses `LISTEN` and `NOTIFY` and allows `UNLISTEN`. The pages do not
call notifications durable; Batter treats a notification only as a wake, and
the poll remains the backstop. Recorded from the plan's research of 2026-11-04
and not re-fetched here: the full-queue failure is SQLSTATE 54000 in `async.c`
at `REL_18_STABLE`, and PgBouncer's transaction pooling cannot hold a `LISTEN`.

SQLx 0.9.0 (`sqlx-postgres/src/listener.rs`): `PgListener::connect_with(pool)`
holds one pool connection (`:60-77`); a dead connection is reconnected only for
`ConnectionAborted`, `UnexpectedEof`, `TimedOut` and `BrokenPipe`, and the
receive returns `Ok(None)` for the lost interval (`:281-308`); `Drop` spawns a
detached `UNLISTEN *` task (`:361-377`). The flags listener therefore rebuilds
and resyncs on `Ok(None)` or any error, and its drain unlistens before the
drop. `PgConnectOptions::new_without_pgpass` reads the libpq environment
defaults (`PGPORT`, `PGHOSTADDR`, `PGHOST`, `PGUSER` and others,
`options/mod.rs:56-68`), and URL parsing starts from it (`options/parse.rs:9-10`),
so options a root builds from a URL still carry those defaults.

`rustls-native-certs` 0.8.4 reads `SSL_CERT_FILE` and `SSL_CERT_DIR`
(`src/lib.rs:200-205`, `:363-364`). This and the SQLx defaults are the
dependency environment reads ADR-016 lists; no other dependency was audited.

File replacement: POSIX.1-2017
[`rename`](https://pubs.opengroup.org/onlinepubs/9699919799/functions/rename.html)
(Issue 7, 2018 edition) keeps "a link named *new*" visible "throughout the
renaming operation" when *new* exists, its rationale requires the action to be
atomic, and a rename that fails other than with `EIO` leaves *new* unaffected;
`EXDEV` covers links between file systems. Linux man-pages 6.19
[`fsync(2)`](https://man7.org/linux/man-pages/man2/fsync.2.html): fsync of a
file "does not necessarily ensure that the entry in the directory containing
the file has also reached disk"; that needs "an explicit fsync() on a file
descriptor for the directory". The last-good file therefore writes a
temporary file under the directory lock, syncs it, renames it over the final
name within the one directory and syncs the directory. Recorded from the plan
and not re-fetched: Rust 1.98.1 `File::sync_all` issues `F_FULLFSYNC` on Apple
targets and `fsync` elsewhere. On macOS 27.2 arm64 (APFS) a directory's
`File::sync_all` returned `Ok` in a probe on 2026-11-08 (`batter-jkw6.28`);
power-loss durability and `flock` on tmpfs or overlayfs remain unverified.

OpenTelemetry semantic conventions v1.45.0
([`feature-flags-events.md`](https://github.com/open-telemetry/semantic-conventions/blob/v1.45.0/docs/feature-flags/feature-flags-events.md)):
the evaluation event is named `feature_flag.evaluation` and is a release
candidate in a document marked Development; `feature_flag.key` is required,
`feature_flag.result.variant` is conditionally required when a variant exists,
`feature_flag.result.value` is opt-in then, and the well-known
`feature_flag.result.reason` values are `cached`, `default`, `disabled`,
`error`, `split`, `stale`, `static`, `targeting_match` and `unknown`. Batter's
`Reason` has six of them, `static`, `default`, `disabled`, `cached`, `stale`
and `error`; it has no `targeting_match` or `split`, since per-subject
rollouts left the epic, and no `unknown`.

Recorded from the plan's research and not re-fetched: PostgreSQL 18
documents `nextval` as never rolled back, so a sequence can leave gaps and, with
a cache above one, hand out values out of order across sessions; the store
instead bumps one counter row under its row lock inside every writing
transaction, which the concurrent-write tests of `batter-flags-postgres`
exercise. On Unix `std::env::set_var` is sound only in a single-threaded
program while `cargo test` runs tests as threads of one process, which is why
flags tests inject a `SettingsSource` or clear a child process's environment.

## Flags store primary-source recheck (T-12, 2026-11-06)

The installed SQLx 0.9.0 `Migrator::run_direct` calls `Migrate::lock` before
schema/history work; PostgreSQL's implementation executes `pg_advisory_lock`
outside migration transactions and starts a transaction per ordinary migration.
PostgreSQL 18 [SET](https://www.postgresql.org/docs/18/sql-set.html) limits LOCAL
settings to the current transaction and says they have no effect outside one.
The [client timeout settings](https://www.postgresql.org/docs/18/runtime-config-client.html)
distinguish statement, lock-wait and idle-in-transaction limits. The new native
bounded migration path uses session settings for these phases and retires the
session so they cannot leak into pool reuse. This is based on installed source
and the versioned primary docs, rechecked during the T-12 review repair.

Installed SQLx 0.9.0 `sqlx-core/src/migrate/migrator.rs` provides
`dangerous_set_table_name` and `create_schema`; the flags migrator fixes its own
ledger at `flags._sqlx_migrations`. The workspace's `batter-sqlx` atomic owner
retires on completion and wraps both validation and COMMIT failures in
`CommitUnconfirmed`; its read-only owner begins REPEATABLE READ READ ONLY and
retires after rollback. T-12 composes these installed APIs rather than the stale
plan's plain-SQLx owner.

PostgreSQL 18 [NOTIFY](https://www.postgresql.org/docs/18/sql-notify.html)
documents transactional delivery, commit ordering, bounded payloads and commit
failure when the notification queue fills. The version-counter/poller tests
exercise the store's inference from lock retention through commit.
[CREATE TRIGGER](https://www.postgresql.org/docs/18/sql-createtrigger.html)
defines deferred constraint triggers and TRUNCATE statement triggers.
[CREATE FUNCTION](https://www.postgresql.org/docs/18/sql-createfunction.html)
documents SECURITY DEFINER search-path protection and revoking PUBLIC execution.
The migration fixes the definer search path, schema-qualifies relation lookup,
and revokes PUBLIC execute in the same transaction. Live tests use a fresh
non-superuser writer login, not owner privilege, for enforcement claims.

