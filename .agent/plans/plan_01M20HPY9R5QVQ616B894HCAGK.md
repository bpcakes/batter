# Enforce subprocess deadline wiring and capture failure coverage

Owning Bead: `batter-fvz`. Baseline: `e5f2f04b2dbb349d08085caf177f662fcbc89812`
plus all existing staged and unstaged work. This changes private tests only;
no dependency, library API or application behavior change is intended.

## Progress

- Review confirmed the pure deadline tests do not enforce the driver's use of
  that deadline. Capture I/O error and panic paths need direct coverage.
- Baseline passed all 37 entries. The expanded focused suite passes 41.
- Both blocked scenarios now assert the actual wait deadline; three reader tests
  cover retry/EOF, I/O failure, panic, partial output and destruction before join
  returns. Focused Clippy passes.
- Fixed-wait, swallowed-read-error and swallowed-reader-panic mutations each
  failed their dedicated assertion (exit 101); exact sources were restored.
- Both full toolchain matrices and all three rebuilt HTTP smoke modes passed
  on Linux and macOS. All 67 source/build file hashes match on both hosts.
- Updated contracts, test counts, implemented status and validation evidence.
  All five Jig targets passed; work evidence/gates report the required verify
  gate fresh (receipt `receipt_01M20J1NPBE4X6D0CKJFZ3JHDG`). Final
  `scripts/jig check test`, package inspection and diff checks passed.
- Implementation and verification are complete; Jig finish and the owning Bead
  carry delivery closure.

## Surprises & Discoveries

`FixtureChild::wait_for_scenario` already uses the correct computed deadline.
A live test can compare its recorded selected deadline with captured drain time,
without depending on delayed startup. Capture already owns the reader thread;
accepting an ordinary `Read + Send + 'static` input lets tests exercise the same
read loop and join path, including failure after partial output.

## Decision Log

Keep deadline policy unchanged. Add one real-process test for both blocked
scenarios, asserting captured drain plus observation equals the deadline actually
passed to wait. Retain existing milestone and termination checks. Script reader
inputs (interruption, evidence bytes, then EOF/error/panic) and observe input
Drop through a channel to establish reader destruction before finish returns.
No clock framework, extra processes for reader failures, or new dependencies.

## Outcomes & Retrospective

Implementation is complete. The computed-deadline regression fails when
`wait(limit)` becomes `wait(EXIT_LIMIT)`; the other mutations fail because
`finish()` incorrectly returns Ok. No scheduling-sensitive startup delay was
added. Linux passes 147 foundation / 168 workspace entries plus two doctests; macOS
passes 145 foundation / 166 workspace entries plus two doctests. Both toolchains
pass compilation, formatting, Clippy, rustdoc and tests. Exact environments,
source hash and command logs are recorded in `docs/validation.md`. All five repository gates and the final backend test passed. The source/build
hashes still match the tested snapshot, and all mutation backups match restored
source. Nothing was committed or published.

## Steps and validation

1. Run the existing focused suite, then add deadline wiring and capture failure
   tests under `crates/batter/tests/non_yielding/`.
2. Temporarily replace the computed wait deadline with `EXIT_LIMIT`; require the
   new live assertion to fail. Temporarily swallow a reader I/O error and a
   reader panic; require their controls to fail. Restore exact source bytes
   after each mutation, including if a command fails.
3. Run `bash scripts/verify.sh` and
   `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` on Linux and the authorized
   `aa@torque.local` macOS host. Use an isolated remote directory and verify
   transferred source hashes. Build the HTTP example with Rust 1.98.1 and run
   default, SIGINT and deadline smoke modes on both hosts.
4. Update guarantees, testing counts, implemented status, validation and Bead
   acceptance/evidence. Run package inspection and diff checks, Jig work
   check/evidence/gates, then finish backend verification with
   `scripts/jig check test`. Finish this plan and close the owning Bead only
   after all required work passes. Do not commit or publish.

Logs and temporary mutation backups belong in ignored
`.agent/tmp/batter-fvz-wiring/`. Preserve earlier plans and append-only Jig state.
