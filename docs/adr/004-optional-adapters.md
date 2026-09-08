# ADR-004: Optional adapters and preserved upstream ownership

Status: accepted for the MVP source. Date: 2026-09-07.

Context: HTTP handlers, workers, CLIs, and tests need shared operational behavior
without importing a web framework everywhere. The user's existing libraries
already have clear persistence, runtime, policy, and provisioning boundaries.

Decision: keep Axum optional and SQLx native. The SQLx dependency is example-only
in this snapshot. Future Runlimit/Runledger/harness integrations must be thin and
optional where practical. Applications own domain errors, authorization, trusted
identity interpretation, schema migration policy, and product workflows.

Runlimit retains admission/storage semantics. Runledger retains durable queue,
workflow, leasing/retry/schedule logic and internal supervision. The harness
retains PostgreSQL provisioning, template cloning, admission, and cleanup.
None of those upstream crates should depend back on Batter.

Consequences: there is no single mandatory "everything" stack. Integration work
requires actual API-version verification and tests rather than guessed adapters.
No second outbox, limiter backend, job runtime, database harness, or universal
repository is permitted merely to make the foundation appear complete.

Amendment, 2026-09-08: HTTP infrastructure failures may use an application-owned
renderer receiving typed failure plus a request-parts snapshot. Trusted metadata
must be established outside the adapter; callback output sanitization and domain
mapping remain application responsibilities. The default Problem JSON is retained.

Packaging amendment, 2026-09-08: [ADR-006](006-workspace-packages.md) selects a
separate `batter-axum` package instead of a foundation feature and moves SQLx
to its own unpublished example package. This leaves upstream ownership and
the combined request-policy contract unchanged. The PostgreSQL harness remains
external and is not a workspace dependency.
