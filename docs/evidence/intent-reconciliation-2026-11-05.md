# Owned intent reconciliation

Owner: `batter-hw3k`. Inventory families: D03, using existing Q02/Q03 owners.
Base: `b3bb2edb4b03062efe7206c9e9f36a7e470f4e16`.

`runledger-postgres/src/atomic/reconciliation.rs` now owns one-query correlation
of an intent, matching promoted job and current definition. All four atomic
scopes expose the read, with an exact `JobScope`. Opaque retained fields and
`Promoted(CorrelatedJob)` encode the correlation invariant. Invalid links fail
closed; absence remains ambiguous and never authorizes replay.

Both public single-intent getters are removed, including jobs/prelude exports.
The [breaking migration](../../runledger/CHANGELOG.md) covers both call shapes.
No deprecated shim: the old signatures bypass the owner and expose full records.
Administrative list and general job APIs serve separate purposes. No migration,
persisted format, promotion lock order or retention policy changes are included.
The new grant selection is additive; schema and old deployed binaries can coexist.

## Verification

Focused native PostgreSQL tests passed: 8 grant tests and 4 reconciliation tests,
on PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2). Coverage includes restricted column
access, own writes, rollback, lost actual COMMIT acknowledgement, same-identity
idempotent replay, concurrent promotion and retention, and invalid scoped links.
The existing malformed-stage promotion regression now reads the healthy intent
through this exact-ID API: an unrelated malformed full record cannot poison its
metadata lookup. Its focused PostgreSQL rerun passed.

The complete `CARGO_TARGET_DIR=/home/aa/Documents/batter/target bash scripts/verify.sh`
passed on Linux x86_64, Rust/Cargo 1.98.1: formatting, unchanged file-budget limits,
all four Clippy configurations, all six test-matrix parts, warning-denied rustdoc
and all five HTTP smoke profiles. This includes both native policy runner modes,
the existing intent/promotion/runtime suites, compile-fail removal checks and the
external single-facade consumer. The final run followed repairs to test assertion
messages and the malformed-row test migration; no tests or limits were weakened.
The local log is `/tmp/batter-hw3k-verify.log`. Hosted CI is separate evidence.
No macOS execution or fresh-agent evaluation is claimed.

## Public API comparison

The [rustdoc surface diff](intent-reconciliation-api-2026-11-05.txt) reports
8 removed entries in `runledger-postgres`: the two removed getter functions and
their `jobs`/`prelude` exports. It adds 79 entries for the new types, their
methods/variants/traits, scope methods and grant variant. `batter-runledger`
adds five reexports and removes none. No other existing call signature changes.

Both revisions used cargo 1.101.0-nightly (29c5daa1a, 2026-10-07) and rustdoc
1.101.0-nightly (32dba69d6, 2026-10-09), with `SQLX_OFFLINE=true`:

```sh
cargo +nightly rustdoc -p runledger-postgres --lib --all-features --locked -- \
  -Zunstable-options --output-format json
cargo +nightly rustdoc -p batter-runledger --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  /tmp/batter-hw3k-api/old /tmp/batter-hw3k-api/new \
  runledger-postgres batter-runledger
```

The old JSON was built from the clean `6ef8388` worktree; its package trees are
identical to base `b3bb2ed`: `e339ecf08c58ad167624e762dbdfb0efbddd00fc`
for `runledger-postgres`, and `461348e9b73640d4e28717dd54a4780da5b077ae`
for `batter-runledger`. The new JSON was built from this change. The surface
extractor compares named public items, not a general semantic compatibility proof;
the reviewed source diff additionally confirms migrations and Cargo.lock unchanged.
