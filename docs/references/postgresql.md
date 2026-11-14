# References: PostgreSQL execution, managed objects and verification

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Skipped pool close and server-session termination (2026-11-11)

For `batter-9wig.5`, checked what remains on the server when the `pool_in`
finalizer is skipped instead of awaited:

- [PostgreSQL 18 protocol flow, 54.2.9 Termination](https://www.postgresql.org/docs/18/protocol-flow.html#PROTOCOL-FLOW-TERMINATION):
  graceful termination is a Terminate message followed by connection close. If
  either side "sees an unexpected closure of the connection, it should clean up
  and terminate", and "for either normal or abnormal termination, any open
  transaction is rolled back, not committed". A statement already running
  outside a transaction block may still finish and commit before the backend
  notices the disconnection.
- Resolved SQLx 0.9.0: `PgConnection::close` (`sqlx-postgres`
  `src/connection/mod.rs`) sends Terminate and shuts the stream down, and is the
  only sender of Terminate. No connection type implements `Drop` to send it.
  `PoolInner::drop` (`sqlx-core` `src/pool/inner.rs`) only marks the pool closed
  and returns permits to a parent pool. A dropped pool handle therefore closes
  nothing while another handle exists; dropping the last one drops the idle
  connections, which closes their sockets without Terminate.
- While a handle survives, SQLx keeps maintaining the pool: its reaper closes an
  idle connection past `idle_timeout` or one past `max_lifetime`
  (`sqlx-core` `src/pool/inner.rs:548-554`; both enabled by default,
  `src/pool/options.rs:161-162`) through the native close, which sends
  Terminate (`sqlx-postgres` `src/connection/mod.rs:162-169`).

So a skipped close does not leave sessions the server cannot end: each backend
exits when its socket closes, at the latest when the process does, and rolls
back an open transaction. What is lost is the Terminate message for
connections dropped rather than closed by a surviving pool, the wait for
checked-out connections to return, and any bound on how long handles held by
surviving tasks keep sessions open. This is source reading; no live server
observed `pg_stat_activity` after a skipped close.

## PostgreSQL 18 GCRA response timing, 2026-11-08

For `batter-1zoj`, checked PostgreSQL 18 semantics and the resolved SQLx 0.9.0
integration before changing delay metadata:

- [Current date/time](https://www.postgresql.org/docs/18/functions-datetime.html#FUNCTIONS-DATETIME-CURRENT):
  `clock_timestamp()` observes the actual wall clock, including changes within a
  statement. It supplies both the evaluation sample and response sample; the
  committed GCRA watermark remains a separate clamp on quota evaluation.
- [Data-modifying WITH](https://www.postgresql.org/docs/18/queries-with.html#QUERIES-WITH-MODIFYING):
  `RETURNING` supplies the rows consumed by the outer query. The final watermark
  update returns one row per shard; an outer aggregate consumes all rows before
  sampling the response clock. Denials sample in their final counter read.

The implementation subtracts only a nonnegative difference of these samples.
It does not measure later commit or transport latency.

## PostgreSQL 18 guarded index removal, 2026-10-27

For `batter-fnj8.10`, PostgreSQL 18.6 SQL-boundary and public-API probes establish
that a completed name swap between inspection and DROP can delete a foreign
UNIQUE index. The protected repair changes the execution boundary, not the matcher.

- [DROP INDEX](https://www.postgresql.org/docs/18/sql-dropindex.html): concurrent
  DROP takes a name and cannot run inside an explicit transaction; ordinary DROP
  can, but requires an exclusive parent-table lock.
- [ALTER INDEX](https://www.postgresql.org/docs/18/sql-alterindex.html): index rename
  acquires SHARE UPDATE EXCLUSIVE; source `RenameRelation` retains it until transaction
  end. Two uncommitted renames restore the public name while retaining that lock.
- [LOCK](https://www.postgresql.org/docs/18/sql-lock.html): NOWAIT refuses contention,
  and transaction locks remain until completion. The repair takes the parent lock
  before its final identity/definition inspection and DROP.
- [Explicit locking](https://www.postgresql.org/docs/18/explicit-locking.html)
  (rechecked 2026-10-27 for `batter-fnj8.11`): `ACCESS EXCLUSIVE` conflicts with
  every table lock mode, including `ACCESS SHARE` from plain reads, `ROW SHARE`
  from `FOR UPDATE SKIP LOCKED` claims and `SHARE UPDATE EXCLUSIVE` from
  (auto)vacuum, so one `NOWAIT` attempt is refused whenever any of them is held.
  A failed `LOCK` or `lock_timeout` raises SQLSTATE `55P03` (`lock_not_available`,
  [error codes](https://www.postgresql.org/docs/18/errcodes-appendix.html)).
  [DROP INDEX](https://www.postgresql.org/docs/18/sql-dropindex.html)
  `CONCURRENTLY` drops an index without locking out concurrent selects, inserts,
  updates and deletes on its table, which is why it is the operator recovery
  for a table that is never idle.
- [PostgreSQL 18 source](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/tablecmds.c):
  `RemoveRelations` resolves and locks the current name occupant, not a client's
  previously inspected OID. `index_drop` uses internal session locks across its
  own concurrent-drop commits; those do not protect a preceding client query.

- [Client connection defaults](https://www.postgresql.org/docs/18/runtime-config-client.html)
  (rechecked 2026-10-28 for `batter-fnj8.13`): `statement_timeout` aborts a
  statement, while `transaction_timeout` and `idle_in_transaction_session_timeout`
  each terminate the session. A session-level advisory lock is held until it is
  released or the session ends
  ([advisory locks](https://www.postgresql.org/docs/18/explicit-locking.html#ADVISORY-LOCKS)),
  so a terminated metadata session also drops the SQLx migration lock. Their
  SQLSTATEs are `25P04` and `25P03`; `ALTER INDEX` on an absent name raises
  `42P01` (`undefined_table`), which repair reports as `ChangedDuringRepair`
  ([error codes](https://www.postgresql.org/docs/18/errcodes-appendix.html)).
- [Cumulative statistics](https://www.postgresql.org/docs/18/monitoring-stats.html)
  (rechecked 2026-10-28): `pg_stat_database.sessions_abandoned` counts sessions
  terminated because the client connection was lost, and `sessions_fatal` those
  terminated by fatal errors. The metadata-repair cancellation test uses them to
  tell a closed client connection from the server's idle-bound termination.

The metadata transaction uses local 100 ms lock and one-second statement,
transaction/idle timeouts; the latter two end the session when exceeded, which
`online_repair/lifecycle.rs` exercises on PostgreSQL 18.6. Long concurrent
creation runs only after commit.

- [Multiple statements in a simple Query](https://www.postgresql.org/docs/18/protocol-flow.html#PROTOCOL-FLOW-MULTI-STATEMENT)
  (rechecked 2026-10-29 for `batter-fnj8.16`): statements after an explicit
  `BEGIN` in one Query message run inside that transaction block, and an error
  aborts the block until `ROLLBACK`. Repair therefore sends its `SET LOCAL`
  bounds in the `BEGIN` message: the build session has lifted its profile
  timeouts, so a separate exchange would leave the server idle in the
  transaction without any bound until it arrived.
- [SQLx 0.9.0 `PgTransactionManager::begin`](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-postgres/src/transaction.rs)
  (rechecked 2026-10-29): `Connection::begin_with` queues the custom statement
  as one simple query and waits for `ReadyForQuery`, discarding row data. Its
  [`PgStream::recv`](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-postgres/src/connection/stream.rs)
  returns a server `ErrorResponse` before that `ReadyForQuery`, and the begin
  guard rolls back only an already counted transaction, so a failed bound
  statement leaves an aborted block SQLx does not track. Repair sends
  `ROLLBACK` itself in that case and retains both errors; outside a block it
  only warns. The next request drains the pending `ReadyForQuery` first.
  Unverified end to end: every statement in that message is static and
  instant, so no executable test can make one fail without changing the SQL
  under test, and this recovery rests on the cited sources only.

Withholding the first request after that `BEGIN`, an extended-protocol
Parse-through-Sync sequence, leaves the server idle in the transaction; the
idle or transaction bound then ends the session on PostgreSQL 18.6. A control
that sent the bounds in a separate request after `BEGIN` timed out instead.
A lost commit acknowledgement is uncertainty, not proof that DROP rolled back.
Executed outcomes and retained diagnostic probes are recorded in the owning Bead.

## PostgreSQL 18 scoped managed apply, 2026-10-26

For `batter-aouh`, rechecked the official PostgreSQL 18 commands:

- [SET CONSTRAINTS](https://www.postgresql.org/docs/18/sql-set-constraints.html):
  switching deferred checks to immediate runs outstanding checks and constraint
  triggers during that command. The new callback path does this before its final
  managed-definition/ledger inspection.
- [ROLLBACK TO SAVEPOINT](https://www.postgresql.org/docs/18/sql-rollback-to.html):
  recovery discards later work and nested savepoints; the named savepoint remains
  until released. Native XID validation is still needed to reject a replacement
  top-level transaction. Already committed effects cannot be recovered this way.

## PostgreSQL 18 managed routine and trigger semantics, 2026-10-15

For `batter-54d0` ([ADR-013](../adr/013-managed-database-objects.md)), rechecked:

* [`CREATE FUNCTION`](https://www.postgresql.org/docs/18/sql-createfunction.html):
  replacing keeps ownership and permissions and requires owning the function
  (including membership in the owning role). It cannot change the name or
  argument types (that creates another overload), the return type or `OUT`
  parameter types, or the name of an existing input parameter. `BEGIN ATOMIC`
  and `RETURN` bodies record dependencies; string bodies do not.
* [`CREATE TRIGGER`](https://www.postgresql.org/docs/18/sql-createtrigger.html):
  `OR REPLACE` replaces every other property of the trigger named on its table;
  it is not supported for constraint triggers. A trigger name cannot be
  schema-qualified; its table may be.
* [Explicit locking](https://www.postgresql.org/docs/18/explicit-locking.html):
  `CREATE TRIGGER` takes `SHARE ROW EXCLUSIVE`; `ACCESS SHARE` conflicts only
  with `ACCESS EXCLUSIVE`. Session-level advisory locks survive `ROLLBACK` and
  are held until unlocked or the session ends.
* Rechecked for `batter-54d0.1`: the same lock table shows that `SHARE` blocks
  the trigger's `SHARE ROW EXCLUSIVE` request. The
  [`pg_locks` view](https://www.postgresql.org/docs/18/view-pg-locks.html) exposes
  backend, relation, mode and grant state, allowing the concurrency tests to
  wait for an actual conflicting request. PostgreSQL 18's
  [`LOCK` contract](https://www.postgresql.org/docs/18/sql-lock.html) requires
  `LOCK TABLE` before the first `SELECT` or data-modification statement in a
  repeatable-read transaction: that first query or modification freezes its
  view of data. `LOCK TABLE` itself does not establish the snapshot. The tests
  verify the managed ledger is locked before snapshot-taking queries.
* [`check_function_bodies`](https://www.postgresql.org/docs/18/runtime-config-client.html):
  when off, routine body strings are not validated during `CREATE FUNCTION` and
  `CREATE PROCEDURE`; `pg_dump` turns it off.
* [System information functions](https://www.postgresql.org/docs/18/functions-info.html):
  `pg_get_functiondef` returns a decompiled `CREATE OR REPLACE FUNCTION` or
  `PROCEDURE` statement and `pg_get_triggerdef` a decompiled trigger command;
  `pg_current_xact_id` assigns and returns the top-level transaction id. The
  page does not say whether these read the transaction snapshot.

Executed evidence, PostgreSQL **18.6** (`postgres:18` container, Linux x86_64):
re-applying an unchanged `CREATE OR REPLACE FUNCTION` or `TRIGGER` keeps oid,
owner and ACL and sets the row's `xmin` to `pg_current_xact_id()::xid`, which
identifies rows a transaction wrote. `pg_get_functiondef` and
`pg_get_triggerdef` qualify type and function names relative to `search_path`
(`mood` versus `public.mood`, `trg_fn()` versus `public.trg_fn()`).
`CREATE OR REPLACE TRIGGER` on an unchanged trigger held `SHARE ROW EXCLUSIVE`
on its table until commit, dropping an existing trigger held
`ACCESS EXCLUSIVE`, `DROP TRIGGER IF EXISTS` for an absent trigger held no table
lock and emitted a notice, and `CREATE OR REPLACE FUNCTION` held none.
`pg_get_triggerdef` renders the same text for an enabled and a disabled
trigger, and `CREATE OR REPLACE TRIGGER` resets `tgenabled` from `D` or `R` to
`O`. Multi-argument `unnest` in `FROM` is parser syntax that cannot be
schema-qualified; `ROWS FROM (pg_catalog.unnest(...), ...)` is its qualified
form.
`DROP ... IF EXISTS` skips a missing schema, argument type or table with a
notice. Removing a parameter default is refused ("cannot remove parameter
defaults from existing function"), changing one is accepted, changing a
routine's kind is refused, and replacing a routine a trigger depends on is
allowed while dropping it is refused with SQLSTATE `2BP01`. With
`check_function_bodies` on, a SQL string body referencing a missing column
fails at creation with `42703`; with it off, creation succeeds. In a
`REPEATABLE READ` transaction, `pg_proc.prosrc` kept the snapshot's body while
`pg_get_functiondef` returned a body committed afterwards by another session,
so rendering reads current catalog state; the verifier's pre-snapshot ledger
lock exists for that reason. These probes do not cover other server versions.

## PostgreSQL 18 constraint trigger semantics, 2026-10-16

For `batter-n2s1` ([ADR-013](../adr/013-managed-database-objects.md), amended),
rechecked:

* [`CREATE TRIGGER`](https://www.postgresql.org/docs/18/sql-createtrigger.html):
  `CONSTRAINT` creates a constraint trigger, "the same as a regular trigger
  except that the timing of the trigger firing can be adjusted using
  `SET CONSTRAINTS`"; constraint triggers "must be `AFTER ROW` triggers on
  plain tables"; `FROM`, `DEFERRABLE` and `INITIALLY` "can only be specified
  for constraint triggers"; a trigger name "must be distinct from the name of
  any other trigger for the same table"; and "Currently, the `OR REPLACE`
  option is not supported for constraint triggers." For constraint triggers
  the `WHEN` condition is evaluated immediately, not deferred.
* [`DROP TRIGGER`](https://www.postgresql.org/docs/18/sql-droptrigger.html):
  the only removal path documented for a trigger; it requires owning the table
  and defaults to `RESTRICT`. The page does not state its lock level.
* [`pg_trigger`](https://www.postgresql.org/docs/18/catalog-pg-trigger.html):
  `tgconstraint` is "the `pg_constraint` entry associated with the trigger
  (zero if trigger is not for a constraint)"; `tgisinternal` is "True if
  trigger is internally generated (usually, to enforce the constraint
  identified by `tgconstraint`)"; `tgdeferrable` and `tginitdeferred` describe
  a constraint trigger's deferrability; `tgenabled` controls the
  `session_replication_role` modes in which it fires.

Executed evidence, PostgreSQL **18.6** (local `postgres:18` container, Linux
x86_64): `CREATE OR REPLACE CONSTRAINT TRIGGER` fails with
"CREATE OR REPLACE CONSTRAINT TRIGGER is not supported"; a user constraint
trigger has `tgconstraint <> 0`, `tgisinternal = false` and a `pg_constraint`
row with `contype = 't'` named after the trigger, so creating one whose name
another constraint on the table already uses fails with a duplicate key on
`pg_constraint_conrelid_contypid_conname_index`. `CREATE OR REPLACE TRIGGER`
over a constraint trigger fails with "trigger ... is a constraint trigger", and
`CREATE CONSTRAINT TRIGGER` under a plain trigger's name fails with "already
exists". `ALTER TABLE ... DROP CONSTRAINT` is refused because the trigger
requires the constraint, and `ALTER TABLE ... ALTER CONSTRAINT` is refused for
it. `DROP TRIGGER` of a constraint trigger held `ACCESS EXCLUSIVE` on its table
until commit; `CREATE CONSTRAINT TRIGGER` alone held `SHARE ROW EXCLUSIVE`; after
both in one transaction the new `pg_trigger` row's `xmin` equalled
`pg_current_xact_id()::xid`. Under the canonical rendering settings
`pg_get_triggerdef` returned `CREATE CONSTRAINT TRIGGER ct AFTER INSERT ON
probe_ct.t FROM probe_ct.r DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN
((new.id > 0)) EXECUTE FUNCTION probe_ct.f()`, and `... NOT DEFERRABLE INITIALLY
IMMEDIATE ...` for a non-deferrable one, so deferrability, `FROM` and `WHEN` are
in the rendered hash while `tgenabled` is not. `ALTER TABLE ... DISABLE
TRIGGER` sets a constraint trigger's `tgenabled` to `D`. `BEFORE` and
statement-level forms are syntax errors for `CREATE CONSTRAINT TRIGGER`. A
deferred row trigger queued by an `INSERT` and fired at commit received the
inserted row version (`NEW.note` empty) even though a later statement in the
transaction updated the row, so a deferred check that must see committed state
reads the row again. With one session holding `ACCESS SHARE` in an open
transaction, a second session's `DROP TRIGGER` of a constraint trigger waited
ungranted, and a third session's plain `SELECT` of the table then waited behind
it until its `lock_timeout` of one second cancelled it ("canceling statement
due to lock timeout"); the same `SELECT` succeeded once the drop finished. A
`DROP TRIGGER` run under `SET lock_timeout = '500ms'` behind such a reader
failed with the same error.

Rechecked on 18.6 for the follow-up after review (same day):
* **Partitions.** A constraint trigger on a partitioned table is cloned onto
  each partition (`tgparentid <> 0`, `tgisinternal = false`,
  `tgconstraint <> 0`). For a clone, `DROP TRIGGER` fails ("... requires it"),
  `CREATE OR REPLACE TRIGGER` fails ("is an internal or a child trigger") and
  `CREATE CONSTRAINT TRIGGER` fails ("already exists"). Creating a parent
  trigger whose name a partition already holds fails with "already exists".
* **Partition clone drops and locks.** Dropping the parent's trigger drops its
  clones, so a later `DROP TRIGGER` on the partition fails with "does not
  exist" (`42704`). Dropping it and creating a same-named constraint trigger on
  the partition in one transaction succeeds. `DROP TRIGGER` on the parent holds
  `ACCESS EXCLUSIVE` on the parent and every partition, and
  `CREATE CONSTRAINT TRIGGER` on the parent holds `SHARE ROW EXCLUSIVE` on all
  of them. `ALTER TABLE partition DISABLE TRIGGER` disables only the clone; the
  parent stays `O`.
* **Rename.** `ALTER TRIGGER ... RENAME` keeps the `pg_constraint` name, so
  re-creating the old name fails on
  `pg_constraint_conrelid_contypid_conname_index`. Dropping the renamed trigger
  first, in the same transaction, makes it succeed.
* **Views and foreign tables.** `CREATE CONSTRAINT TRIGGER` on a view fails
  ("is a view") and on a foreign table ("is a foreign table"). `LOCK TABLE`
  refuses a foreign table ("cannot lock relation"). `LOCK TABLE ONLY` on a view
  also takes `ACCESS EXCLUSIVE` on its base table, while `DROP TRIGGER` on a
  view locks only the view.
* **Lock scope and order.** `LOCK TABLE ONLY` on an inheritance or partitioned
  parent locks only that table, and `DROP TRIGGER` on an inheritance parent
  locks only the parent. Behind another session's `ACCESS EXCLUSIVE`,
  `DROP TRIGGER` waits first for `ACCESS SHARE`.
* **Routine kinds.** `DROP FUNCTION` refuses a procedure ("is not a function")
  and `DROP PROCEDURE` refuses a function.
* **Two-session re-read.** One session holds `ACCESS EXCLUSIVE` and replaces a
  plain trigger with a same-named constraint trigger. A second session reads
  the trigger, waits in `LOCK TABLE ONLY ... IN ACCESS EXCLUSIVE MODE`, then
  reads it again:
  * under REPEATABLE READ the second read still shows the plain trigger, while
    `DROP TRIGGER` then drops the constraint replacement;
  * under READ COMMITTED the second read shows the constraint trigger.
* **Relation replacement and lock order (library live cases).**
  * A `DROP TRIGGER` that waits for the lock of a view or foreign table follows
    the name once another session commits a same-named ordinary table, and
    removes that table's constraint trigger. Without a relation check, apply
    retired it.
  * A transaction holding `ACCESS SHARE` whose write request conflicts only
    with apply's waiting `ACCESS EXCLUSIVE` request is queued ahead of it and
    proceeds. If apply already holds `SHARE ROW EXCLUSIVE` on the table, or on
    the partitioned table of the partition it waits for, the two deadlock
    (`40P01`).
  * A write routed through a partitioned table locks it before the partition
    it reaches. An apply holding `ACCESS EXCLUSIVE` on that partition while
    waiting for the partitioned table deadlocks with it (`40P01`); locking the
    partitioned table first lets the write finish.
  * Probed: `LOCK TABLE` without `ONLY` on a partitioned table takes
    `ACCESS EXCLUSIVE` on it and every partition, including a foreign-table
    partition, as `DROP TRIGGER` on it does. A constraint trigger cannot be
    created on a partitioned table with a foreign partition ("... is a foreign
    table").
  * In a waiting round, that recursive `LOCK TABLE` takes the partitioned
    table and then waits for a partition another transaction read. When that
    transaction then writes through the partitioned table, the two deadlock
    (`40P01`); waiting with `LOCK TABLE ONLY` on the partition alone lets the
    write finish.
* **Partition-tree footprints (probed, 2026-10-16).** On a tree of a
  partitioned table, a partitioned partition with two partitions, and an
  ordinary partition:
  * `DROP TRIGGER` on the partitioned table, for a constraint or plain row
    trigger, takes `ACCESS EXCLUSIVE` on every member and also `ACCESS SHARE`
    on the table it names. `CREATE CONSTRAINT TRIGGER` and
    `CREATE OR REPLACE TRIGGER ... FOR EACH ROW` take `SHARE ROW EXCLUSIVE` on
    every member. On the partitioned partition, the same statements lock only
    its own subtree.
  * `LOCK TABLE ONLY` on each member, as apply's walk lists them, yields
    exactly those relations and modes. An ordinary table's inheritance child
    is in neither footprint.
  * A statement-level trigger has no partition clones. Its `DROP TRIGGER` and
    `CREATE OR REPLACE TRIGGER` lock only the partitioned table.
  * `pg_partition_tree` takes `ACCESS SHARE` on every partition below the
    relation it is given and keeps it until the transaction ends.
    `pg_partition_ancestors` takes no lock.
  * A plain row trigger on a partitioned table is cloned onto a foreign
    partition, and its `DROP TRIGGER` and `CREATE OR REPLACE TRIGGER` lock that
    partition (`ACCESS EXCLUSIVE`, `SHARE ROW EXCLUSIVE`). `LOCK TABLE ONLY` on
    the foreign partition fails ("cannot lock relation", "This operation is not
    supported for foreign tables"). A constraint trigger cannot be created on a
    partitioned table with a foreign partition. Under an existing constraint
    trigger, `CREATE FOREIGN TABLE ... PARTITION OF` and `ATTACH PARTITION` of
    a foreign table both fail ("... is a foreign table", "Foreign tables cannot
    have constraint triggers").
* **Lock privileges (PostgreSQL 18 documentation, probed 2026-10-16).**
  * [`LOCK`](https://www.postgresql.org/docs/18/sql-lock.html) permits any mode
    with `MAINTAIN`, `UPDATE`, `DELETE` or `TRUNCATE` on the table.
    [`CREATE TRIGGER`](https://www.postgresql.org/docs/18/sql-createtrigger.html)
    needs `TRIGGER` on the table and `EXECUTE` on the function.
    [`DROP TRIGGER`](https://www.postgresql.org/docs/18/sql-droptrigger.html)
    needs ownership of the table.
  * A login holding only `TRIGGER` on another role's table can run
    `CREATE TRIGGER`, `CREATE OR REPLACE TRIGGER` and
    `CREATE CONSTRAINT TRIGGER`. `DROP TRIGGER` fails ("must be owner of
    relation"), and `LOCK TABLE ONLY` fails ("permission denied for table").
    After `GRANT MAINTAIN`, or with membership in `pg_maintain`, it locks in
    `SHARE ROW EXCLUSIVE` and `ACCESS EXCLUSIVE` mode.
  * A login that owns a partitioned table, but not one of its partitions:
    * creating a row trigger on the partitioned table fails with "permission
      denied for table" naming the partition, until it holds `TRIGGER` there;
    * `DROP TRIGGER` on the partitioned table removes the partition's clone
      without any privilege on the partition;
    * `LOCK TABLE ONLY` on the partition fails, also with `TRIGGER` granted
      there, until it holds `MAINTAIN`;
    * a recursive `LOCK TABLE` on the partitioned table checks no privilege
      on the partition.
* **Partition membership changes (probed 2026-10-16).**
  * `ATTACH PARTITION` takes `SHARE UPDATE EXCLUSIVE` on the partitioned
    table and `ACCESS EXCLUSIVE` on the attached table.
  * `CREATE TABLE ... PARTITION OF`, `DETACH PARTITION` and `DROP TABLE` of a
    partition each take `ACCESS EXCLUSIVE` on the partitioned table.
  * Renaming a partition takes `ACCESS EXCLUSIVE` on the partition.
  * A `LOCK TABLE ONLY` waiting for a partition that the holder then drops or
    renames fails at the holder's commit with `42P01` ("relation ... does not
    exist").
  * Library live case: a partition attached while apply waits for another
    partition was missing from a set read only once. The later `DROP TRIGGER`
    waited for it while holding the rest and deadlocked with the transaction
    holding it (`40P01`).
  * Library live cases: after a retirement, a managed trigger that another
    session had replaced on a table outside the set, or on one held only in
    `SHARE ROW EXCLUSIVE` mode, made apply's drop wait for that table while
    holding the retired trigger's table. A reader of the table that then
    wrote the other deadlocked with it (`40P01`).
  * Probed cost of the walk that the coverage check runs: 600 separate walks,
    three per table over 100 plain and 100 two-partition tables, took 293 to
    412 ms through `psql` over loopback. One batched walk over the same 200
    tables took 71 to 87 ms, including the same client startup. This was a
    focused query probe, not an application benchmark.
  * Probed cost of uncoalesced lock requests: the lock-set query, given 1,000
    requests for one table with 1,000 partitions, expanded 1,001,000
    recursive rows into 1,001 targets in 3.07 s. Given one request, it
    expanded 1,001 rows in 8 ms. These are isolated query measurements.
* **`NOWAIT` and savepoints (probed).**
  * `LOCK TABLE ... NOWAIT` on a table another transaction holds fails at
    once with `55P03` ("could not obtain lock on relation").
  * `ROLLBACK TO SAVEPOINT` releases every lock taken after the savepoint,
    while `RELEASE SAVEPOINT` keeps them.
  * A wait ended by `lock_timeout` also reports `55P03`, so apply tells the
    two apart by the statement it ran.
  * Validating a SQL-language routine body takes `ACCESS SHARE` on the tables
    it reads, and waits behind an `ACCESS EXCLUSIVE` holder.
  * sqlx 0.9's nested `rollback()` sends `ROLLBACK TO SAVEPOINT` without
    `RELEASE`, which left later statements in a subtransaction whose rows
    failed the `xmin` identity check, so apply issues its own savepoint
    statements.

## PostgreSQL cleanup prepared-plan selection, 2026-10-12

For `batter-z4m9.1`, rechecked PostgreSQL 18's
[`PREPARE`](https://www.postgresql.org/docs/18/sql-prepare.html) and
[query-planning settings](https://www.postgresql.org/docs/18/runtime-config-query.html).
Automatic selection compares a generic plan with the first five custom plans;
`plan_cache_mode = force_generic_plan` overrides that choice, and
`EXPLAIN EXECUTE` inspects the selected prepared plan. Disabling sequential scans
only discourages them; it does not establish a production access-path guarantee.

A disposable `postgres:18` probe of the actual cleanup statement on 20,000
counters (20 expired), with a one-row bound, used primary-key delete probes under
forced custom planning and a full-table hash join under forced generic planning.
The row bound limits deletions, not scan cost. The native plan regression now
checks repeated automatic selection with sequential scans enabled and has a
forced-scan negative control. Executed test outcomes and server versions belong
to the follow-up Bead; this probe is not evidence for every cardinality or cost
configuration.

## PostgreSQL column privileges for key-identified cleanup, 2026-10-11

For `batter-z4m9`, rechecked the rules that decide whether a bounded delete can
run under column privileges once it stops reading `ctid`:

* [`DELETE`](https://www.postgresql.org/docs/18/sql-delete.html): deleting needs
  the `DELETE` privilege on the target table and `SELECT` on any table in the
  `USING` clause or whose values are read in the condition. Column `SELECT` on the
  columns the condition reads is enough; no relation-level `SELECT` is implied.
* [`SELECT`](https://www.postgresql.org/docs/18/sql-select.html) and
  [privileges](https://www.postgresql.org/docs/18/ddl-priv.html): a locking
  `SELECT ... FOR UPDATE` needs `UPDATE` on at least one column in addition to
  `SELECT` on the columns it reads, as recorded on 2026-10-03.
* System columns remain non-addressable by column privileges (2026-10-03 entry
  below), which is why the replacement statement reads only ordinary columns.

Executed evidence, PostgreSQL **18.6** (`postgres:18` container, macOS arm64
host, Rust 1.98.1): the facade's `grants_live` case
`maintained_limiters_clean_expired_rows_under_their_selections_until_drain`
provisions a `NOINHERIT` login holding only the admission and expiry-cleanup
selections, verifies it is exactly within that policy, requires
`SELECT ctid` to be refused, and has registered maintenance delete owner-seeded
expired counters for both families. As a negative control, pointing the native
statement back at the imported `ctid` query under the same narrowed login left
the first cleanup unable to commit, so readiness was never published. The cleanup-only case
additionally requires `SELECT` of every counter column the cleanup does not read
to be refused. Native Runlimit PostgreSQL tests also passed against PostgreSQL
16.15 and 18.6. Other server versions and platforms were not executed.

## PostgreSQL 18 grant semantics for native requirement fragments, 2026-10-03

For `batter-qjxi`, checked the privilege rules that decide what a native
Runledger or Runlimit operation really needs, then executed each one against
PostgreSQL **18.6** (`postgres:18` container, Linux x86-64) before encoding it.

Primary sources, rechecked on 2026-10-03:

* [`SELECT`](https://www.postgresql.org/docs/18/sql-select.html) and
  [`privileges`](https://www.postgresql.org/docs/18/ddl-priv.html): a locking
  `SELECT` requires `UPDATE` in addition to the `SELECT` requirements. Executed
  check: with `SELECT (id, tag)` and `UPDATE (tag)` on one relation, both
  `FOR UPDATE` and `FOR KEY SHARE` succeeded; after revoking `UPDATE (tag)` the
  same `FOR KEY SHARE` read failed with `42501`. One column is enough, and the
  column is the implementer's choice, so `UPDATE(id)` on `job_enqueue_intents`
  and `UPDATE(capacity_shard)` on `runlimit_capacity_shards` are ordinary
  mutation authority rather than an ACL-enforced lock-only capability.
* [`LOCK`](https://www.postgresql.org/docs/18/sql-lock.html): lock privileges are
  relation level. Executed check: `LOCK TABLE ... IN ACCESS SHARE MODE` failed
  with `42501` under column-level `SELECT` only and succeeded after a
  relation-level `GRANT SELECT`; `SHARE ROW EXCLUSIVE` and `ROW EXCLUSIVE` both
  succeeded under relation-level `MAINTAIN` alone. Runledger's full-schema
  snapshot therefore needs relation-level `SELECT`, and catalog synchronization
  and scheduled dispatch need relation-level `MAINTAIN` on `job_definitions` and
  `job_schedules`.
* [`INSERT`](https://www.postgresql.org/docs/18/sql-insert.html): `ON CONFLICT DO
  UPDATE` needs `SELECT` on the arbiter columns and on any column read in its
  expressions. Executed check: `INSERT ... ON CONFLICT (k) DO UPDATE SET a =
  EXCLUDED.a` failed with `42501` under `SELECT (k)` alone and under `SELECT (a)`
  alone, and succeeded with both, so `EXCLUDED` references count as reads of the
  target relation's columns.
* System columns are not column-ACL addressable. Executed check: `SELECT ctid,
  tag` failed with `42501` under column-level `SELECT` and succeeded after a
  relation-level `GRANT SELECT`. Runlimit's bounded fixed-window expiry delete
  selects `ctid`, so that selection reads every counter column.
* [`CREATE TRIGGER`](https://www.postgresql.org/docs/18/sql-createtrigger.html):
  creating a trigger needs `EXECUTE` on its function; firing an installed trigger
  through ordinary DML does not. Executed check: the grant suite revokes all
  routine privileges from PUBLIC in the native schemas and asserts no serving
  login or PUBLIC role holds `EXECUTE` on any routine there, while the installed
  resource-claim and capacity triggers still run.

These are implementation-specific semantics of PostgreSQL 18.6 on Linux. No other
server version or platform was executed for this task, and a trigger function's
owner, body safety and application-added trigger dependencies remain explicit
application prerequisites.

## Native query adapters: reviewed 2026-09-22

For `batter-ywd2`, checked SQLx0.9.0's locked `sqlx-core/src/query.rs`,
`query_scalar.rs`, and native executor definitions, plus its
[Map documentation](https://docs.rs/sqlx/0.9.0/sqlx/query/struct.Map.html) and
[query macro contract](https://docs.rs/sqlx/0.9.0/sqlx/macro.query.html).
Map fetches apply the mapper and preserve native errors. Map also implements
Execute; executor-level execute discards rows without invoking that mapper.
QueryScalar similarly retains scalar decoding for fetches. SQLx macros consume
live database descriptions or generated offline metadata; forwarding their native
values preserves that checking. Runtime constructors produce the same types, so
macro origin cannot be enforced by a trait over those values.

The initial opaque-future adapter failed concrete Send checks with Rust1.98.1's
diagnostic referring to the compiler's
[higher-ranked lifetime limitation](https://github.com/rust-lang/rust/issues/100013).
Boxed Send dispatch futures compiled with those checks while keeping concrete
query output/error types. No measured latency claim follows from statement counts
or this representation choice. Native query output precedes PgLease's existing
asynchronous idle-state handshake; retaining it before cleanup is a local result
publication guarantee, not a transaction or external-effect proof.

## Fail-fast transaction recovery: reviewed 2026-09-22

For `batter-wobk`, SQLx remains 0.9.0. PostgreSQL 18
[ROLLBACK TO SAVEPOINT](https://www.postgresql.org/docs/18/sql-rollback-to.html)
allows recovery from failed SQL; [RELEASE SAVEPOINT](https://www.postgresql.org/docs/18/sql-release-savepoint.html)
removes the named and later savepoints. The
[transaction status functions](https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-PG-SNAPSHOT)
report the supplied transaction's status, not the history of work on a connection.
Executed PostgreSQL 18.6 counterexamples showed that an original aborted XID can
coexist with a committed replacement transaction and a later failed transaction.
Our inference is that checking original status after bare ROLLBACK cannot preserve
the existing transaction-boundary-loss contract. The fast runner therefore retains
one private birth guard, recovers through it on rejection, and validates the
original XID before whole rollback. Successful calls need no per-call savepoint.
Native tests count two statements unprofiled or three profiled, excluding setup
and completion, and retain uncertainty for replacement or missing-guard cases.

## Declared transaction timeouts: reviewed 2026-09-22

Owning Bead: `batter-qhps`; resolved SQLx 0.9.0 and PostgreSQL 18.6.

- PostgreSQL 18 [RESET](https://www.postgresql.org/docs/18/sql-reset.html) and
  [DISCARD](https://www.postgresql.org/docs/18/sql-discard.html) establish that
  reset restores defaults, including startup options, rather than always zeroing
  timeouts. Session `SET` values are removed. SQLx 0.9.0's local
  `sqlx-postgres/src/options/mod.rs::options` and `connection/establish.rs` send
  those options in the startup packet.
- PostgreSQL 18 [client defaults](https://www.postgresql.org/docs/18/runtime-config-client.html)
  specify millisecond units, zero as disable, idle-interval versus total-transaction
  termination, precedence when total timeout is shorter or equal, and the prepared
  transaction exclusion. These are server policies, not Rust callback deadlines.
- The PostgreSQL 17 [release notes](https://www.postgresql.org/docs/17/release-17.html)
  introduce `transaction_timeout`. Complete declarations require its existence
  even for explicit zero; native setup errors reject unsupported parameters.
  Compatibility construction does not query or set either undeclared GUC.
  This historical feature introduction does not define Batter's support floor:
  PostgreSQL 18 is the minimum for both constructors and all PostgreSQL integrations.

The native cases distinguish startup/session values, profile enforcement and
independently observed backend/lock release from a still-held local lease. This
does not establish a hard wall-clock guarantee or permanent protection from SQL
that changes settings between validation boundaries. PostgreSQL 16/17 runtime
coverage remains unexecuted here and those versions are unsupported. Fresh-agent
usability evaluation remains unexecuted.

## PostgreSQL 18 schema and runtime-authority verification, 2026-09-13

The verifier targets PostgreSQL **18.x** and rejects other server-version
families before issuing the version-specific catalog queries. Its role graph
follows the PostgreSQL 18 [`pg_auth_members` catalog](https://www.postgresql.org/docs/18/catalog-pg-auth-members.html)
and the documented [`INHERIT`, `SET` and `ADMIN` role-membership semantics](https://www.postgresql.org/docs/18/role-membership.html):
object ACLs are evaluated from captured catalog rows, `SET` reachability is a
separate path, and role attributes are not treated as inherited object grants.
An `ADMIN TRUE` edge to an ordinary role is usable by its member regardless of
that edge's `INHERIT` value. PostgreSQL 18's
[`membership authorization check`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/user.c#L2110-L2173)
instead reserves grants and revocations of membership in a superuser role to an
already active superuser, so an ADMIN-only superuser target is not promoted to
usable authority. Ordinary membership grants follow privilege reachability, while
the PostgreSQL 18 [`ALTER ROLE` rules](https://www.postgresql.org/docs/18/sql-alterrole.html)
also let an active CREATEROLE identity manage a target for which it holds ADMIN
through membership edges even when those edges grant neither SET nor INHERIT.
Because ADMIN lets that target be granted back with SET and INHERIT enabled,
the verifier retains target identities and audits each target's potential
authority instead of reducing ADMIN to one boolean.
The [`GRANT` reference](https://www.postgresql.org/docs/18/sql-grant.html)
also establishes PUBLIC grants and grant options as independent authority
surfaces. The [`pg_database_owner` contract](https://www.postgresql.org/docs/18/predefined-roles.html)
gives that role exactly one implicit member, the current database owner, and
forbids stored membership grants both into and out of the role. The verifier
therefore models that implicit edge separately and treats any `pg_auth_members`
row involving `pg_database_owner` as uninterpretable catalog evidence.
[`SET SESSION AUTHORIZATION`](https://www.postgresql.org/docs/18/sql-set-session-authorization.html)
allows an initially authenticated superuser to mask `session_user` and later
reset it. The verifier therefore records the call-entry effective role, uses
`SET LOCAL SESSION AUTHORIZATION DEFAULT` to recover the initial database role
for authority inspection, then reconstructs the call-entry session/current
identity before catalog and ledger reads. Rollback restores the caller's
session-level identity state.

The [`pg_parameter_acl` catalog](https://www.postgresql.org/docs/18/catalog-pg-parameter-acl.html)
is cluster-wide. Live fixtures therefore use unique custom parameter names and
require their administrative connection to target a dedicated disposable
cluster; cleanup never revokes a fixed operator-owned parameter grant.

ACL rows are captured with [`aclexplode`](https://www.postgresql.org/docs/18/functions-info.html)
and catalog defaults inside one repeatable-read, read-only transaction. The
transaction-isolation contract does not make internal privilege helpers a
repeatable snapshot, so the implementation evaluates captured ACL, role and
catalog rows locally rather than calling `has_*_privilege` or `pg_has_role`.
Routine identity is reconstructed from the captured [`pg_proc`](https://www.postgresql.org/docs/18/catalog-pg-proc.html)
input-type OIDs and [`pg_type`](https://www.postgresql.org/docs/18/catalog-pg-type.html)
rows; formatted catalog strings are display-only. True arrays require
`array_subscript_handler` and the element type's reverse `typarray` link, keeping
subscriptable `oidvector`/`int2vector` identities distinct. PostgreSQL's single
array type OID cannot represent source-level dimensionality. PostgreSQL's
[`MVCC caveats`](https://www.postgresql.org/docs/18/mvcc-caveats.html)
and [`transaction isolation`](https://www.postgresql.org/docs/18/transaction-iso.html)
documentation bound this choice.

Protected-parameter ACLs use the PostgreSQL 18 [`pg_parameter_acl` catalog](https://www.postgresql.org/docs/18/catalog-pg-parameter-acl.html),
including PUBLIC and grant-option entries. [`pg_settings`](https://www.postgresql.org/docs/18/view-pg-settings.html)
is only an auxiliary source for visible parameter context: a requested name
may have an ACL row while remaining hidden from that view, and is not reported
as missing for that reason. PostgreSQL also accepts unknown custom names with
two or more identifier components. Its [`valid_custom_variable_name` implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/misc/guc.c)
does not apply the 63-byte catalog-identifier limit to the complete name, so a
caller-declared custom placeholder retains implicit SET authority even when an
unrelated ACL is its only catalog row. Context-wide defaults are evaluated only for
parameters named by the caller, while every explicit parameter ACL row remains
in scope. PUBLIC parameter allowances are declared independently from the
reachable-role allowance. The generic result explicitly lists supported ACL
surfaces and unsupported classes; application-owned ledger shape/history,
SECURITY DEFINER search paths and bodies, extension upgrade semantics and
profile-specific grants remain additive local checks.

The migration reader checks the ledger's
[`pg_class`](https://www.postgresql.org/docs/18/catalog-pg-class.html)
row-security flags before reading rows. PostgreSQL row-level security can
silently filter `SELECT` results, so an enabled policy is a finding
rather than evidence for a complete migration comparison.

### Owned execution and downstream policy correction, 2026-09-13

Rechecked PostgreSQL 18 [LOCK ordering and privileges](https://www.postgresql.org/docs/18/sql-lock.html),
[MVCC caveats](https://www.postgresql.org/docs/18/mvcc-caveats.html),
[row_security](https://www.postgresql.org/docs/18/runtime-config-client.html#GUC-ROW-SECURITY)
and [membership semantics](https://www.postgresql.org/docs/18/role-membership.html).
ACCESS SHARE requires SELECT (or a stronger listed privilege), survives until
transaction end, and allows ordinary row writers. Taking it before the first
SELECT establishes the intended DDL ordering. `row_security = off` requests a
failure for filtered reads; it does not bypass policies.

The installed SQLx 0.9.0 PostgreSQL `transaction.rs` and `connection/mod.rs`
separate managed transaction depth from private protocol transaction status.
Consequently the protected verifier owns a fresh pool checkout and acknowledges
ROLLBACK of residual raw state; it cannot infer arbitrary borrowed-connection
idleness from managed depth. The serving pool's session configuration remains
application-owned. Migration-only and authority-only entrypoints share this
executor and expose only their inspected coverage.

Current-role required ACLs follow PUBLIC and INHERIT, not SET/ADMIN potential.
Table privileges also supply corresponding column privileges; revoked owner
ordinary privileges remain revoked until explicitly restored. These semantics
are evaluated from captured catalog rows, with native SQL controls in the live
suite and disposable consumer. Schema discovery extends selection, not the
meaning of a privilege. Per-kind defaults and exact overrides replace duplicate
consumer catalog queries; application/native function and history checks remain
outside the shared evaluator.

Parameter names use case-normalized comparison keys for both `pg_settings` and
`pg_parameter_acl`. Each materialized catalog query, including expanded ACLs,
uses a 10,001-row sentinel and rejects counts above 10,000. Variable-length
parameter names are checked before transfer against 1,024 bytes; fixed catalog
identifiers retain PostgreSQL's native limits. These are client retention limits,
not an assertion about server execution resources.

### Catalog resolution and dependent type ACL correction, 2026-09-13

PostgreSQL 18 [schema resolution](https://www.postgresql.org/docs/18/ddl-schemas.html#DDL-SCHEMAS-PATH)
permits application overloads on the search path to intercept unqualified
functions and operators. An exact overload can win over an implicitly cast
built-in even when pg_catalog appears first. The owned transaction therefore
sets a trusted path containing only pg_catalog and the explicit trailing
pg_temp position before inspection, and rollback restores application settings.
Native controls cover both current_database redirection and an exact left(name,
integer) overload that could hide discovered schemas.

The [LOCK reference](https://www.postgresql.org/docs/18/sql-lock.html) distinguishes
parent-only from descendant-inclusive locking. The earlier descendant lock repair
prevented child TRUNCATE after capture, but did not freeze inheritance membership.
PostgreSQL18 [inheritance DDL](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/tablecmds.c)
takes ShareUpdateExclusive on the parent for attachment, compatible with AccessShare.
[Planner descendant discovery](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/pg_inherits.c)
uses a fresh catalog snapshot. A PostgreSQL18.6 two-session reproduction deletes
a required row from an unrelated table and attaches it in one commit: the ledger
is empty before and after, but the earlier row snapshot reads that deleted row
through the new membership. The adapter therefore rejects inheritance present in
the captured snapshot and reads supported standalone ledgers with ONLY. It does
not require stronger serving-role privileges or reconstruct an inheritance engine.

PostgreSQL's [type ACL implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/aclchk.c)
resolves automatic array access through the element and multirange access through
the range. Their own synthesized default ACLs are not independent effective
authority. Discovery and explicit type policy must retain effective element/range
checks without mistaking aliases for standalone types. The reverse array link
keeps automatically generated aliases separate from catalog vector identities
in discovery and routine signatures. Effective ACL resolution follows PostgreSQL's
element/subscript-handler rule, including vectors, without that reverse-link condition.

The [pg_settings visibility contract](https://www.postgresql.org/docs/18/view-pg-settings.html)
allows superuser-only settings to be hidden without pg_read_all_settings. A
restricted native probe confirms session_preload_libraries and dynamic_library_path
are absent from the view. Absence alone is not evidence that the setting does
not exist; declared-but-unobservable parameters need explicit incomplete coverage.
Even privileged pg_settings omits NO_SHOW_ALL settings such as role. For absent
noncustom names, only captured current superuser or INHERIT pg_read_all_settings
visibility permits a bounded `current_setting(name, true) IS NOT NULL` existence
probe. It returns no values and supplies no missing context or ACL authority.
Existing settings without observable context remain Incomplete; positively absent
names become MissingObject. Restricted sessions do not run that probe. ACL and
role rows use the catalog snapshot; the existence probe describes the serving session.

The final required-parameter correction separates possible excess authority from
positive current-role requirements. PostgreSQL's
[custom-name assignment check](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/misc/guc.c)
rejects reserved prefixes even for syntactically valid dotted names;
[PL/pgSQL initialization](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/pl/plpgsql/src/pl_handler.c)
reserves plpgsql. Native execution loads that language, rejects an unknown setting
under its prefix, and demonstrates that syntax alone cannot prove required SET.
[Configuration visibility](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/misc/guc_funcs.c)
can also hide custom definitions. A required custom name without observable
context therefore yields Incomplete, including when a separate ACL is present.
Ordinary placeholder potential remains conservative; a visible loaded parameter
still supports a positive requirement. No live SET probe enters verification.

## Protected PostgreSQL catalog safeguards, 2026-09-14

PostgreSQL 18's [schema contract](https://www.postgresql.org/docs/18/ddl-schemas.html)
states that tables, sequences, indexes, views, materialized views, and foreign
tables share one namespace within a schema. The
[`pg_class` catalog](https://www.postgresql.org/docs/18/catalog-pg-class.html)
stores those relation-like objects under one schema/name identity and separates
ordinary tables (`r`), partitioned tables (`p`), views (`v`), materialized views
(`m`), foreign tables (`f`), and sequences (`S`) with `relkind`. Batter therefore
rejects one qualified policy identity that requires both relation and sequence
semantics, including a migration ledger combined with a sequence declaration,
before execution. This is a local representability rule; it does not claim that
an independently changed database still matches a compiled policy. A captured
opposite kind is therefore database drift: the declared kind remains a
missing-object or missing-privilege finding, and discovery evaluates the catalog's one actual object
with defaults for that actual kind. Only a structurally impossible captured
snapshot containing both kinds for the same identity is a catalog-expansion
failure.

The selected SQLx 0.9.0 source in the locked Cargo registry creates
`_sqlx_migrations` with `version bigint` as its sole primary key plus non-null
`description text`, `installed_on timestamptz`, `success boolean`, `checksum
bytea`, and `execution_time bigint`. The protected manifest copies only bounded
version/checksum expectations; it does not enable SQLx's migrate feature, retain
SQL bodies, or run `Migrator`.

PostgreSQL 18's [`pg_proc`](https://www.postgresql.org/docs/18/catalog-pg-proc.html)
stores per-routine settings in `proconfig` and marks SECURITY DEFINER routines
with `prosecdef`; its `proargtypes` entries reference `pg_type` OIDs and define
the input call signature. Catalog-to-policy reconstruction therefore preserves
an exact identifier or routine-type `PolicyError` as the source of a redacted
catalog-identity failure rather than discarding the structural cause. The
[CREATE FUNCTION security guidance](https://www.postgresql.org/docs/18/sql-createfunction.html#SQL-CREATEFUNCTION-SECURITY)
recommends excluding writable schemas from a definer's search path and placing
`pg_temp` last. Batter verifies the application-selected exact stored string for
every definer in explicit schemas; it does not infer body safety from that fact.

PostgreSQL 18's [`pg_shdepend`](https://www.postgresql.org/docs/18/catalog-pg-shdepend.html)
retains database-local object addresses as `classid`, `objid`, and `objsubid`,
the referenced role in `refobjid`, and distinguishes owner (`o`) rows from ACL,
initial-ACL, policy, and tablespace dependency kinds. The protected ownership
check excludes structurally valid rows of the three non-owner role-reference
kinds and well-formed owners outside the login's capability graph before applying
its bound. Missing referenced roles remain retained evidence. The evaluator
requires the database owner, every membership endpoint, every retained owner and
the predefined `pg_database_owner` identity to resolve in the same bounded role
snapshot; it reports reachable owner addresses coarsely and fails incomplete on
unknown, malformed, impossible, or missing role-reference forms. In particular, a
tablespace dependency cannot reference `pg_authid`. The same catalog
documentation states that pinned roles have no
dependency records; capability reachability to a superuser or predefined role
therefore also makes the ownership-specific result incomplete. Other databases
and application-specific object semantics remain outside the result.

PostgreSQL 18's [`DECLARE`](https://www.postgresql.org/docs/18/sql-declare.html)
and [`FETCH`](https://www.postgresql.org/docs/18/sql-fetch.html) references define
a non-holdable cursor as transaction-local and make later row retrieval operate
through that previously created cursor. The
[`pg_cursors` view](https://www.postgresql.org/docs/18/view-pg-cursors.html)
lists the current session's available cursor names. The verifier chooses a
bounded generated name absent from that inventory so a caller's surviving
holdable cursor cannot collide with its transaction-local portal. The
[`pg_locks` view](https://www.postgresql.org/docs/18/view-pg-locks.html) exposes
granted relation locks for the current backend independently of ordinary catalog
snapshot visibility. At the same pinned source revision,
[`FirstNormalObjectId`](https://github.com/postgres/postgres/blob/c66a31c21595a88ec5f38e4c92c51dbd8d42846e/src/include/access/transam.h)
is 16,384 and the surrounding contract reserves lower OIDs for initdb/catalog
objects; normal multiuser allocation resumes at that boundary after wraparound.
The rechecked PostgreSQL 18 source at
[`c66a31c2`, `PerformCursorOpen`](https://github.com/postgres/postgres/blob/c66a31c21595a88ec5f38e4c92c51dbd8d42846e/src/backend/commands/portalcmds.c#L39-L139)
plans the cursor query, copies that plan into the cursor's portal, starts the
portal, and defers actual row execution until fetch; its
[`AtSubCommit_Portals`](https://github.com/postgres/postgres/blob/c66a31c21595a88ec5f38e4c92c51dbd8d42846e/src/backend/utils/mmgr/portalmem.c#L937-L964)
reparents a cursor created in a successfully released savepoint. The protected
migration path therefore opens its non-holdable history cursor immediately after
proving the declared name resolves to the locked OID. Before and after that
declaration it inventories the current backend's ACCESS SHARE relation locks. A
new non-catalog lock is accepted only for the protected heap or its snapshot-known
indexes and TOAST relations, so planning the quoted RangeVar against an empty
replacement is observable even though no row can supply `tableoid`. The portal
uses a numeric retained-OID identity constant rather than putting a
policy-controlled name in an SQL string literal. The later fetch checks every
returned `tableoid`; resolution failure, an unexpected planning lock or an OID
mismatch is incomplete rather than a read of a purportedly protected replacement.

## Exact-role grant rendering, 2026-09-13

PostgreSQL 18's primary [`GRANT` reference](https://www.postgresql.org/docs/18/sql-grant.html)
was rechecked for the pure exact-role renderer. It defines the distinct database,
schema, table, column and routine statement forms, permits `ROUTINE` to name
functions or procedures without requiring the compiler to classify the routine,
and restricts routine privileges to `EXECUTE`. The renderer therefore validates
privileges against the selected object kind, renders structural overload argument
types through the existing `RoutineSignature`, uses `ON ROUTINE`, and never
turns verification-only grant-option ceilings into `WITH GRANT OPTION`. The same
reference states that PostgreSQL does not grant grant options to PUBLIC; PUBLIC
delivery remains verifier policy rather than renderer output. No database
connection or server behavior is inferred from this syntax-only contract.

The PostgreSQL 18 [`CREATE ROLE` reference](https://www.postgresql.org/docs/18/sql-createrole.html)
and [predefined-role inventory](https://www.postgresql.org/docs/18/predefined-roles.html)
were also checked for renderer target boundaries. The high-level renderer rejects
the PUBLIC/NONE special spellings and PostgreSQL's reserved `pg_` namespace. This
does not prove an ordinary target has no members; application role selection and
membership remain outside the pure plan.

Post-review row-type identity was rechecked against PostgreSQL 18's
[`CREATE TABLE` reference](https://www.postgresql.org/docs/18/sql-createtable.html)
and [`pg_class` catalog](https://www.postgresql.org/docs/18/catalog-pg-class.html).
Every table receives a same-name composite row type, including a typed table
created with `OF`: `reltype` identifies that table row type, while `reloftype`
separately identifies the stand-alone composite type from which a typed table is
derived. A relation-name exact PUBLIC type override therefore addresses the
same row type that the verifier selects through `typrelid`; it does not stand in
for policy on the underlying stand-alone type.
## Managed-object release fence — 2026-11-05

- PostgreSQL 18 [ALTER TABLE](https://www.postgresql.org/docs/18/sql-altertable.html):
  adding the nonvolatile constant-default column under the already-held
  `ACCESS EXCLUSIVE` lock changes the ledger shape transactionally.
- PostgreSQL 18 [transaction isolation](https://www.postgresql.org/docs/18/transaction-iso.html):
  apply uses READ COMMITTED after acquiring the shared session advisory lock;
  each waiting applier sees the prior committed revision.
- PostgreSQL 18 [explicit locking](https://www.postgresql.org/docs/18/explicit-locking.html):
  session advisory locking spans migration and managed transaction boundaries;
  transaction-held relation locks serialize verification and shape adoption.

Local dependency remains SQLx 0.9.0; live validation server is PostgreSQL 18.6.
This change adds no dependency. Application revision numbers are not derived from
SQLx migration history.

## Intent reconciliation snapshot and grants (2026-11-05)

- PostgreSQL 18 [transaction isolation](https://www.postgresql.org/docs/18/transaction-iso.html):
  an ordinary READ COMMITTED SELECT sees a statement snapshot and its own
  transaction's changes; separate statements need not see the same committed state.
  The native reader therefore joins all three metadata sources in one statement.
- PostgreSQL 18 [GRANT](https://www.postgresql.org/docs/18/sql-grant.html): SELECT
  can be granted on named columns. The new operation grants only the columns its
  projection and predicates need. Existing wider grants remain effective.
- Integration uses the workspace-locked SQLx 0.9.0 API and existing owned atomic
  runner; live failure evidence records PostgreSQL 18.6. No SQLx transaction or
  wire protocol implementation was added to production code.

## Shared profiled pool ownership (2026-11-05)

For `batter-nhu5`, rechecked the resolved SQLx 0.9.0 registry sources:
`sqlx-core/src/pool/options.rs` (`connect_lazy_with`, lines 574–582),
`sqlx-core/src/pool/mod.rs` (`close`, lines 420–443), and the shared `PoolInner`
implementation. The upstream source locations are
[pool options](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/options.rs)
and [pool closure](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/mod.rs).
Lazy construction can launch minimum-connection maintenance. Pool clones share
capacity and closure; awaited close drains pool-accounted connections and waits
for outstanding checkouts. It does not prove detached server sessions ended.
The adapter's existing three profile hooks are unchanged. Local source inspection,
plus the native PostgreSQL shared-pool test, is the executed evidence; the web
retrieval of versioned docs.rs pages was unavailable during this check.
