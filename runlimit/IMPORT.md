# Runlimit import provenance

Epic: `batter-isdr`. Source: latest fetched master
[`12e035dac504a1d348c2058ee7ade8e61f2e7974`](https://github.com/bpcakes/runlimit/tree/12e035dac504a1d348c2058ee7ade8e61f2e7974),
which was already the exact native revision selected by Batter.

All five packages moved from upstream `crates/` to `runlimit/`. Native Rust and SQL
behavior, tests, fixtures, package versions, dual licenses and strict lint settings
are retained. Two Rust files have narrowly scoped unknown-lint allowances for a newer Clippy
lint already waived upstream. This preserves first-poll deferral on Rust 1.94 and
1.98.1 without changing async bodies. Native Clippy configuration recognizes the
PostgreSQL product name on the older compiler while retaining the default valid
identifier list and every semantic lint. Root Cargo resolves one local identity per native package and owns
the generated lock. Manifests make inherited metadata explicit, inherit local native
dependencies, adapt README paths and disable publishing. Native behavior, public Rust
contracts and persisted schema are unchanged by this import.

`upstream-assets.json` retains SHA-256 digests of imported SQL and root licenses.
The workspace check rejects changes/removal; it permits new forward migrations.
Source archives retain SQL and both license names, and mutation fixtures include
this native root. Root verification preserves default/all-feature tests and lint,
the release-mode fail-closed regression, and CI's explicit native PostgreSQL tests.

The native smoke source is retained unchanged at `smoke/native_consumer.rs`.
`python3 scripts/check_runlimit_consumer.py` executes it and the added facade
quota fixture from exported sources with a standalone Cargo workspace, no Git
metadata or patches, and the selected compiler (`batter-isdr.2`). Upstream release scripts, Cargo.lock,
CI/deny configuration, tracker, backlog and execution-plan administration remain
historical upstream records. This unpublished workspace does not run registry
release or upstream repository archival. README developer paths and agent guidance
are adapted to the shared root; the changelog is retained as historical evidence.

[ADR-012](../docs/adr/012-native-runlimit-workspace.md) preserves native ownership.
Execution results belong to the owning Beads and root status/testing pages.
