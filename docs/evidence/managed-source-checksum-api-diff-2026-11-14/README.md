# Public API surface diff for the set-wide source checksum rule, 2026-11-14

Owning task: `batter-6d1g`. This record checks the hard cut that replaces the
per-call `statement_start` with `SourceChecksum` against a public-API surface
diff, using the method and `surface.py` from
[`public-api-diff-2026-10-09`](../public-api-diff-2026-10-09/README.md).

## Method

- old: `6e4d926c` (`origin/master`), extracted with `git archive` into a
  scratch directory;
- new: this branch's working tree, after the implementation, example and
  documentation changes.

rustdoc JSON was produced by the pinned Rust 1.98.1 rustdoc with
`RUSTC_BOOTSTRAP=1`, each revision into its own target directory:

```sh
RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=<target-dir> cargo rustdoc -p batter-sqlx --lib \
  --all-features --locked -- -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py <old-doc-dir> <new-doc-dir> batter-sqlx
```

The method compares entries by path and kind, so a changed signature on a kept
name is not detected. One such change was checked by hand:
`ManagedObjectManifest::new` gains a required `source_checksum: SourceChecksum`
parameter after `public_execute`.

## Results

| Package | Entries old to new | Removed | Classification |
| --- | --- | --- | --- |
| batter-sqlx | 1379 to 1391 | 1 | `ManagedObject::checksum` is removed; `CHANGELOG.md` has the `Breaking:` entry with before/after code and the shim decision, and `ManagedObjectManifest::object_checksums` carries the `Migration:` sentence. The additions are the `SourceChecksum` enum, its two variants, its derived impls and re-export, and `ManagedObjectManifest::{source_checksum, object_checksums}`. `statement_start` appears in neither revision: it existed only on this unmerged branch. |
| batter | not regenerated | 0 | The `sqlx` namespace re-exports `batter_sqlx::*`, so the change is reachable as `batter::sqlx::managed::...` without a facade entry change. |

The full output for `batter-sqlx` is [`surface.txt`](surface.txt).
