# Runlimit import provenance

Epic: `batter-isdr`. Source: latest fetched master
[`12e035dac504a1d348c2058ee7ade8e61f2e7974`](https://github.com/bpcakes/runlimit/tree/12e035dac504a1d348c2058ee7ade8e61f2e7974),
which was already the exact native revision selected by Batter.

All five packages moved from upstream `crates/` to `runlimit/`. Native Rust and SQL
behavior, tests, fixtures, package versions, dual licenses and strict lint settings
are retained. Both in-memory `Limiter` impls suppress `clippy::unused_async_trait_impl`
per method, with a reason: each async body defers the synchronous check to first
poll, which the `Limiter` contract requires. The import's impl-wide unknown-lint
allowances and its `doc-valid-idents` override served Rust 1.94 and 1.98.1; both
were removed once the pinned toolchain became the declared minimum
(`batter-39wm.1`). Root Cargo resolves one local identity per native package and owns
the generated lock. Manifests make inherited metadata explicit, inherit local native
dependencies, adapt README paths and disable publishing. Native behavior, public Rust
contracts and persisted schema are unchanged by this import.

`upstream-assets.json` retains SHA-256 digests of imported SQL and root licenses.
The workspace check rejects changes/removal; it permits new forward migrations.
A superseded imported statement is replaced by a new file rather than edited:
since `batter-z4m9`, fixed-window cleanup executes
`runlimit-postgres/src/cleanup_expired_by_key.sql`, which identifies expired rows
by primary key, and the imported `cleanup_expired.sql`, which read `ctid`, is
retained byte-identical as a provenance record that no code path executes.
Since `batter-ctk7.8`, attempt completion likewise executes
`runlimit-postgres/src/attempt_complete_scheduled.sql`, which reads the failure
delay from a schedule bound by Rust, and the imported `attempt_complete.sql`,
which restated the backoff formula, is retained byte-identical and unexecuted.
Both records live in [`provenance/runlimit-postgres/`](provenance/README.md),
outside every package. The workspace check requires both superseded records,
by digest, to be inventoried only under canonical `provenance/` paths, every
other file there except the README and OS metadata to be inventoried, and no
symlink in or at the directory. It then packages and verifies each native
crate with `cargo package --all-features`, which builds every library from its
own archive where the records are absent, and rejects an archive member, under
any name, that carries a record's bytes (`batter-39wm.2`). Cargo does not
build packaged test targets from the archive; that pre-existing gap is
`batter-88bd`.
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
