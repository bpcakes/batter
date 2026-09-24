Address PR #13 review coverage under Bead batter-rpgk, based on
1044cd7dd8d4d0220f35d9f36d42c877508551b4.

Added a closed-pool begin failure whose policy cancels the context before
returning its consumer error. Both context wrappers must retain that error.
A live deferred-commit failure covers both wrappers with provisional output,
SQLSTATE and constraint assertions. Native intent observation and resource
enqueue now exercise real CHECK failures, consumer conversion, successful SQL
after savepoint recovery, and acknowledged persistence of the successful rows.
There are no production API or dependency changes. Existing exhaustive Jig
source/test/script inputs cover the added file and live inventory entry.

Focused Rust 1.98.1 checks passed on macOS arm64: two context unit tests,
two native policy integration tests, and the new live policy test on PostgreSQL
18.6. Both full `scripts/verify.sh` runs passed (Rust 1.98.1 and 1.94.0),
including native tests, doctests, strict Clippy and rustdoc. The minimum-toolchain
run used `CARGO_TARGET_DIR=target/policy-coverage-msrv` for independent builds.
The exact SQLx live inventory passed 107/107 on each toolchain with PostgreSQL
18.6. Both HTTP builds and all five smoke modes per toolchain passed. The
disposable live-test container was stopped after execution.

The first full SQLx live run passed the new controls but hit two-second local
operation bounds in three pre-existing cancellation tests. All three and the
full inventory passed unchanged on rerun; no assertions or bounds were weakened.

Jig passed Clippy, formatting, contract and file budgets, but its first test
matrix returned workspace exit 101. Its 64 KiB progress preview and stored
receipt omitted the failing test diagnostic. A direct rerun of the exact
workspace command passed with 167 successful test-target summaries. The final
`scripts/jig check api:test --plan-id plan_01M371N4R7E4Y0QGK8JWZ7JTF2` rerun
passed unchanged. The initial Jig failure's specific test/cause remains unknown;
the subsequent complete workspace and Jig runs did not reproduce it.
The passing final `api:test` receipt is
`receipt_01M373REV4WF98KTGEYEJESV01`; source inputs and the default toolchain
remained unchanged through documentation and tracker closeout.

Cargo.lock is unchanged, SHA-256
`ec6260d8f553a2dc6a176820033cbfd8029d94b497b1473d2722df455540b1af`.
No production implementation, public API, migration or dependency changed.
No Linux execution, commit, push, merge or publication is claimed.
