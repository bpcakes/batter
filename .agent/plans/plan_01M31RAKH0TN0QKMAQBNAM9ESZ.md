# Declared PostgreSQL authority across owned scopes

Owning Bead: `batter-ldb`. Companion to Runledger PR 20 and its checked-in
`docs/atomic-integration-repair-plan.md`. No provisioning, publication or merge.

## Progress

- Foundation profile declaration, reset/application/validation and scope retention
  implemented; foundation commit `4dc0889` is available for an immutable pin.
- Runledger adapter and reference startup/submission migrated to the configured
  database type; capacity tests retain the actual pool used by the application.
- PostgreSQL 18.6 role/custom-schema/timeout/tenant parity and poisoning tests pass
  in Runledger. Foundation unit, compile-fail and clippy checks pass.
- Full verification passed on Rust 1.94.0 and 1.98.1. SQLx and adapter live tests,
  all 66 reference cases and both private library probes passed on PostgreSQL 18.6.
- Fresh-agent compile-only exercise passed first attempt; retained upstream at
  `docs/evidence/profile-consumer-2026-09-21.*`. It found a manual-first README
  example, now corrected. All five HTTP profiles passed on both toolchains.
- All five final Jig targets passed against frozen source inputs. Exact-head
  hosted checks remain a PR delivery check, not implied by local receipts.

## Surprises & Discoveries

DISCARD removes after_connect policy. SET ROLE does not load the target role's
login settings. Therefore policy must be declarative and re-established by the
owner, not a caller callback. Profile validation must precede BEGIN for snapshot
inspectors to acquire locks before their first snapshot-bearing query.

## Decision Log

User approved custom schemas and SET ROLE support. `PgSessionProfile` is immutable
policy, not a witness. Its Debug is redacted; identifiers are quoted and settings
bound. Runledger owns mandatory pool hooks and accepts one authoritative ordinary
schema to exclude fallback resolution. The reference application declares direct
login/public policy, retaining its operation-context deadlines and cleanup slot.
Arbitrary SQL remains an escape hatch; this is not a privilege sandbox.

## Outcomes & Retrospective

Implementation and local verification are complete across both workspaces.
Hosted delivery evidence is tracked on the paired PR checks. No merge or
publication is performed; this plan records local verification only.
An early Jig run overlapped source edits and other full builds, invalidating its
receipts and reaching its command budget. Final gates must run against frozen
source inputs; the independent successful Rust/live passes remain scoped evidence.

## Execution and validation

Foundation files: `crates/batter-sqlx/src/{profile,atomic,atomic_runner,snapshot}.rs`.
Consumer files: `crates/batter-runledger` and `examples/reference-service`.
Run `bash scripts/verify.sh` on Rust 1.98.1 and 1.94.0; run SQLx, Runledger adapter
and reference live tests on dedicated PostgreSQL 18 clusters; execute required
Jig gates with this plan ID after freezing inputs. HTTP smoke remains required.
The companion Runledger suite tests role/schema/parameter parity and rejected
conflict rollback. Pin coordinated sources, push feature branches and update PRs;
do not merge or publish. Errors retire resources; no data migration needs rollback.
