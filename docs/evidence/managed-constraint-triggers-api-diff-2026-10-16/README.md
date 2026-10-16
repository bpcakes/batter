# Public API surface diff for managed constraint triggers, 2026-10-16

Owning task: `batter-n2s1`. This record checks the constraint-trigger change
against a public-API surface diff, as the repository's hard-cut rule requires,
using the method and `surface.py` from
[`public-api-diff-2026-10-09`](../public-api-diff-2026-10-09/README.md).

## Method

- old: `38a09a1` (`origin/master` when the branch was cut), extracted with
  `git archive` into a scratch directory;
- new: this branch, after the implementation, example and documentation
  changes.

No nightly toolchain was installed, so rustdoc JSON (format version 60) was
produced by the pinned Rust 1.98.1 rustdoc with `RUSTC_BOOTSTRAP=1`, each
revision into its own target directory:

```sh
RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=<target-dir> cargo rustdoc -p <package> --lib \
  --all-features --locked -- -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py <old-doc-dir> <new-doc-dir> \
  batter-sqlx batter
```

The JSON format is unstable; the walk was correct for this toolchain's output.
The limits recorded for the original method apply unchanged: entries are
compared by path and kind, so a changed signature on a kept name is not
detected. No existing signature was edited by this change.

## Results

| Package | Entries old to new | Removed | Classification |
| --- | --- | --- | --- |
| batter-sqlx | 1288 to 1292 | 0 | Four additions. `ManagedObject::constraint_trigger` and `Retirement::constraint_trigger` are new methods. `ManagedObjectKind::ConstraintTrigger` is a variant added to an existing exhaustive enum, which breaks exhaustive matches; `CHANGELOG.md` has the `Breaking:` entry with before/after code and the shim decision, and the enum's rustdoc carries a `Migration:` note. The `use batter_sqlx::managed::ManagedObjectError <- error::ManagedObjectError` entry records that the enum's definition moved into the private `managed::error` module to keep `managed.rs` within its file budget; checked by hand, the public path `batter_sqlx::managed::ManagedObjectError`, its variants and its trait impls are unchanged in both revisions. |
| batter | 17 to 17 | 0 | The `sqlx` namespace re-exports `batter_sqlx::*`, so the additions are reachable as `batter::sqlx::managed::...` without a facade entry change. |

The full output for both packages is [`surface.txt`](surface.txt).
