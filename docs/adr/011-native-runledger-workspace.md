# ADR-011: Native Runledger in the shared workspace

Status: accepted for the source migration owned by `batter-biqv`.

## Context

The Runledger PostgreSQL package consumes Batter's SQLx foundation. Separate Git
pins plus a foundation-source attestation required coordinated revisions and
consumer-root patches, and prevented testing a changed foundation with its
native consumer in one working tree. The user authorized moving the latest
Runledger master into Batter.

## Decision

Import master `46b5cd085d011e597de9552dfebbed4c19416453` as five distinct root
workspace members under `runledger/`. Preserve package names, version 0.12.0,
strict lint policy, tests, SQL migrations, offline query metadata and licenses.
Root Cargo owns dependency resolution and its generated lockfile. No nested
workspace, source-pin build guard, or consumer patch is needed. All packages
remain unpublished. [Import provenance](../../runledger/IMPORT.md) records the
snapshot and excluded repository administration files.

Native Runledger continues to own durable contracts, persistence, worker
supervision, test databases and the operator TUI. `batter-runledger` remains an
optional adapter. Dependencies flow from native PostgreSQL to `batter-sqlx` to
`batter-core`; no native package depends on the facade. Sharing a repository
does not merge runtime, cleanup or transaction ownership.

## Enforcement and consequences

Cargo graph checks require all five native packages and both foundation packages
to resolve to this workspace with one identity each. Negative controls reject
remote and sibling sources, duplicates, reverse dependencies and publication.
Asset checks compare canonical migrations and query caches with every vendored
copy. Native PostgreSQL 18 tests become part of root verification; Docker is a
prerequisite for the complete default matrix. Required Jig inputs include native
sources, fixtures, caches, migrations and package configuration.

The default facade still has no adapter feature enabled. No public runtime API
or persisted schema changes in this import. Linux/macOS remain the supported
platforms; claims of execution are recorded separately in the owning Beads and
implementation status. Existing upstream file sizes receive exact per-file
ceilings rather than an unrelated source refactor in this migration.

Fresh-agent usability evaluation is proposed and unexecuted. Standalone source
consumer verification and shared-workspace maintenance tools belong to the
second delivery task. The upstream repository is not archived by this decision.

## Publication amendment

On 2026-09-22, `batter-ddc` prepared all five native packages as coordinated
version 0.13.0 crates restricted to crates.io. This semver-minor release signals
the breaking pre-1.0 API changes recorded in the changelog. Internal path
dependencies carry registry versions, while native ownership, migrations,
package identities, and the dependency direction through `batter-sqlx` remain
unchanged. Artifact validation and an explicit upload record are still required
before claiming the release exists on the registry.
