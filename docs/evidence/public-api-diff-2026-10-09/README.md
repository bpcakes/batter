# Public API surface diff, 0.0.1 to 2026-10-09

Owning task: `batter-pasy`. This record checks the backfilled hard-cut notes
against a public-API diff between the 0.0.1 release commit and the head of
master on 2026-10-09, as that task's acceptance requires.

## Method

Both revisions were documented with rustdoc's unstable JSON output on the same
nightly toolchain, cargo 1.100.0-nightly (b2e9d5f9d 2026-09-02) and rustdoc
1.100.0-nightly (a69a63265 2026-09-03):

- old: `f646187` ("release: prepare 0.0.1 publication train"), checked out in
  a detached worktree;
- new: `71e37ea` (master after PR 46).

For every library package in the workspace:

```sh
cargo +nightly rustdoc -p <package> --lib --all-features --locked -- \
  -Zunstable-options --output-format json
```

`surface.py` walks each crate's module tree from its root and records
kind-qualified entries: modules, structs, enums, variants, public fields,
traits and trait items, functions, type aliases, inherent methods, trait impls
other than blanket and synthetic ones, and `use` re-exports keyed by their
source path. It prints the entries present only in the old revision as
`REMOVED` and those present only in the new one as `ADDED`.

```sh
python3 surface.py <old-doc-dir> <new-doc-dir> batter-core batter-axum ...
```

`batter-packages.txt` is the output for the nine Batter packages and
`native-packages.txt` for the nine native Runledger and Runlimit packages.
`batter-otlp` did not exist at 0.0.1 and is listed as present only in the new
revision.

## Limits

- The JSON format is unstable; the walk is correct for this nightly only.
- Entries are compared by path and kind. A changed signature, bound, or return
  type on an item that kept its name is not detected. The sealed-assembly and
  operation-authority entries in `CHANGELOG.md`, and the native `Breaking:`
  entries in `runledger/CHANGELOG.md`, remain the record for those.
  Manual comparison also confirms that `OperationContext::child` returned
  `Result<Self, ConfigurationError>` at `f646187` and returns
  `Result<OperationOwner, ConfigurationError>` now. Its before/after call shape
  and shim decision are recorded in the operation-authority entry; this change
  does not affect the generated entry counts below.
- A `use` entry changes when a re-export's source module moves, even when the
  public path is unchanged. Each such entry below was checked by hand.
- Additions can also break source compatibility: variants added to an
  exhaustive enum invalidate old exhaustive matches, and required public fields
  invalidate old struct literals and patterns. The added variants/fields were
  checked against their historical parent types: `ReadinessUnreadyReason` and
  `JobDeadLetterInfo` need migrations; `BoundaryAssemblyError` was already
  `#[non_exhaustive]` at 0.0.1. The other added variants/fields belong to new types.
- Counts are entries of this walk, not a count of public items.

## Results

| Package | Entries old to new | Removed | Classification |
| --- | --- | --- | --- |
| batter-core | 850 to 1030 | 6 | The operation-authority cut covers `OperationContext::{new, at, under, cancel}`, `OperationAdmission::admit`, `OperationPhases: Clone`, plus the manually checked `child` return type. The added exhaustive `ReadinessUnreadyReason::Condition` variant also breaks old matches. Both have `Breaking:` before/after examples, item-level `Migration:` rustdoc and shim decisions. |
| batter-axum | 457 to 563 | 18 | One cut: eleven helpers moved to `low_level` (twelve entries counting their re-exports) and `AssembledHttp::into_router`. Already recorded as `Breaking:` with before/after code; `low_level` and `in_process` now carry `Migration:` rustdoc. |
| batter-sqlx | 1068 to 1153 | 0 | No removed or renamed public item since 0.0.1. The `PgLease::connection` and `return_to_pool` removal that a downstream consumer recorded happened before 0.0.1. |
| batter-runledger | 22 to 50 | 0 | Additions only. |
| batter-runlimit | 180 to 208 | 0 | Additions only. The denial-accessor change a consumer recorded happened before 0.0.1. |
| batter-at-rest | 137 to 145 | 0 | Additions only. |
| batter-test-support | 21 to 21 | 0 | Unchanged. |
| batter | 16 to 17 | 0 | One added namespace (`otlp`). Facade modules re-export their adapters, so their contents are covered by the adapter rows. |
| runledger-core | 1500 to 1571 | 2 | `jobs::JobDeadLetterInfo` and `jobs::JobDeadLetterReason` moved from `runtime_types` to a `dead_letter` module; both keep their public path through `jobs`. That move is not a cut. The required `origin` field and `JobDeadLetterInfo::new` fourth argument are a separate cut: `runledger/CHANGELOG.md` has `Breaking:` before/after constructor, struct-literal and exhaustive-pattern examples; the type and constructor have `Migration:` rustdoc. |
| runledger-postgres | 3074 to 3152 | 0 | Additions only. |
| runledger-runtime, runledger-test-support | unchanged | 0 | Unchanged. |
| runlimit-core, -memory, -postgres, -http, -axum | unchanged | 0 | Unchanged. |

## Shim decisions

No `#[deprecated]` shim is kept for the identified cuts.

- `OperationContext::new`, `at`, `under`: the core keeps compile-fail doctests
  that reject each constructor, so that every root is created through a visible
  `OperationOwner`. A deprecated constructor would restore root creation with no
  owner, the state the cut removed.
- `OperationContext::cancel`: cancellation authority on a shared handle is the
  invalid state the split removed.
- `OperationContext::child`: no same-name deprecated shim can coexist based on
  return type alone. Restoring the context return would hide child ownership;
  callers explicitly select `into_context()` or retain the owner instead.
- `OperationAdmission::admit`: a deprecated form returning a context would be a
  second root path that hides the owner, for the same reason as above.
- `OperationPhases: Clone`: `cancel_work` is authority; cloning the pair would
  duplicate it.
- `ReadinessUnreadyReason::Condition`: a shim cannot make an old exhaustive
  match accept a new variant. Mapping it to an older reason would mislabel an
  application denial as a lifecycle or dependency failure. The enum stays
  exhaustive so consumers review their response policy.
- The eleven Axum helpers and `into_router`: the sealed-assembly entry already
  records that no root alias or router shim is kept, so an invalid composition
  cannot be restored as a rollback mechanism.
- `JobDeadLetterInfo` / `JobDeadLetterInfo::new`: the former fields/arguments
  do not determine whether the worker or reaper delivers the hook. A default
  origin would mislabel one path, so no deprecated shim is kept. Rust cannot
  overload `new` by arity; a constructor shim also cannot preserve old literals
  or exhaustive patterns. The native changelog records all three migrations.

## Reproduction

```sh
git worktree add --detach /tmp/batter-0.0.1 f646187
for c in batter batter-core batter-axum batter-sqlx batter-runledger batter-runlimit batter-at-rest batter-test-support; do
  (cd /tmp/batter-0.0.1 && CARGO_TARGET_DIR=/tmp/doc-old cargo +nightly rustdoc -p $c --lib --all-features --locked -- -Zunstable-options --output-format json)
  CARGO_TARGET_DIR=/tmp/doc-new cargo +nightly rustdoc -p $c --lib --all-features --locked -- -Zunstable-options --output-format json
done
python3 docs/evidence/public-api-diff-2026-10-09/surface.py /tmp/doc-old/doc /tmp/doc-new/doc \
  batter batter-core batter-axum batter-sqlx batter-runledger batter-runlimit batter-at-rest batter-test-support
```

The nightly toolchain is used only to produce this record. It is not part of
`scripts/verify.sh` or CI.
