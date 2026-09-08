# Preserve startup evidence across later panic diagnostics

Owning Bead: `batter-fvz`. Baseline `e5f2f04b2dbb349d08085caf177f662fcbc89812`
plus all pre-existing staged/unstaged work. Private test-harness and evidence
changes only. No commit, push or publication.

## Progress

- Confirmed the stale Linux-only status sentence and capture-wide panic check
  occurring before lookup of previously captured startup evidence.
- Baseline passed 41 entries. Regressions failed the old code: four failures
  included the actual live panic-before-startup-resolution path (exit 101).
- Capture bounds now precede event lookup independently of final panic checks.
  Missing startup still rejects panic promptly, late startup still fails, and
  final validation still rejects every panic. All 44 focused entries pass.
- Two mutations removing final panic rejection and startup overflow rejection
  each failed (exit 101); exact source bytes were restored.
- Both full toolchain matrices and all three rebuilt HTTP smoke modes passed on
  Linux and macOS. All 67 source/build file hashes match on both hosts.
- All five Jig targets passed. Work evidence/gates report required verify fresh
  (receipt `receipt_01M20KHTNDCEPWNQZ78BT3JY5J`). Final
  `scripts/jig check test`, package inspection and diff checks passed.
- Implementation and verification are complete; plan finish and the owning Bead
  record delivery closure.
- Corrected the stale status limits and updated contracts/test descriptions.

## Surprises & Discoveries

An on-time drain followed by the fixture's deliberate panic is valid timing
input but unsuccessful final evidence. Conflating these decisions makes the
negative control depend on polling before the panic. The earlier tests
encoded the combined expectation; they now distinguish the two outcomes.

## Decision Log

Keep capture overflow fail-closed before all event lookups. Preserve prompt
panic rejection when startup is missing; a late event remains a deadline error.
Resolve captured on-time events independently of later panic diagnostics, and
retain unconditional panic rejection during final validation. Add deterministic
clock cases plus a live test that synchronizes on captured panic output before
its first startup-resolution call, with no near-deadline launch sleep.

## Outcomes & Retrospective

Linux passes 150 foundation / 171 workspace entries and two doctests; macOS
passes 148 foundation / 169 workspace entries and two doctests, using both
Rust 1.94.0 and 1.98.1. Formatting, compilation, Clippy and rustdoc pass as well.
Exact commands, hashes and host limits are in `docs/validation.md`.

Corrected the implemented-status limits to agree with
Linux/macOS host results, the unexecuted updated hosted CI job, and Linux-only
orphan-reaping probes. No attempt to emulate Linux subreaping on macOS is needed.

## Steps and verification

1. Establish the 41-entry baseline. Add regressions, confirm the old startup
   check fails them, then separate capacity and timing/diagnostic decisions.
2. Verify on-time/panic/late/missing/overflow combinations, original deadline
   wiring, and final rejection. Run focused tests and Clippy.
3. Run both supported Rust toolchains through `scripts/verify.sh` and rebuild
   HTTP default/SIGINT/deadline smokes on Linux and authorized `aa@torque.local`.
   Use an isolated remote directory and source manifest; retain exact command,
   host, versions and results in `docs/validation.md`.
4. Update guarantees, testing, implemented status and owning Bead. Run package
   inspection, diff checks, Jig work check/evidence/gates and final
   `scripts/jig check test`; close plan and Bead only when complete.

Logs and temporary files go under ignored `.agent/tmp/batter-fvz-panic/`.
Restore exact original bytes after any mutation, even on command failure.
Preserve prior plans and append-only repository state.
