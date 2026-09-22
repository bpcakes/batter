# Runledger source import

Owning Beads: `batter-biqv`, `batter-biqv.1`, `batter-biqv.2`.
Imported on 2026-09-21 from the then-current remote `master` of
[bpcakes/runledger](https://github.com/bpcakes/runledger/tree/46b5cd085d011e597de9552dfebbed4c19416453),
full revision `46b5cd085d011e597de9552dfebbed4c19416453`.
The source repository's Git history is retained there; this is a source snapshot
import, not a rewritten or fabricated Git history. Its MIT license is preserved
in this directory and in all five packages.

All five package implementations, tests, examples, migrations, SQLx caches and
current operational documents are retained. Three historical consumer audits that
cite private application paths remain linked at their immutable upstream source
instead of becoming broken local references. Old tracker/planning artifacts, repository
CI, release automation and paired-checkout smoke/attestation machinery are not
active in Batter. The original revision remains the reference for those artifacts.
Shared-workspace consumer verification and SQLx refresh are maintained by
`scripts/check_runledger_consumer.py` and `scripts/refresh_runledger_sqlx.py` at the
Batter root; README snippet checks live in the shared graph/asset checker.
Historical research, migration notes and evidence under `docs/` and CHANGELOG
retain their original scope and do not establish verification of this import.

Adaptations: local root-workspace dependencies, explicit 0.12.0 package versions,
retained native lint/features, local repository metadata, shared Cargo.lock,
Jig/CI integration, updated current adoption/development guidance, and removal
of the foundation Git attestation and its obsolete tests. The native build scripts
still check canonical migration copies. Python graph tests now reject remote or
duplicate foundation sources, facade dependency inversions and asset drift.
Two `mod.rs` files use the equivalent named-module layout required by Batter.
Four atomic wrapper methods and six internal scheduler functions retain their concrete public error types with
reasoned local large-error lint allowances; the now-included strict workspace
Clippy pass exposed their existing 128–160-byte errors. Native runtime, persistence
and public API behavior are not redesigned here.

The root defaults remain Batter's existing adoption graph; enabling the Runledger
facade feature selects the native packages. Runledger's TUI and test harness are
separate packages, not dependencies of ordinary foundation consumers. Every
package remains unpublished. No source repository archival, release, deployment,
or application database migration accompanies this import.
