# Native Runledger packages

Follow the root Batter guide and nearest package guide. These five packages own
native durable jobs, persistence, supervision, test support and the operator TUI.
They are members of the root Cargo workspace, not an independent nested workspace.
They must not depend on the Batter facade. Preserve package names, native public
contracts, strict package lint policy, coordinated version 0.13.0, and
publication restricted to crates.io.

PostgreSQL 18 is the authoritative database baseline. Native database tests use
`runledger-test-support` and Docker's `postgres:18` image by default. Record exact
server versions with live evidence; other PostgreSQL majors remain unverified.
Batter's own SQLx fixture provisioning remains external and separate.

`migrations/` is the canonical native schema source. Never rewrite applied SQL.
Keep `runledger-postgres/migrations/` and `runledger-test-support/migrations/`
identical. Keep canonical `.sqlx/` synchronized with each checked-query package's
copy. Do not delete tests or bypass container lifecycle checks to make migration
verification pass. See the root testing guide and [import provenance](IMPORT.md).

Use the root Beads tracker; the old repository's tracker/plans remain historical
upstream records. Run commands from the Batter root. Root verification includes
native packages; use `cargo test -p runledger-core -p runledger-postgres
-p runledger-runtime -p runledger-test-support -p runledger-tui --locked` for the
focused native suite. Publication/deployment still needs an explicit decision.
