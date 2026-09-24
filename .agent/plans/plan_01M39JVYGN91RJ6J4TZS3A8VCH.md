Merge origin/feat/sqlx-failure-policy into PR 14, preserving fail-fast behavior, OperationOwner cancellation, native error exports and both sets of tests. Validate the merged tree on Rust 1.98.1 and 1.94.0, HTTP smokes, live SQLx tests where prerequisites are available, and required Jig gates. Owning Bead: batter-wobk.

Resolved four textual conflicts by preserving the fail-fast runner and its
unpolled/pre-cancelled controls, the base OperationOwner cancellation path,
both PgScopeRolledBack/PgTransactionError exports, and both documentation histories.
Migrated the fail-fast live cancellation control to OperationOwner as well;
no assertions, public API behavior, dependencies, or migrations changed.
The combined exact live inventory contains 52 atomic and 115 total cases.

Executed on macOS arm64 with disposable PostgreSQL 18.6: all 115 SQLx live tests
and all five rebuilt HTTP smoke modes pass on Rust 1.98.1 and 1.94.0.
Both full `verify.sh` runs passed, including workspace tests, doctests, Clippy
and rustdoc. The unchanged Jig rerun passed all five required gates; api:test
receipt: `receipt_01M39MDNAEWTX0CTXBY3PWW5W6`, run:
`run_01M39KS0SP262C106H5112XMQW`.
Logs: /tmp/batter-pr14-{verify,live,http}-{1981,1940}.log and
/tmp/batter-pr14-jig.log.


The first Jig gate run was invalidated by concurrent documentation edits.
Its workspace-test command also returned status 101; the retained Jig output
preview does not include the individual failing test. The separate Rust 1.98.1
workspace run passed. The unchanged required-gate rerun passed. No assertions were relaxed; the
individual cause of the initial test failure remains unestablished.
