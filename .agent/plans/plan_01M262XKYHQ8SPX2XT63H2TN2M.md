# Implement batter-5pm typed root settings

Owning Bead: `batter-5pm`; baseline `0e47f7dbd5d0c04d878d290c5d8c9181d1162b18`.
The living execution plan and complete acceptance audit are
[batter-5pm.md](batter-5pm.md). Beads owns scope and dependencies.

## Progress

- [x] Implement shared sources, bounds and value-hiding diagnostics.
- [x] Implement validated reference native constructors and offline failure tests.
- [x] Adopt configured HTTP router and existing native pool/worker probe seams.
- [x] Pass final Rust 1.98.1 and 1.94.0 verification (729 executions each), ten
  rebuilt HTTP smoke profiles and a real file/environment deadline exercise.
- [x] Refresh final Jig receipts after the added secret-bearing startup test:
  all required targets pass; managed work evidence records the latest api:test
  receipt and final freshness after documentation updates.
- [ ] Execute the exact nineteen-case PostgreSQL inventory on both toolchains.
  POSTGRES_TEST_ADMIN_URL is unset; the user has been asked for an existing
  externally provisioned endpoint. Both explicit invocations exit 1 before tests.

## Surprises & Discoveries

SQLx native URL formatting cannot handle a bare IPv6 host; an actual native
startup/password handshake proves that constructor path instead. Failed Startup
before running transfer remains Draining. The graph has no SQLx TLS backend;
mode retention is tested but TLS negotiation remains unverified.

## Decision Log

Keep source policy and schema in application roots, native types at consumers,
no ambient native PG defaults/passfile, no worker from_env, and no new framework.
Retain the prepared plan's full acceptance, including real PostgreSQL evidence.
Do not close the producer or imply production/downstream adoption from probes.

## Outcomes & Retrospective

Implementation and offline/native HTTP evidence are delivered. Live pool,
worker concurrency and pool/lease startup evidence remain required. The goal,
Bead and this session remain open; do not finish them until the external
prerequisite permits both live runs. No commit/publication/provisioning occurred.

## Verification and recovery

See docs/validation.md for exact versions, commands, lock hash and results.
Inspect current work evidence/gates for this plan before selecting checks.
After endpoint selection run `bash scripts/test_reference_live.sh` on 1.98.1
and 1.94.0, repair any real failures, refresh appropriate validation and current
Jig receipts, then audit every original AC before closing.
