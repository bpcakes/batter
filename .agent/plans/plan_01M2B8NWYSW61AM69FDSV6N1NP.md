# Repair SQLx ownership and verification oracles

This ExecPlan is a living document governed by `.agent/PLANS.md`. Its owning
delivery task is reopened Bead `batter-lp2.3`. It repairs test-evidence defects
found by the all-reviewer review of commit
`b87708db06640fa9fd81f6d15cd2f2cfa538f7be`; the completed delivery plan remains
historical.

## Purpose / Big Picture

The owned-pool API already publishes cleanup synchronously, but its live suite
must fail for the named contract violation rather than for any convenient error.
Authentication evidence must be an exact server password rejection. Pool-close
evidence must prove awaited completion beyond `is_closed()`. Reservation cases
must exercise the real fallible composition path, and matrix controls must prove
that failure in every actual batch and command stops later work.

## Progress

- [x] (2026-09-12 16:36Z) Reopened and claimed `batter-lp2.3`; recorded the ADR-010 recurring-defect assessment.
- [x] (2026-09-12 17:40Z) Rechecked PostgreSQL 18 authentication rules/error codes, SQLx 0.9 pool-close behavior, and the local external fixture.
- [x] (2026-09-12 18:02Z) Replaced generic authentication and pool-close checks with violation-specific oracles.
- [x] (2026-09-12 18:02Z) Made invalid/duplicate reservation and held-checkout tests exercise causal production-shaped paths.
- [x] (2026-09-12 18:05Z) Covered every command in every actual verification batch and updated runner controls.
- [x] (2026-09-12 18:28Z) Ran offline/live, two-toolchain, all ten HTTP smokes and fresh Jig verification; recorded validation evidence.

## Surprises & Discoveries

- Observation: PostgreSQL `trust` performs no password check, and the first
  matching `pg_hba.conf` rule decides authentication without fallback. SQLSTATE
  `28000` is generic invalid authorization, while `28P01` is invalid password.
  Evidence: PostgreSQL 18 primary authentication and error-code documentation.
- Observation: the ordinary local Unix-socket URL succeeds without a password;
  the same role over local TCP with a deliberately wrong password reaches a real
  server password rejection. The unprivileged test role cannot inspect
  `pg_hba_file_rules`, so endpoint policy must remain an explicit runner input.
- Observation: SQLx sets a pool's closed flag before all checked-out connections
  return. `is_closed()` is therefore an initiation witness, not completion.
- Observation: the matrix has actual batch sizes `[4, 1, 3, 1]`, while the old
  negative controls exercised only the first two calls and returned surplus
  outcomes that the mocked runner did not validate.

## Decision Log

- Decision: Require `BATTER_SQLX_AUTH_REJECT_URL` in the live runner and assert
  exact PostgreSQL SQLSTATE `28P01`.
  Rationale: SQLx exposes the server error, not a stable public proof of the
  negotiated HBA method. A separately provisioned endpoint plus exact invalid-
  password result is the strongest read-only evidence available without adding
  provisioning or privileged server inspection.
  Date/Author: 2026-09-12 / Codex.
- Decision: Split cleanup-record validation from native close-completion
  validation. Require zero size and `PoolClosed` on a later acquisition; for held
  checkouts also prove the owner task remains unfinished before release.
  Rationale: this independently detects premature `is_closed()` success.
  Date/Author: 2026-09-12 / Codex.
- Decision: Route invalid and duplicate reservations through `?` directly before
  the construction counter and `pool_in` call.
  Rationale: the observed command failure then comes from the intended
  reservation boundary, and any accidental continuation enters measurable native
  construction.
  Date/Author: 2026-09-12 / Codex.

## Outcomes & Retrospective

No production API change was needed. The live runner now requires an explicit
password-rejecting endpoint and accepts only `28P01`; a deliberate Unix trust /
missing-role run fails the exact assertion. Reservation tests propagate the real
invalid/duplicate result before a measurable constructor entry. Successful pool
completion requires zero size and a later `PoolClosed`, while held checkout and
premature-record controls prove an unfinished close cannot pass. The matrix
control now injects all nine command failures across `[4, 1, 3, 1]`.

Offline controls and all 24 live cases pass on both Rust 1.98.1 and 1.94.0 against
the external PostgreSQL 18.6 fixture. Both complete repository verifiers, all ten
HTTP process smokes, and fresh Jig `api:test`, `api:fmt`, `api:clippy`,
`repo:contract`, and `repo:file-budget` gates pass. No production API change was
needed; the durable mitigation is violation-specific evidence at every boundary.

## Context and Orientation

`crates/batter-sqlx/tests/pool_ownership_live.rs` owns the fourteen ignored pool
ownership scenarios. `scripts/sqlx_live.py` owns their exact inventory and live
prerequisites; `scripts/test_sqlx_live.py` prevents partial output from counting.
`scripts/test_matrix.py` defines four verification batches and
`scripts/test_parallel_process.py` owns its negative controls. Public production
code in `crates/batter-sqlx/src/lib.rs` should not change unless a test exposes a
real adapter defect.

## Plan of Work

Add a separate authentication-rejection connection helper and require its URL in
the Python runner. Preserve the returned `sqlx::Error::Database`, but accept only
`28P01`. Update runner unit tests so either missing URL stops before invocation.

Refactor the live close assertions into exact cleanup-record and native-completion
helpers. Apply both helpers to the two-pool case. In the held-checkout success case,
assert the owner task is unfinished after close initiation and before release.
Build the premature negative control with an otherwise-valid successful cleanup
record so only the unfinished close condition rejects it.

Rewrite invalid and duplicate cases as actual Command flows using `?` at the
reservation boundary, followed by a construction counter and `pool_in`. Require
the exact `RegistrationError` in the retained work result, zero rejected
construction, and only the legitimately registered cleanup record.

Generate matrix negative cases from `[4, 1, 3, 1]`; inject one failure at every
command position and require execution to stop after that batch. Update contracts,
references, status and validation with executed evidence and limitations.

## Concrete Steps

Work from `/home/aa/Documents/batter`.

Run the offline controls first:

    python3 -m unittest discover -s scripts -p 'test_sqlx_live.py' -v
    python3 -m unittest discover -s scripts -p 'test_parallel_process.py' -v
    cargo test -p batter-sqlx --features test-support --locked

Run live evidence with an ordinary external database URL and a separate password-
rejecting URL:

    DATABASE_URL='postgresql://aa@localhost/postgres?host=/var/run/postgresql' \
    BATTER_SQLX_AUTH_REJECT_URL='postgresql://aa:invalid-pool-ownership-password@127.0.0.1:5432/postgres?sslmode=disable' \
      bash scripts/test_sqlx_live.sh

Then run both complete verifiers, all five HTTP smokes on each toolchain, and the
owning Jig gates.

## Validation and Acceptance

The authentication case passes only with exact `28P01`; a trust endpoint,
missing role, connection refusal or generic `28000` cannot satisfy it. The
premature close negative control must have a valid cleanup record and closed flag
yet fail because the close task is unfinished. Two-pool success must prove both
native pools reject later acquisition. Held checkout success must prove the owner
was pending before release. Invalid/duplicate cases must retain the exact
reservation error and zero construction count. Matrix controls must inject nine
distinct failures across batches `[4, 1, 3, 1]`.

## Idempotence and Recovery

All external database operations are read-only connection/query/close checks;
this plan provisions no role, database or HBA rule. A missing password-rejecting
endpoint is incomplete evidence, not a reason to relax the SQLSTATE. Tests and
documentation changes are repeatable. No commit or push is authorized.

## Interfaces and Dependencies

No public Rust API or dependency change is planned. The live runner gains one
required environment variable, `BATTER_SQLX_AUTH_REJECT_URL`. It must identify an
externally provisioned endpoint that rejects its supplied password. `DATABASE_URL`
continues to identify the ordinary disposable PostgreSQL database.

Plan revision note: created after the all-reviewer findings and ADR-010 assessment
to bind each claimed behavior to an independent failure oracle.
