# Owned execution cutover

Compared `986d294c24610bbbec8809a76bdf5f78d2031a49` with the PR 106 implementation
for `batter-f9kq.1` and `batter-f9kq.2`. The complete kind-qualified surface
comparison is [surface.txt](surface.txt).

Rustdoc JSON was generated for `batter-core` and `batter-runledger`, with all
features and the checked-in lockfile, using `rustc 1.101.0-nightly
(32dba69d6 2026-10-09)`. Nightly is used only for API extraction; implementation
verification uses the repository's pinned Rust 1.98.1. Each revision used a
separate Cargo target directory to avoid sharing local-package artifacts.

```sh
cargo +nightly rustdoc -p batter-core --lib --all-features --locked -- \
  -Zunstable-options --output-format json
cargo +nightly rustdoc -p batter-runledger --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  OLD_DOC_DIR NEW_DOC_DIR batter-core batter-runledger
```

The only removed API names are:

- `batter_core::admission::Bulkhead::enter`, also exposed by the facade.
- `batter_runledger::job_phases`, also exposed by `batter::runledger`.
  Rustdoc records this as both a function and its re-export.

The replacement methods are `Bulkhead::run` and `run_resolved`; native handlers
use `PhasedJob` with `PhasedJobHandler`. The adapter-only borrowing driver is
new under `operation::low_level`; it does not re-export either removed API.

The extractor compares paths, not signatures or feature semantics. A source
diff review additionally checked the existing public methods, retained
`JobPhasesRejection`, test-support API and facade re-exports: no other public
signature was cut. The `test-support` feature remains; JSON payload support is
now an ordinary dependency of the adapter, already present in the locked graph.
Native Runledger APIs and persisted job payloads are unchanged.

[CHANGELOG](../../../CHANGELOG.md) supplies before/after migrations for raw
async permits, blocking and process-task handoffs, and both manual and owned
job-phase call shapes. Replacement rustdocs contain `Migration:` entries for
the removed names. No deprecated shim is kept: exposing a raw permit restores
premature release, and exposing the native phase pair restores skipped or
reordered finalization. Compile-fail examples reject both removed entrypoints.
