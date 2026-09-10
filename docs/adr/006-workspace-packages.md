# ADR-006: Separate adoption packages in a virtual workspace

Status: accepted. Date: 2026-09-08.

Context: the early foundation has no downstream consumers. Its optional Axum
module and SQLx example currently share one package manifest, making package
ownership and dependency adoption less explicit than their actual roles.

Decision: use a virtual root workspace with three libraries: `batter`,
`batter-axum`, and `batter-test-support`. Move the native SQLx executable to
`batter-example-postgres-lifecycle`. Keep core examples/tests with the core,
HTTP examples/tests with the adapter, and SQLx dependencies with the executable.
Make the direct source cutover from `batter::http` to `batter_axum`; remove the
old `axum` and `postgres-example` features without a compatibility facade.

The core keeps its existing modules together. The Axum package depends on it;
the core never depends on the adapter. Generic test support remains independent
of both. A public `batter::telemetry::with_current_dispatch` helper returns an
opaque future while the pin/drop wrapper stays private. It captures dispatch
at the helper call and protects polling and destruction without adding task
ownership or `Send`/`'static` bounds. The HTTP entrypoint calls it during its
first poll to preserve the previous behavior. `RequestPolicy` still combines
readiness and request deadline policy; this is not a middleware redesign.

Each package declares version 0.1.0, Rust 1.94, and `publish = false` explicitly.
Share the root lockfile, dependency requirements, lints, and verification tooling.
SQLx's minimum applies to the example; this move makes no lower-MSRV claim for
the libraries. Independent packages may acquire different versions/minimums
later, subject to their dependency contracts and validation.

The external PostgreSQL harness stays outside the workspace, with no new
dependency or downstream change. No SQLx, Runlimit, Runledger, or fixture library
is created speculatively. Integration composition belongs in an unpublished
reference application first. Generic test support must not gain higher-layer
dependencies; foundation tests already use it. This also avoids dev-dependency
cycles and their duplicate-type hazards.

Consequences: consumers select the packages they need, and library adoption does
not bring in SQLx or Axum. Workspace commands select packages explicitly; the
full matrix still compiles the SQLx executable without claiming PostgreSQL
execution. Existing lifecycle, HTTP, tracing, and cleanup failure contracts
must pass after relocation, and the public dispatch seam needs direct tests.
Publication remains a separate decision. See [Cargo references](../references.md#workspace-packaging-reviewed-2026-09-08)
and [validation](../validation.md) for external semantics and execution evidence.

SQLx amendment, 2026-09-09 (`batter-7r3.2`): add a fourth library, independently
selected `batter-sqlx`, and retain the native lifecycle executable as its runnable
consumer. The original example-only SQLx restriction is superseded for proven
connection-disposition mechanics. Core and generic test-support dependency
directions remain unchanged. The adapter keeps Rust 1.94, version 0.1.0 and
publishing disabled; consumers select their own native SQLx TLS features.

Reference amendment, 2026-09-09 (`batter-4t6`): add the second unpublished example,
`batter-example-reference-service`, for executed SQLx/Runledger/harness compatibility
probes. The harness is an external-only development dependency of that package;
its provisioning implementation remains upstream. The workspace now has four
libraries and two examples, with one native SQLx graph and unchanged core/leaf
dependency direction. These probes do not implement a Runledger adapter.

Fixture amendment, 2026-09-09 (`batter-4jz`): the optional `test-support` feature of
`batter-sqlx` now selects the pinned external-only harness and generic test support.
It shares declared native pool/database ownership, ordered fingerprint inputs and
lock observation. Reference tests consume it through a development dependency;
application SQL and initializer policy stay there. Default SQLx adapter, core and
generic leaf graphs exclude the harness. Provisioning and template caching remain
upstream; no Runledger dependency enters the fixture module.
