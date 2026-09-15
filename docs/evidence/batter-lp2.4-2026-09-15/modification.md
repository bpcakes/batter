# Frozen modification prompt

The evaluator told the original author that `initial-repair-1` compiled and
passed all independent initial runtime oracles against PostgreSQL 18, then gave
this execution-free modification:

1. Acquire a second application dependency during protected startup after the
   database pool. Reserve and register its real asynchronous cleanup under the
   exact name `secondary.close` before advancing. Keep database cleanup named
   `database.close`.
2. Add a fallible startup stage after both dependencies are acquired. When
   `CONSUMER_STARTUP_FAILURE=1`, return a concrete startup failure before
   publishing the listener. Fixed stderr remains exactly `consumer failed`.
3. For startup failure, normal SIGTERM, and later native failure, inspect the
   actual retained cleanup report and print each successful cleanup record
   exactly once in executed LIFO order: `consumer-cleanup:secondary.close:succeeded`
   followed by `consumer-cleanup:database.close:succeeded`, then the appropriate
   existing result line. Do not manufacture success from names alone.
4. Preserve `/query`, `/trigger-failure`, real native queries, exit behavior, and
   secret-safe fixed diagnostics.
5. Do not add or duplicate signal routing/listeners; reuse the original protected
   startup signal selection unchanged.

At most two repair attempts were available after the first modification
submission. The author was required to write a new source directory, not alter
any earlier submission, and was forbidden to compile, run, test, format, inspect
the oracle, or read inputs outside the original public packet.
