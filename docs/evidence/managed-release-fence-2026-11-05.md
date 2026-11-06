# Managed-object release fence — 2026-11-05

Owning task: `batter-ofom`. Inventory families Q05/Q07 remain partly implicit;
this slice removes the older-release overwrite path through managed apply.
The design and its limits are in [ADR-013](../adr/013-managed-database-objects.md#release-ordering-and-legacy-adoption--2026-11-05).

## Behavior and invalid-state review

Every `ManagedObjectManifest` requires an opaque positive `ManagedRevision`.
The locked ledger authorizes managed DDL before execution; a private
`AuthorizedRelease` borrows the same manifest used for mutation and recording.
An existing versioned ledger cannot become a fresh install by deleting its rows.
Application callbacks cannot alter revision evidence and receive a successful
receipt. The content digest remains independent of the release number.

| Requested state | Result |
| --- | --- |
| Higher revision | Apply and publish revision in the managed transaction |
| Same revision and digest | Reapply, repairing drift if needed |
| Same revision, different digest | `RevisionConflict`, no managed DDL/callback |
| Lower revision | `Downgrade`, no managed DDL/callback |
| Legacy five-column ledger, matching recorded digest | Adopt baseline transactionally, adding the sixth column |
| Legacy ledger, different/missing digest | `LegacyBaselineRequired` |
| Versioned ledger, missing/invalid revision evidence | `InvalidLedger` |

Number assignment and compatibility between running application versions remain
application policy. An intentional content rollback needs a higher number.
SQLx migrations still run first and may commit before a refusal. Administrative
DDL and ledger resets are outside this fence. The returned revision is evidence
of the acknowledged transaction, not a permanent claim about remote state.

## PostgreSQL evidence

Executed on Linux, pinned Rust 1.98.1, SQLx 0.9.0 and a disposable external
PostgreSQL **18.6 (Debian 18.6-1.pgdg13+2)** container. The crate provisions no
server. No macOS execution is claimed.

```sh
cargo test -p batter-sqlx --test managed_objects_live --locked -- \
  --ignored --test-threads=1
```

With `BATTER_SQLX_ADMIN_URL` set by the external harness: **52 passed**.
The four release cases cover downgrade/conflict refusal without mutation or
callback invocation, same-release reapply, revision-only advance, a waiting
older writer, explicit rollback under a newer number, atomic baseline adoption,
and missing/invalid metadata. The existing cancellation case also checks the
retained revision after observing actual backend termination. Callback integrity
now includes direct revision mutation. A further assertion checks that identical
content at a different revision yields exactly the revision-mismatch finding.

The first broad run found four existing setups that reused older manifests or
copied a ledger with a different revision. They now explicitly model successive
releases (or copy the expected revision); their original trigger and locking
assertions are retained. The implementation was not weakened to accept those
setups.

## Actual older-writer compatibility

Executed two temporary client packages against the same disposable database:

- old: unmodified `batter-sqlx` and `batter-core` at
  `6ef838865187ad9b788ab7566d8c046aa3eff1f0` (the SQLx tree equals PR base
  `b3bb2edb4b03062efe7206c9e9f36a7e470f4e16`);
- new: this change, using the required revision constructor argument.

Both clients used `PgLease::migrate_and_apply`, an empty `Migrator`,
`PublicExecute::Revoke`, one `release_value()` SQL function, and the same
application-named ledger. Each call acquired a fresh lease under a 30-second
operation context. Builds used Rust 1.98.1 and SQLx 0.9.0.

| Step | Executed result |
| --- | --- |
| Old client installs function returning 10 | Success; real five-column ledger created |
| New client adopts identical source at revision 1 | Success; function still returns 10 |
| New client applies revision 2, returning 20 | Success; function returns 20 |
| Old client reapplies its original source | `ManagedApplyError::LedgerShape`; function still returns 20 |

This checks the actual pre-fence shape validator, beyond the live test's
transactional five-to-six-column assertions. Client sources and execution logs
were local temporary artifacts; the permanent regression cases are in
`crates/batter-sqlx/tests/managed_objects_live/releases.rs`.

## Public API comparison and migration

Used the repository's [surface extractor](public-api-diff-2026-10-09/surface.py)
on old/new rustdoc JSON generated with the same nightly:
`cargo 1.101.0-nightly (29c5daa1a 2026-10-07)` and
`rustdoc 1.101.0-nightly (32dba69d6 2026-10-09)`.

```sh
# At each revision, retain target/doc/batter_sqlx.json in old-doc/new-doc.
cargo +nightly rustdoc -p batter-sqlx --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  old-doc new-doc batter-sqlx
```

[Saved result](managed-release-fence-api-2026-11-05.txt): 1,347 to 1,378 entries,
zero removed and 31 added. Moving the manifest implementation preserves its
public path. Manual comparison of the JSON signatures additionally identifies
`ManagedObjectManifest::new` gaining the required first `revision` argument;
the path inventory alone cannot detect signature changes.

The two added exhaustive variants are also source-breaking:
`ManagedApplyError::Release` and `FindingKind::ManagedObjectRevisionMismatch`.
The [changelog](../../CHANGELOG.md) contains before/after call shapes and shim
decisions for all three cuts; replacement rustdoc contains `Migration:` notes.
No default revision shim can preserve the fence, and no shim can make old
exhaustive matches handle new outcomes. Compile-fail rustdoc rejects omission
of the revision. The canonical PostgreSQL lifecycle example supplies it.

Full pinned workspace verification is recorded in the owning Bead and PR.
Fresh-agent consumer evaluation is proposed and unexecuted.
