# External consumer sources

Standalone Rust sources compiled by temporary consumer manifests outside this
checkout. They are not workspace members and have no `Cargo.toml` here; they
exist so the manifests those checks generate stay reviewable source in Git.

- `single_facade_consumer.rs` is the acceptance consumer for the
  single-dependency recipe in [reference compatibility](../docs/reference-compatibility.md).
  It must keep declaring only `batter` plus ordinary registry crates: adding any
  direct native-workspace dependency or a `[patch]` section would remove exactly
  the property it proves.
- `single_facade_harness.rs` owns the disposable PostgreSQL 18 database through
  Runledger's existing native test support and runs the consumer against it. It
  is built from its own manifest so the consumer keeps proving it needs no test
  support, and it reaches that support through the facade's
  `runledger-test-support` feature, so it declares only `batter` as well.

`scripts/check_single_facade_consumer.py` builds both from a Git-free source
copy, asserts the consumer's resolved graph, and requires each executed marker.
`scripts/test_single_facade_consumer.py` covers that script's own assertions.
Runlimit's direct-package consumers stay in [`runlimit/smoke`](../runlimit/smoke);
they prove a different contract and still declare native packages on purpose.
