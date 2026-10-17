# Publication evidence dossier, 2026-10-17 (`batter-ddc`)

This dossier assembles the evidence an owner needs for a go/no-go decision on
publishing the release train to crates.io. It performs no publication, name
reservation, push or deployment. Permission to publish is a separate owner
instruction. Every executed check below ran on Linux x86_64 with the pinned
Rust 1.98.1 toolchain on 2026-10-17 unless a row says otherwise.

## Revision and versions

| Item | Value |
|---|---|
| Repository revision | `master` at `2e4aa19d1504966df894b054aa85ab9a1cf69ca2` (merge of PR #59, 2026-10-17) |
| `Cargo.lock` SHA-256 at that revision | `2c7579814d4d8b9f9d53eb0c0ceae60b3391efe17767c890b3af2d7be2d199c6` |
| `Cargo.lock` SHA-256 after the rustls update below | `cf5ce7b483dbdee63d8c16563bade2b9fb5c19d1cb4ad8cf6af134af46eef630` |
| Batter packages (9) | `batter`, `batter-at-rest`, `batter-axum`, `batter-core`, `batter-otlp`, `batter-runledger`, `batter-runlimit`, `batter-sqlx`, `batter-test-support`, all `0.0.1` |
| Native Runledger packages (5) | `runledger-core`, `runledger-postgres`, `runledger-runtime`, `runledger-test-support`, `runledger-tui`, all `0.13.0` |
| Native Runlimit packages (5) | `runlimit-axum`, `runlimit-core`, `runlimit-http`, `runlimit-memory`, `runlimit-postgres`, all `0.4.0` |
| Unpublished packages | `batter-example-postgres-lifecycle`, `batter-example-reference-service` (`publish = false`) |
| Minimum Rust version | every publishable manifest declares `rust-version = "1.94"` |
| Publication targets | all 19 publishable manifests declare `publish = ["crates-io"]` |

## Gate summary

| Gate | State | Evidence |
|---|---|---|
| Crate-name availability | Satisfied | crates.io API, 2026-10-17: all nine Batter names return no crate. |
| Existing-name ownership | Satisfied | `runledger-*` 0.12.0 and `runlimit-*` 0.3.x on crates.io are owned by the repository owner's registry account; so is `postgres-test-harness`. |
| License and attribution | Satisfied | See [Licenses](#licenses). |
| Dependency advisories | Satisfied after one lock update | Five rustls advisories at `2e4aa19`; zero after updating rustls to 0.23.46. One upstream-bound unsound warning remains; see [Supply chain](#supply-chain). |
| Dependency licenses | Satisfied | Inventory contains only permissive licenses; see [Licenses](#licenses). |
| Dependency sources | Unresolved | `batter-sqlx` declares `postgres-test-harness` by Git revision; the published 0.2.0 differs from that revision. See [Harness source](#harness-source-mismatch); `batter-ddc.1`. |
| CI-action supply chain | Satisfied | All workflow actions are pinned to commit SHAs. |
| MSRV and locked CI | Satisfied | Hosted CI runs Rust 1.94.0 and 1.98.1 with `--locked` and passed on `2e4aa19`. |
| Packaging | Satisfied for leaves, unrehearsable for dependents | See [Packaging](#packaging-and-publish-order). |
| Private vulnerability reporting | Unresolved, owner action | GitHub reports `{"enabled": false}` for the repository on 2026-10-17; `batter-ddc.2`. |
| Threat model | Recorded | See [Threat model](#threat-model-and-reporting-channel). |
| Two-consumer evidence for promoted APIs | Unresolved | Owned by `batter-hq3`; see [Consumer evidence](#consumer-evidence). |
| Remaining P0 work | Satisfied | No open P0 issue in the tracker on 2026-10-17. |
| Local verification after the lock update | Satisfied locally | `scripts/verify.sh` exit 0 on Rust 1.98.1; hosted CI not yet run on the update. See [Verification](#verification). |

## Names and ownership

Queried through the crates.io API on 2026-10-17 with a descriptive user agent.
`cargo info <name>` is not usable for this check inside the workspace because it
resolves the local package first.

| Name | Registry state |
|---|---|
| nine `batter*` names | absent |
| `runledger-core`, `-postgres`, `-runtime`, `-test-support`, `-tui` | present at 0.12.0 (updated 2026-08-26) |
| `runlimit-core`, `-memory`, `-http`, `-axum` | present at 0.3.0 (updated 2026-08-24) |
| `runlimit-postgres` | present at 0.3.1 (updated 2026-09-05) |
| `postgres-test-harness` | present at 0.2.0 |

The owners endpoint lists the same single user account for `runledger-core`,
`runlimit-core` and `postgres-test-harness`. The publishing account must hold
that ownership when the train runs.

## Licenses

| Packages | Declared license | License files shipped |
|---|---|---|
| eight Batter packages | `MIT` through `license.workspace = true` | root `LICENSE` (MIT, "Batter contributors") |
| `batter-at-rest` | `MIT` in its own manifest | `crates/batter-at-rest/LICENSE` |
| five Runledger packages | `MIT` | `LICENSE` in each package directory |
| five Runlimit packages | `MIT OR Apache-2.0` | `LICENSE-MIT` and `LICENSE-APACHE` in each package directory |

Dependency inventory from `cargo deny list -l license` over the locked graph
(476 packages, all targets): Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC,
Zlib, Unlicense, BSL-1.0, Unicode-3.0, CDLA-Permissive-2.0 (`webpki-roots`
root-store data) and Apache-2.0 WITH LLVM-exception. `LGPL-2.1-or-later` appears
only as the third option of `r-efi`'s `MIT OR Apache-2.0 OR LGPL-2.1-or-later`
expression. No copyleft-only package is present. The inventory includes
upstream target-specific packages for non-Unix targets; they do not broaden
this project's platform support (ADR-007).

## Supply chain

### Advisories

`cargo audit` against the RustSec database (1334 advisories, commit
`64f92cdc64`) at revision `2e4aa19`:

| Advisory | Package | Patched in | Reached through |
|---|---|---|---|
| RUSTSEC-2026-0285 | rustls 0.23.44 | >= 0.23.45 | `sqlx-core` (every SQLx consumer), `reqwest` (`batter-otlp`), `bollard` (`runledger-test-support`) |
| RUSTSEC-2026-0346 | rustls 0.23.44 | >= 0.23.46 | same |
| RUSTSEC-2026-0347 | rustls 0.23.44 | >= 0.23.46 | same |
| RUSTSEC-2026-0348 | rustls 0.23.44 | >= 0.23.46 | same |
| RUSTSEC-2026-0349 | rustls 0.23.44 | >= 0.23.46 | same |
| RUSTSEC-2026-0205 (unsound) | scc 2.4.0 | >= 3.8.4 | `metrics-exporter-otel` 0.3.1 (`batter-otlp` only) |

`sqlx-core` requires `rustls ^0.23.24` and `reqwest` requires `^0.23.4`, so
`cargo update -p rustls --precise 0.23.46` is a lock-only change. It was applied
in this working tree; the lock diff is the rustls version and checksum lines
only. A repeat audit reports zero vulnerabilities and the single unsound
warning. That warning cannot be resolved by a lock update: `metrics-exporter-otel`
0.3.1 requires `scc ^2.1`, and the patched line is 3.8.x. The affected path is
`Array::insert` when a caller-supplied comparison function panics; Batter's
exporter does not supply such a function, but this is an upstream dependency
decision, not a local proof. Tracked as `batter-ddc.3`.

### Duplicate versions and sources

`cargo deny check advisories bans sources` with a minimal configuration
(`multiple-versions = "warn"`, unknown registries and Git sources denied):
bans pass with 18 crates present at two or three versions; sources fail on one
Git dependency, `postgres-test-harness`, described next.

### Harness source mismatch

`crates/batter-sqlx/Cargo.toml` declares
`postgres-test-harness = { version = "0.2.0", git = ..., rev = "3d525e6f..." }`
as an optional test-support dependency. On publication Cargo strips the Git
source and the registry version 0.2.0 is used. The published 0.2.0 crate was
downloaded and compared with the pinned revision: `src/harness.rs` and
`src/fingerprint.rs` differ (292 changed lines), the revision is eight commits
past 0.2.0 and adds `DatabaseTemplate::derive`, derived fingerprints and a fix
retaining template sources through detached copy workers. Batter's own
test-support code uses only items that exist in 0.2.0, so a published
`batter-sqlx` compiles, but its test-support feature would run harness behavior
that the local evidence never executed. Resolution is an owner action in the
harness repository: publish the pinned revision as a new version and change
the declaration to a registry version only. Tracked as `batter-ddc.1`.

### CI actions

Every `uses:` in `.github/workflows` is pinned to a commit SHA with a version
comment: `actions/checkout` (v4.2.2 and v7.0.0), `Swatinem/rust-cache` (v2),
`actions/cache` (v4.3.0).

## CI and MSRV evidence

`ci.yml` runs both verification jobs on Rust 1.94.0 and 1.98.1 with `--locked`,
and a weekly schedule checks floating stable. On revision `2e4aa19` both hosted
workflows passed:

- Rust verification: <https://github.com/bpcakes/batter/actions/runs/39007244854>
- Repository policy: <https://github.com/bpcakes/batter/actions/runs/39007244858>

These runs predate the rustls lock update; the update must pass the same CI
before publication.

## Packaging and publish order

`cargo package --locked --no-verify --list` succeeded for all 19 publishable
packages with no errors or warnings. `cargo publish --locked --dry-run` succeeded
for the four leaves whose dependencies are all on the registry already:
`batter-at-rest`, `batter-test-support`, `runledger-core`, `runlimit-core`.

A dry run of `batter-core` fails with `no matching package named
batter-test-support found`: a versioned development dependency must exist on the
registry before its dependent can even be packaged. Dependents therefore cannot
be rehearsed against crates.io before the earlier tiers are uploaded, and an
upload cannot be undone. A private registry rehearsal would remove that risk;
none was performed.

Dependency order from workspace edges (normal and build), with the two
same-tier constraints that development dependencies add:

1. `batter-test-support`, `batter-at-rest`, `runledger-test-support`,
   `runledger-core`, `runlimit-core`, then `batter-core` (it dev-depends on
   `batter-test-support`)
2. `batter-axum`, `batter-otlp`, `batter-sqlx`, `runlimit-axum`,
   `runlimit-http`, `runlimit-memory`, then `runlimit-postgres` (it dev-depends
   on `runlimit-memory`)
3. `batter-runlimit`, `runledger-postgres`
4. `runledger-runtime`, `runledger-tui`
5. `batter-runledger`
6. `batter`

Each upload must wait for registry index propagation before the next tier.

## Threat model and reporting channel

Batter has had no dedicated security audit. [SECURITY.md](../../SECURITY.md)
records the boundaries that remain application-owned (authentication,
authorization, proxy trust, tenant isolation, TLS, CSRF/CORS, body and queue
limits, secret loading, migration policy, external-effect idempotency), the
documented limits of bulkheads, HTTP deadlines and finite admission, and the
diagnostics hygiene contract. The threat model for publication is therefore:
consumers receive operational mechanics with explicit non-guarantees, no
embedded credentials or network listeners by default, `unsafe` forbidden in the
library crate roots, and transitive dependencies that the consumer must audit
on their own target. Nothing in the packages claims to be a security control.

SECURITY.md requires an owner-controlled private vulnerability reporting channel
before publication and forbids inventing a contact. The GitHub repository's
private vulnerability reporting was disabled on 2026-10-17 (API read). Enabling
it is a one-time owner action; the exact command is recorded in `batter-ddc.2`.
Until it is enabled, this gate blocks a go decision.

## Consumer evidence

The acceptance criteria require two real consumers to exercise partial startup
and shutdown and API ergonomics before stable adapter promotion. The read-only
two-consumer survey of 2026-09-27 and the whole-application fresh-agent
baseline ([2026-10-09](whole-app-baseline-2026-10-09/README.md)) cover API
ergonomics for the facade path. The partial-startup and shutdown failure
collection is the explicit scope of `batter-hq3`, which is open. This dossier
records that gate as unresolved; it does not substitute survey evidence for it.

## Verification

Local `bash scripts/verify.sh` on Linux x86_64 with Rust 1.98.1 after the rustls
lock update, started 2026-10-17T10:50:28Z and finished 11:07:41Z with exit 0:
formatting, file budgets, all four Clippy configurations, all six test-matrix
parts (3755 tests passed, 0 failed), rustdoc with warnings denied, and every
HTTP smoke profile passed. The hosted CI matrix, including Rust 1.94.0, must
also pass on the committed lock update; that run has not happened.

## Not done or unverified

- No crate was uploaded; no docs.rs build has ever run for these packages.
- No private-registry rehearsal of tiers 2 to 6.
- The two unpublished example packages were not packaged.
- The advisory database snapshot and name checks are dated 2026-10-17 and must
  be repeated on the day of publication.
