# Error handling corrections

Owning Bead: batter-ey0. Baseline: 9aa2f8006000d460011638a7c9f12ace75e907ca.

## Progress

- Implemented the four user-approved review corrections and targeted regressions.
- Updated guarantees, testing, implemented status, example guidance and primary references.
- Both Rust verification matrices and both five-profile HTTP smoke runs passed.
- Final Jig gates and closure are recorded in this plan's receipts.

## Surprises & Discoveries

The receipt retained Arc<E> but its derived source chain omitted concrete E;
the report already exposed E. Result-returning main printed raw VarError Debug.
Targeted receipt and executable regressions failed before the changes and pass now.

## Decision Log

Keep public variants and shared error ownership. Implement the receipt Error
trait manually. Keep SQLx failures and reports intact until an explicit sanitized
ExitCode boundary. Keep HTTP logging errors private with retained native sources
and sanitized Display/Debug. Mark reports must_use with compiler regressions.
No dependency changes, commits or publication are in scope.

## Outcomes & Retrospective

All four corrections are implemented. Rust 1.98.1 and 1.94.0 each passed 433
test/doctest executions with zero failures, ignored tests or warnings; both
builds passed five HTTP smoke profiles. Invalid HTTP logging executable checks
passed too. docs/validation.md records exact scope and limits; no database or
new macOS/hosted execution is claimed. The lockfile and public variants are unchanged.

## Execution and validation

Files: lifecycle/process.rs and process_ownership/error_sources.rs own concrete
receipt sources; cleanup.rs and lifecycle.rs own report diagnostics; the SQLx
example main and tests/diagnostics.rs own exit redaction; the HTTP example and
http_service/logging.rs own filter rejection. Retain all existing semantic tests.
Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh,
then build the HTTP example and execute the five smoke profiles in docs/testing.md.
Update docs/validation.md with actual platform, lock hash and results. Run
scripts/jig work check, evidence, gates and finish for this plan; finish backend
verification with scripts/jig check test. On failure repair the implementation
or test cause without weakening assertions. Beads owns delivery acceptance.
