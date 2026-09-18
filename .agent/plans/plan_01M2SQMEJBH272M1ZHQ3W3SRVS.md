# Protected quota API hardening (batter-rbq)

This ExecPlan implements Bead `batter-rbq` against the current dirty workspace.
The baseline is the current Git HEAD plus existing uncommitted Runlimit delivery;
do not reset or discard unrelated work. The package is unpublished, so update
the in-repository consumers in one coordinated cutover.

## Progress

- [x] Replace positional public probes with named opt-in and prove guarded routes stay guarded.
- [x] Install a private-constructor authenticated principal type and migrate handlers/tests.
- [x] Narrow public admission/interruption results while retaining native detail.
- [x] Check auth and selector bounds at construction; compile a fresh external consumer.
- [x] Classify unknown future denial kinds without claiming storage capacity.
- [x] Update contracts and evidence; run both toolchains, HTTP smokes and Jig gates.

## Surprises & Discoveries

- The first new guarded-default test failed because its fixture treated the
  presence of an empty Authorization header as authenticated. The test fixture
  now requires the exact generic token; no production behavior was changed.
- The exact Runlimit checkout already validates allowed batches and exposes a
  consuming decision vector. `AllowedBatch` keeps that vector without another
  allocation and projects only scalar allowed metadata to callers.
- A disposable external consumer compiled with an unannotated subject selector.
  Invalid auth and selector signatures both failed at `HttpQuota::new` with
  E0631, confirming the small constructor-bound change suffices.
- The first full Rust 1.98.1 verification reached strict Clippy and identified
  `unwrap_or_else(Router::new)` as `unwrap_or_default`. The default was changed;
  the full gate passed on both toolchains.
- The initial Jig plan check passed test, Clippy, formatting and contract gates,
  then failed the 800-line budget for `tests/http.rs` at 858 lines. Moving two
  existing tests to a child module brought the parent to 775 lines; the same
  18 HTTP tests and a targeted file-budget rerun passed.

## Decision Log

- Keep native Runlimit decisions and denial details lossless. No backend behavior,
  retry, refund or database lifecycle work is added.
- Public probes default to none; explicit opt-in is the sole unguarded route input.
- `Authenticated<P>` has a private constructor and its own Axum extractor.
  Raw `Extension<P>` remains unrelated application metadata; canonical handlers
  use the new extractor. This avoids deleting unrelated values of type `P`.

## Outcomes & Retrospective

- Protected routes now default to guarded; public probes require named opt-in.
  Handlers use `Authenticated<P>`, and public result variants contain only
  reachable quota states while retaining native details.
- Both `verify.sh` toolchains, all ten HTTP smoke profiles, both runnable quota
  examples, and the fresh external consumer compile check passed.
- Jig's final `api:test` receipt is
  `receipt_01M2SS0KQ9CHGAMEWFXVHZX90V`; the four sibling targets also passed.
  The native wildcard denial is only statically checkable at the pinned revision.

## Context and plan of work

`crates/batter-runlimit/src/http.rs` owns HTTP assembly and authentication;
`quota.rs` owns the result type and native check boundary;
`crates/batter-axum/src/quota_observation.rs` owns sanitized HTTP facts.
The finite example and `tests/http.rs` are consumer contracts. Update them and
`tests/quota.rs` alongside source edits. Contract prose lives in
`docs/integrations.md`, `docs/guarantees.md`, `docs/testing.md` and
`docs/status.md`; executed evidence belongs in `docs/validation.md`.

Use exact pinned upstream Runlimit decision views. Keep `BatchDecision` details
available through narrowed admitted/rejected values without allowing the opposite
outcome in either result. `OperationContext::run` interruption permits only
not-started or in-flight check states. A fresh consumer must be a temporary
standalone Cargo project outside this workspace, import the public API, compile
one valid call without hand-annotated principal/selector argument types, and
reject a wrong auth or selector signature at `HttpQuota::new`.

## Validation and recovery

Run focused Axum/Runlimit tests and doctests after source changes. Then run
`bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
Rebuild and execute all five `docs/testing.md` HTTP smoke modes on both fixed
toolchains, plus the Runlimit example. Run Jig work check/evidence/gates/finish
and inspect the fresh `api:test` receipt. Update validation only with executed
outcomes. If any gate fails, retain its evidence, repair the cause and rerun the
affected gate. No commit, publication or deployment is authorized.
