# Public API surface diff for the browser credential gate, 2026-10-17

Owning task: `batter-52s3`. This record checks the credential-gate change
against a public-API surface diff, as the repository's hard-cut rule requires,
using the method and `surface.py` from
[`public-api-diff-2026-10-09`](../public-api-diff-2026-10-09/README.md).

## Method

- old: `9aea0f5` (`origin/master` when the branch was cut), extracted with
  `git archive` into a scratch directory;
- new: this branch, after the implementation, test and documentation changes.

No nightly toolchain was installed, so rustdoc JSON was produced by the pinned
Rust 1.98.1 rustdoc with `RUSTC_BOOTSTRAP=1`, each revision into its own target
directory:

```sh
RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=<target-dir> cargo rustdoc -p <package> --lib \
  --all-features --locked -- -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py <old-doc-dir> <new-doc-dir> \
  batter-axum batter
```

The JSON format is unstable; the walk was correct for this toolchain's output.
The limits recorded for the original method apply unchanged: entries are
compared by path and kind, so a changed signature on a kept name is not
detected.

## Results

| Package | Entries old to new | Removed | Classification |
| --- | --- | --- | --- |
| batter-axum | 563 to 590 | 2 | Both removed entries are `browser::read_cookie`: the function and its re-export. It is a hard cut with no shim; `CHANGELOG.md` has the `Breaking:` entry with before/after code, and `BrowserCredentials::cookie`, `low_level::read_unchecked_cookie` and the `browser` module carry `Migration:` notes. The 29 additions are `MutationCheckWaiver` with its two variants, `browser::BrowserCredentials` with `cookie` and its `FromRequestParts` and `Debug` impls, `browser::BrowserCredentialsRejection` with its two variants and impls, their re-exports, and `low_level::read_unchecked_cookie`. |
| batter | 17 to 17 | 0 | The `axum` namespace re-exports `batter_axum::*`, so the cut and the additions reach `batter::axum::...` without a facade entry change. |

Checked by hand, because the walk cannot see it: `BrowserPolicy::without_mutation_checks`
kept its name and changed signature from `(PrivateResponsePolicy) -> Self` at
`9aea0f5` to `(PrivateResponsePolicy, MutationCheckWaiver) -> Self`. Its
before/after call shape and shim decision are in the same `CHANGELOG.md` entry,
its rustdoc carries a `Migration:` note, and a compile-fail rustdoc rejects the
one-argument form. No other existing signature was edited.

The full output for both packages is [`surface.txt`](surface.txt).
