# References: Native Runledger and Runlimit integration

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Native PostgreSQL Docker dependency removal: reviewed 2026-11-06

For `batter-z6nw` (external X286), Cargo initially resolved Testcontainers 0.28.0
and `astral-tokio-tar` 0.6.4. The [upstream manifest](https://github.com/testcontainers/testcontainers-rs/blob/main/testcontainers/Cargo.toml)
still makes the tar dependency mandatory; feature selection cannot remove it.
[RUSTSEC-2026-0440](https://rustsec.org/advisories/RUSTSEC-2026-0440.html)
classifies all versions as unmaintained following Astral's 3 November announcement.

The existing native fixture now calls the already-resolved Bollard 0.21.1
without its unused BuildKit features. Checked that version's
[Docker connection and negotiation implementation](https://docs.rs/bollard/0.21.1/src/bollard/docker.rs.html),
[container API source](https://docs.rs/crate/bollard/0.21.1/source/src/container.rs)
and [image API source](https://docs.rs/crate/bollard/0.21.1/source/src/image.rs).
Create uses a unique name and attempt label, IPv4 loopback with Docker-selected
host port, and the configured platform; start and inspection remain separate.
Removal uses force plus anonymous-volume deletion by immutable ID. Pull progress
is consumed and errors retained. Registry lookup uses
[docker_credential 1.4.0](https://docs.rs/docker_credential/1.4.0/docker_credential/)
for config/credential helpers or explicit `DOCKER_AUTH_CONFIG`.
Unix socket discovery retains the previous Docker Desktop locations; Bollard
selects the explicit `DOCKER_HOST` transport and TLS settings. Remote-daemon TLS,
private registries, macOS execution and platform emulation remain unverified for
this change. The fixture preserves Testcontainers' exact `DOCKER_TLS_VERIFY=1`
rule for TCP/HTTP hosts: Bollard otherwise treats any set value, including `0`,
as enabling TLS. A subprocess with `DOCKER_TLS_VERIFY=0` verifies plaintext
negotiation against a local HTTP fixture, without changing parent environment.
The historical Testcontainers diagnosis below explains the retained
mapping and failed-start contracts; its implementation is no longer selected.

## Runledger claim-write batching (2026-10-14)

The workspace uses SQLx 0.9.0; checked-query metadata was generated with SQLx
CLI 0.9.0 against PostgreSQL 18.6. PostgreSQL 18 documents
[`INSERT ... SELECT`](https://www.postgresql.org/docs/18/sql-insert.html) as
inserting the rows produced by its query, and
[row locks](https://www.postgresql.org/docs/18/explicit-locking.html#LOCKING-ROWS)
as retained through transaction end. Claim selection and leasing already lock
the exact queue rows before the batch inserts read their run, attempt and stage.
The workflow update retains its `ENQUEUED`/`RUNNING` predicate. This is an internal
statement-shape change; it adds no public API or remote-effect guarantee.

## Runledger online migration failure handling (2026-10-25)

Checked against SQLx 0.9.0 and PostgreSQL 18.6. PostgreSQL 18's
[connection settings](https://www.postgresql.org/docs/18/runtime-config-connection.html#GUC-CLIENT-CONNECTION-CHECK-INTERVAL)
document that `client_connection_check_interval = 0` disables in-query socket
checks. A positive interval checks while queries run on Linux, macOS and other
supported servers; network failures may still require keepalive detection.
SQLx 0.9.0's pool `close_on_drop` schedules connection closure, and its PostgreSQL
[connection implementation](https://docs.rs/sqlx-postgres/0.9.0/src/sqlx_postgres/connection/mod.rs.html)
sends Terminate and shuts down the stream, rather than a separate CancelRequest.
Runledger enables a bounded check interval only during concurrent builds; this
is not proof of remote termination across network failure or runtime death.

PostgreSQL's [progress view implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/pgstatfuncs.c)
returns NULL progress details for another role's backend without statistics
privilege. The restricted-role regression verifies classification with hidden
relid, index_relid and phase fields. The implementation still exposes PID and
database identity. Runledger correlates those fields with
[`pg_locks`](https://www.postgresql.org/docs/18/view-pg-locks.html) relation,
database, PID, granted status and mode. PostgreSQL's
[table-lock modes](https://www.postgresql.org/docs/18/explicit-locking.html#LOCKING-TABLES)
include `ShareUpdateExclusiveLock` for concurrent index creation and `ShareLock`
for ordinary index creation. This also covers manual builders without the
migration advisory lock. A hidden builder holding either on the target
table conservatively defers repair, including another index on that table;
unrelated tables and databases do not qualify. The combined restricted-owner,
manual-builder and unrelated-table regressions are tracked in `batter-fnj8.5`.
The `ShareLock` half is exercised separately (`batter-fnj8.16`): another
session's uncommitted relation under the builder's index name keeps an
ordinary `CREATE INDEX` waiting in its catalog insert, after
[`DefineIndex`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/indexcmds.c)
has started its progress report, with its `ShareLock` on `job_queue` granted.
On PostgreSQL 18.6 the restricted owner sees that progress row only with NULL
details. Removing `ShareLock` from the correlation made the regression report
`Invalid` instead of `BuildInProgress`.
The independent INVALID-index regression in `online_failures.rs` holds SQLx's
migration lock after the interrupted builder exits and confirms there are no
CREATE progress rows. Verification reports `BuildInProgress` while the lock is
held and `Invalid` after release, without changing catalog identity or history.
This isolates the advisory-lock fallback from progress correlation. It covers
the classification needed while a migration-lock holder has no CREATE progress,
such as a protected runner that holds the lock while it inspects and
transactionally removes an INVALID index before the replacement build, or a
cooperating manual
[`DROP INDEX CONCURRENTLY`](https://www.postgresql.org/docs/18/sql-dropindex.html)
waiting for conflicting transactions; the
[CREATE progress view](https://www.postgresql.org/docs/18/progress-reporting.html#CREATE-INDEX-PROGRESS-REPORTING)
reports CREATE/REINDEX, not DROP. Protected repair itself removes the INVALID
index with an ordinary transactional `DROP INDEX` under its table lock. The
regression isolates the fallback state; it executes neither window.
Execution evidence belongs to `batter-fnj8.6`.

PostgreSQL's [`pg_get_indexdef_worker`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/ruleutils.c)
uses syscache and can return NULL after an index disappears even when direct
catalog reads retain its row in an older repeatable-read snapshot.
[`index_drop`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/index.c)
uses two waiter phases for concurrent removal;
[`WaitForLockersMultiple`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/storage/lmgr/lmgr.c)
collects existing holders and does not wait for later arrivals. Thus a verifier
can acquire its table locks after the final waiter collection but before DROP
commits. The shared classifier treats an expected-table index with a vanished
definition as transient while the migration lock is held and missing otherwise,
while retaining non-index and foreign-table refusal. `online_snapshot.rs` drives
the public verifier across that actual DROP commit, using ephemeral event-trigger
and row-security gates, and covers both a cooperating manual drop that holds the
migration lock and lock-free settlement.
Both tests failed with `DefinitionMismatch` before the classifier correction;
executed validation belongs to `batter-fnj8.7`.

The [`REINDEX` contract](https://www.postgresql.org/docs/18/sql-reindex.html#SQL-REINDEX-CONCURRENTLY)
exchanges the old and new index names and switches `indisvalid` (step 4) before
waiting for running queries that might reference the old index (step 5).
[`WaitForOlderSnapshots`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/indexcmds.c)
waits only for snapshots listed when that wait starts. A verifier snapshot taken
after the list, but before the swap commits, therefore retains the original
index as valid under the declared name while `pg_get_indexdef` already renders
the renamed original. Isolated PostgreSQL 18 probes on 2026-10-28 showed both
halves: inside a repeatable-read snapshot a committed rename left `pg_class`
unchanged while `pg_get_indexdef` returned the `_ccold` header, and a reindex
held back by a reader stayed in `waiting for readers before marking dead` after
renaming its original to `_ccold`. `to_regclass` resolved the declared name
through the current catalog in the same probe, so the shared classifier compares
that name's current owner with the snapshot's OID and routes a moved name like a
vanished definition. `online_snapshot.rs` drives a real reindex swap inside the
public verifier's snapshot; it failed with `DefinitionMismatch` before the
correction. Executed validation belongs to `batter-fnj8.15`.

PostgreSQL 18.6's
[`ReindexRelationConcurrently`](https://github.com/postgres/postgres/blob/REL_18_6/src/backend/commands/indexcmds.c)
changes `PROGRESS_CREATEIDX_INDEX_OID` from the original to its replacement
during build and validation, while retaining session-level
`ShareUpdateExclusiveLock` locks on both indexes. The
[`REINDEX` contract](https://www.postgresql.org/docs/18/sql-reindex.html)
permits `REINDEX INDEX CONCURRENTLY` on an INVALID index. Runledger correlates
visible concurrent-reindex progress with the original index's granted lock,
scoped to the builder PID and database; another index on the same table does
not qualify. Two isolated PostgreSQL 18.6 probes first reproduced the missed
classification and competing DROP/rebuild. `online_reindex.rs` then exercises
the three public entry points after progress names the replacement, retaining
identity/history and foreign/unrelated-index controls. The public verifier test
failed before the correction. The separate preparation cancellation regression
retains native SQLSTATE `57014` without claiming an acknowledged migration;
[`pg_cancel_backend`](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADMIN-SIGNAL)
cancels the current statement rather than terminating the session. Executed
validation belongs to `batter-fnj8.8`.

[CREATE INDEX storage parameters](https://www.postgresql.org/docs/18/sql-createindex.html#SQL-CREATEINDEX-STORAGE-PARAMETERS)
define B-tree fillfactor's default as 90. Explicit options appear in the catalog
rendering and intentionally fail Runledger's strict definition match;
[ALTER INDEX RESET](https://www.postgresql.org/docs/18/sql-alterindex.html)
removes the explicit option. The live test verifies that this restores acceptance.

SQLx 0.9.0's [PostgreSQL executor](https://docs.rs/sqlx-postgres/0.9.0/src/sqlx_postgres/connection/executor.rs.html)
and stream await response I/O without a per-query client deadline. The locally
resolved [Tokio 1.53.1 timeout](https://docs.rs/tokio/1.53.1/src/tokio/time/timeout.rs.html)
polls its inner future before checking expiry, so an absolute poll deadline still needs a post-response admission
check. Runledger bounds local polling, without inferring backend termination.
The fault relay follows PostgreSQL 18's
[message formats](https://www.postgresql.org/docs/18/protocol-message-formats.html):
startup is untagged; Parse and Query contain SQL; tagged frames carry their own
length. It withholds real DataRow delivery or closes at restoration, while the
fixture database executes the SQL.

PostgreSQL's [SET privilege](https://www.postgresql.org/docs/18/ddl-priv.html)
applies to parameters normally restricted to superusers. The
[PostgreSQL 18 parameter declarations](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/misc/guc_tables.c)
mark the three lifted timeouts and `client_connection_check_interval` as
`PGC_USERSET`; revoking SET is therefore not a restoration-failure injection.
The regression instead disconnects the real SQLx session at restoration.

The PostgreSQL 18 [index progress view](https://www.postgresql.org/docs/18/progress-reporting.html#CREATE-INDEX-PROGRESS-REPORTING)
exposes database and index identities separately. Its
[backend implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/pgstatfuncs.c)
iterates all backends, filtering by command and statistics visibility rather
than current database. Runledger therefore correlates both `datid` and
`index_relid`. The live regression clones an owned disposable database using
[CREATE DATABASE TEMPLATE](https://www.postgresql.org/docs/18/sql-createdatabase.html),
which requires closing the template's connections. PostgreSQL's
[reindex implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/index.c)
publishes the index OID before obtaining its exclusive index lock; a held
reloption change lets the test keep that remote progress observable without
blocking local repair. The role test uses the function
[EXECUTE privilege](https://www.postgresql.org/docs/18/ddl-priv.html), separately
revoking unlock while preserving lock acquisition. These checks target native
PostgreSQL 18; execution results are recorded in `batter-fnj8.4`.

PostgreSQL 18's [`quote_all_identifiers` setting](https://www.postgresql.org/docs/18/runtime-config-compatible.html#GUC-QUOTE-ALL-IDENTIFIERS)
forces quoting when the server renders SQL. The PostgreSQL 18
[`pg_get_indexdef_worker` and `quote_identifier` implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/ruleutils.c)
applies it to index, table, access-method, column and type identifiers. Runledger
declares both canonical renderings and compares the catalog text unchanged;
only the trusted expected type reference may gain schema qualification. This
preserves literals and avoids mutating session settings during inspection.
