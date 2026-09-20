# Acknowledged atomic workflows and enforced phases

Owning Bead: batter-gzh. Coordinated Runledger Bead:
runledger-runledger-simplification-audit-eor. Scope: the five confirmed findings
on Batter PR 2 at bb285062, plus the consuming Runledger integration in PR 20.

## Progress

- Implemented the canonical runner, phase cutover, session reset/retirement,
  original-guard snapshot cleanup and distinct read-only capability.
- Migrated the facade checks, reference service, producer example and external
  consumers. No legacy alias or borrowed-view bridge remains.
- Passed PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2): 81 SQLx cases, the Batter
  Runledger integration, 66 reference cases plus two additional controls, 25
  Runledger migrations, the new intent/queue/app atomicity test, external source
  and packaged smoke (nine each), and the producer/worker round trip.
- Passed Rust 1.94 verify.sh; Rust 1.98 matrix, Clippy, fmt and Jig gates in
  run_01M30CQHJBKVWD1WRBKYV55EG6; strict rustdoc and all five HTTP smokes.
- Final paired revision pins, test diagnostic lint repair, fresh final evidence
  and PR updates remain to finish.

## Surprises & Discoveries

DISCARD ALL invalidates server prepared statements, so normalization clears the
SQLx driver cache first. Snapshot inspectors must still acquire authoritative
locks before their first snapshot query. Retiring a client is not synchronous
backend termination; live tests separately observe eventual advisory-lock release.
Runledger lint requires diagnostic expect messages, including in new test code.

## Decision Log

The foundation owns run_atomic, retaining the body result until acknowledged
commit or rollback. Scope interruption removes the internal owner before polling
SQL so catching cancellation cannot resurrect it. PgAtomicError retains uncertain
output/rejection. Only low_level exposes exceptional consuming-owner composition.

Runledger owns a PgIntentScope consumed into PgQueueScope; recording has no method
on the queue phase. Arbitrary SQL against internal tables is an explicitly
lower-level escape hatch, not a claimed SQL sandbox. All atomic/snapshot paths
retire their sessions, including acknowledged completion. Acquisition resets
inherited session settings, resources and driver statement caches.

## Validation and recovery

Run targeted Rustdoc compile-fail and positive tests; scripts/sqlx_live.py on a
dedicated PostgreSQL 18 database; scripts/reference_live.py using two distinct
PG18 clusters; paired Runledger scripts/lint.sh and external-consumer smoke;
both toolchain verification matrices and five HTTP smokes; final Jig gates.
Repair failures without weakening semantic assertions. Keep both source trees
unchanged during gate execution and validate peer inputs before reusing receipts.

No persisted migrations, payload changes, package publication or merges. Update
the existing feature branches and PRs after validation, retaining uncertainty
rather than automatically replaying any failed database operation.

## Outcomes & Retrospective

The confirmed causes were library/adapter API gaps, not consumer misuse.
The runner removes separately paired output/completion from the canonical path;
phase consumption removes the inverse named lock order. Compile-fail and live
adversarial scenarios are executed evidence, not a fresh-agent usability study.
Final delivery and paired revision identities will be recorded in the owning Beads.
