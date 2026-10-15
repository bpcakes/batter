# Public API surface diff for managed database objects, 2026-10-15

Owning task: `batter-54d0`. This record checks the managed-object change
against a public-API surface diff, as the repository's hard-cut rule requires,
using the method and `surface.py` from
[`public-api-diff-2026-10-09`](../public-api-diff-2026-10-09/README.md).

## Method

- old: `c4ffaf4` (`origin/master` when the branch was cut), in a detached
  worktree;
- new: this branch, after the implementation and documentation commits.

No nightly toolchain was installed, so rustdoc JSON was produced by the pinned
Rust 1.98.1 rustdoc with `RUSTC_BOOTSTRAP=1`:

```sh
RUSTC_BOOTSTRAP=1 cargo rustdoc -p <package> --lib --all-features --locked -- \
  -Zunstable-options --output-format json
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
| batter-sqlx | 1153 to 1287 | 0 | 134 additions. All are new items in the new `managed` module, `PgLease::migrate_and_apply`, `VerificationPlan::{managed_objects, with_managed_objects}` and `verification::verify_managed_objects`, except 13 variants added to existing exhaustive enums: nine `FindingKind`, two `PlanError`, one `SupportedSurface` and one `UnsupportedSurface`. Those break exhaustive matches; `CHANGELOG.md` has the `Breaking:` entry with before/after code and the shim decision, and each enum's rustdoc carries a `Migration:` note. |
| batter | 17 to 17 | 0 | The `sqlx` namespace re-exports `batter_sqlx::*`, so the new module is reachable as `batter::sqlx::managed` without a facade entry change. |

The full output for both packages is [`surface.txt`](surface.txt).
