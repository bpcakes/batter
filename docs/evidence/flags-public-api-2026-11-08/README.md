# Flags epic public API surface, 2026-11-08

Owning task: `batter-jkw6.28`. This record checks the flags epic's changes to
existing packages, and its new packages, against a public-API surface diff,
with the method of [the 0.0.1 diff](../public-api-diff-2026-10-09/README.md).
It was regenerated on 2026-11-11, after the branch merged master and after
the LISTEN wake, the notification channel, `manifest_diff` and the public
`Authority` transitions were withdrawn; the first version of this record
compared `fb792dd` with `87eabd0`.

## Method

Both revisions were documented with rustdoc's unstable JSON output on the same
nightly as that record, rustc 1.100.0-nightly (a69a63265 2026-09-03):

- old: `1053293d` (master), in a detached worktree;
- new: the flags branch at `bcf74006` merged with that master.

```sh
cargo +nightly rustdoc -p <package> --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 ../public-api-diff-2026-10-09/surface.py <old-doc-dir> <new-doc-dir> \
  batter-core batter-axum batter-sqlx batter-runledger batter-runlimit batter-otlp \
  batter-outbound-http batter-at-rest batter-test-support batter
```

`existing-packages.txt` is that output, unedited. Because the old revision is
the master the branch merged, every entry is the flags epic's. The same limits
apply as in the 0.0.1 record: entries are compared by path and kind, so a
changed signature on a kept name is not detected.

## Existing packages

| Package | Entries | Flags epic entries |
| --- | --- | --- |
| `batter-core` | 1328 → 1359 | `periodic::register_skippable_in`, `PeriodicOutcome::{Ran, Skipped}` and the field `PeriodicSummary::skipped`; `settings::SettingsSelection::{Captured, Selected}` and `SettingsSource::selection`; `Registration::check_names` and `owns_cleanup`; `cleanup::CleanupRegistration` and `CleanupSlot::register_tracked` |
| `batter-axum` | 590 → 606 | `GroupGate` (`new`, `with_disabled_level`), `Gated::{Serve, Disabled}`, `GateView` (`value`, its extractor), `GroupPolicy::gated` and `browser_gated`, and the variant `HttpFailure::Disabled` |
| `batter-sqlx` | 1379 → 1410 | `PgAtomicTransaction::commit_unless` and `operation_until`, `PgCommitStop::{Interrupted, Unconfirmed}`, `PgDeadlineError::{Elapsed, Failed, RecoveryElapsed}`, `PgLease::migrate_until` with `PgMigrationTimeouts` and `PgMigrationTimeoutError`, `PgReadOnlySnapshot::inspect_until` and `PgProfiledPool::is_registered_in` |
| `batter` | 21 → 23 | the `flags` module re-exporting `batter_flags` |
| `batter-runledger`, `batter-runlimit`, `batter-otlp`, `batter-outbound-http`, `batter-at-rest`, `batter-test-support` | unchanged | none |

No flags-epic change to an existing package removes or renames a public item.
Two additions break source compatibility, and each is recorded with
before/after code in the CHANGELOG: `HttpFailure::Disabled`, a variant of an
exhaustive enum, and `PeriodicSummary::skipped`, a public field of a struct
that is not `#[non_exhaustive]`.

## New packages

| Package | Entries |
| --- | --- |
| `batter-flags-core` | 1238 |
| `batter-flags-postgres` | 535 |
| `batter-flags` | 561 |

The three are new since master, and since 0.0.1, so every entry is an
addition; none was released, so changes made to them within the epic carry no
`Breaking:` entries. The counts are of the same new revision, taken by
importing `surface.py`'s `surface` on its JSON. `batter-flags-cli` is a binary
with no library surface.
