# SQLx scope validation statement reduction

Owning Bead: `batter-hkgr`. Public APIs and dependencies remain unchanged.

Combine declared profile observations and atomic continuity in one native query.
Successful operations validate after the body and await RELEASE without a second
validation. Unprofiled opening validation is redundant because the owner exposes
SQL only inside an operation and validates before returning. Profiled openings
remain: another session can remove a schema or revoke USAGE between operations.
The resulting one-query success count is four unprofiled or five profiled,
excluding acquisition/setup and final completion; recovery costs one more.

New live controls exercise those counts for first/repeated/post-recovery scopes,
both profile constructors, final commit, each setting, session/effective role drift,
external schema removal and external USAGE revocation. Existing live tests cover
XID replacement, swallowed errors, nested savepoint confinement, cancellation,
poison retention and uncertain commit. Snapshot and pool profile checks reuse the
combined observation without changing their lifecycle or first-snapshot ordering.

Progress: implementation and four new PostgreSQL 18.6 controls pass. The exact
99-case SQLx live inventory passes on both Rust 1.98.1 and 1.94.0/macOS arm64.
All five HTTP smoke profiles pass on both toolchains. Both complete
`bash scripts/verify.sh` runs pass, including native Docker tests, doctests, Clippy
and rustdoc. Final Jig run `run_01M35F830A3HJ7XCRF7AA3N42Z` passes all five required
targets, including `api:test` receipt `receipt_01M35FRMAZY9ZVS4Y5PGB3EDBB`.
The initial Jig run was rejected because tracker edits occurred during its
read-only layer; it also reported a workspace-test exit 101 with truncated
failure detail. Both standalone verification runs and the final unchanged-tree
Jig rerun passed without code/test changes or relaxed assertions. The temporary
PostgreSQL cluster was removed after live verification. Cargo.lock remains
unchanged (SHA-256 d7f2d01ef91cf346c1ac3ff843b9a3c4192e8cba744042e061adfa40bce8fe77).
No Linux-client or hosted CI execution is claimed for this change.

Existing exhaustive `crates/*/src/**`, `crates/*/tests/**` and script scopes cover
the new files in both Jig contract files. No input roots or public APIs are added.
The pre-existing tracker SQLite import conflict is isolated by using br --no-db;
only this Bead is appended to the checked-in JSONL.
