# Protected shared profiled pool

Owner: `batter-nhu5`. Base: `cd6802a08d2f1a6c102ba55d1e24a9c7d785fd99`.

`batter_sqlx::pool_in` now requires a declared session profile, validates capacity,
constructs the lazy pool and publishes its awaited close finalizer synchronously
under an already reserved `CleanupSlot`. Only then can the caller obtain the
`PgProfiledPool`. Its mandatory SQLx hooks are unchanged. Native options other
than the three session hooks remain intact.

`RunledgerDatabase::from_profiled` validates the existing single-authoritative-schema
rule and takes a shared pool view. It creates no new connections, capacity or
finalizer. The reference root and external single-dependency consumer use this
path; the latter now constructs inside protected startup and receives its database
with the running owner through `finish_with_output`.

Standalone native constructors and arbitrary-hook native construction are explicit
`low_level` entry points. Native tests and standalone tools use those paths and
retain their own cleanup protocols. The finite retirement command retains its
physical-session replacement guard through `low_level::pool_in`. Profiled fixture
hosting remains the separate `batter-7r3.8.1` task.

## Invalid-state review (ADR-010)

- Canonical construction cannot expose a pool before cleanup registration: the
  function consumes the reservation and contains no await or user callback.
- Invalid zero capacity creates no pool or finalizer. Duplicate/invalid cleanup
  names are rejected by reservation before construction. An unstarted initializer
  cannot construct capacity.
- Runledger cannot obtain a capability from a raw pool or a fallback-schema
  profile. Rejected conversion leaves the original registered close intact.
- Sharing adds no second finalizer and all clones share closure. Different roles
  or session policies intentionally require separate pools; types cannot infer
  that application policy.
- The core owner still orders dependent settlement before cleanup, including
  failure and owner loss during startup. Runtime death and detached server-session
  termination remain outside the local guarantee. Native SQL and explicit native
  close remain escape hatches; this is not a SQL sandbox.
- No deprecated shims preserve the old canonical unregistered constructors or
  unprofiled pool path. Existing source must adapt, but no persisted state,
  migration, baseline deployment or coordinated data rollout changes.

## Verification

Executed locally on Linux with pinned Rust 1.98.1:

- Seven `batter-runledger --all-features --test shared_pool` tests pass. These
  exercise drain-before-close, fallback-schema rejection, zero capacity and
  duplicate reservation, initialization error, owner loss, inert construction,
  and a PostgreSQL test of shared physical session/capacity and close waiting
  for an outstanding checkout. The live test uses native Runledger provisioning.
- The existing SQLx live profile/reset test now uses canonical `pool_in` and its
  registered finalizer; its assertions still cover hook replacement and session
  restoration. That selected ignored test passed on disposable PostgreSQL 18.6;
  `/tmp/batter-nhu5-live.log` records the server version and result.
- Workspace all-target/all-feature Clippy passes without warnings. File budgets
  pass without limit changes. Full `scripts/verify.sh` passed (exit 0): all four
  Clippy configurations, all six test-matrix parts, rustdoc and every HTTP smoke
  profile. This includes the migrated external single-dependency consumer with
  real worker execution. Its log is `/tmp/batter-nhu5-verify.log`; the owning Bead
  records the delivered commit. Only verification prose changed after that run.

No macOS or fresh-agent consumer evaluation is claimed. Hosted CI is separate.

## Public API comparison

The comparison uses the repository's rustdoc JSON surface extractor, plus manual
signature review because that extractor compares names/kinds, not signatures.
The old package trees and Cargo graph at clean `e59fe7f` are identical to base
`cd6802a` for the compared packages. Both use the same nightly toolchain.

The [generated surface diff](owned-profiled-pool-api-2026-11-05.txt) confirms cuts: `PgProfiledPool::{connect, connect_lazy}`,
`RunledgerDatabase::{connect, connect_lazy}`, and root `register_pool_close`.
`pool_in` keeps its name but gains the profile argument and returns
`Result<PgProfiledPool, sqlx::Error>`. The unpublished reference helper
`database::configured` additionally takes a cleanup slot. Root/native changelogs
record every before/after shape and the no-shim decision; replacement rustdoc
names the removed APIs in `Migration:` sentences.

`batter-sqlx` has 3 removed and 4 added entries (1378 → 1379);
`runledger-postgres` has 4 removed and 5 added entries (3395 → 3396), including
its prelude reexports. The facade and adapter reexport these same identities.
There are no new exhaustive error variants or persisted-format changes.

Both revisions used cargo 1.101.0-nightly (29c5daa1a, 2026-10-07) and rustdoc
1.101.0-nightly (32dba69d6, 2026-10-09), with `SQLX_OFFLINE=true`:

```sh
cargo +nightly rustdoc -p batter-sqlx --lib --all-features --locked -- \
  -Zunstable-options --output-format json
cargo +nightly rustdoc -p runledger-postgres --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  /tmp/batter-nhu5-api/old /tmp/batter-nhu5-api/new batter-sqlx runledger-postgres
```

Manual signature review covers `pool_in` and the reference helper above. The
source comparison confirms Cargo.lock and every database migration are unchanged.
