# Automate HTTP observation verification and close composition test gaps

Owning Bead: batter-faj.5. Address the completed review while preserving all prior
uncommitted changes, public APIs, dependency versions and observation behavior.

## Progress

- [x] Read the completed review, repository guidance, current tests and CI wiring.
- [x] Add CI profiles and document the loopback prerequisite.
- [x] Strengthen composition, filtering, middleware-order, unwind and live readiness tests.
- [x] Run focused checks, both Rust matrices, five HTTP smokes and Jig gates; update evidence and close work.

## Surprises & Discoveries

- The resolved Axum 0.8.9 Router route_layer layers matched path endpoints, including their method fallback. Starting/Draining therefore return admission 503 for POST on a guarded GET route; unrecognized paths remain unguarded 404.
- The test helper permits a filter without changing global tracing state. Use it to assert enabled and suppressed events against the same actual example router.

## Decision Log

- Keep loopback tests mandatory. State their prerequisites instead of marking them ignored or adding a feature switch.
- Add filtered smoke profiles to Linux CI and adapter tests plus all smoke profiles to macOS CI. Keep hosted/macOS execution claims separate from workflow configuration.
- Preserve response severity metadata through nested observers. Test observer order and inherited metadata explicitly without adding a consuming extension API.
- Broader streaming/disconnect/shutdown semantics remain owned by batter-u0m.

## Outcomes & Retrospective

Both review findings and the bounded test gaps are addressed. Rust 1.98.1 and
1.94.0 each passed 391 test/doctest executions, with formatting, Clippy and rustdoc
clean. The rebuilt HTTP example passed all five smoke modes. Workflow YAML parsed
and shell syntax passed. All five tracked verify targets passed; evidence/gates
reported fresh inputs. The final scripts/jig check test also passed 391 executions.
Logs and final gate reports are in .agent/tmp/batter-faj.5; docs/validation.md
records exact commands and limitations. CI config now includes macOS adapter
runtime/HTTP coverage, but hosted/macOS execution remains unverified. No runtime
implementation, public API or dependency changed. Changes remain uncommitted.

## Context and orientation

The HTTP observer is in crates/batter-axum/src/lib.rs. Its observation tests are
split by behavior under tests/observation; the private example router's live test
is examples/http_service/tests.rs. .github/workflows/ci.yml owns Linux and macOS
commands. scripts/verify.sh runs example tests through all-targets, so it requires
loopback socket permission as well as the existing subprocess/Python prerequisites.

## Plan of work

Add the missing CI commands and correct verification documentation. Extend the
existing lifecycle test for guarded method fallback and newly added routes in
all phases. Add a small composition-edge test module for filtered overrides,
outer response rewriting and nested observer inheritance. Require the original
identity on the handler resource's unwind event. Reuse the live readiness driver
under INFO and mixed filters, checking response IDs against individual captured
events, including expected absence of filtered INFO events.

## Validation and acceptance

Run cargo test -p batter-axum --locked and the example test, adapter Clippy, both
scripts/verify.sh toolchains, rebuild the HTTP example and run all five smoke modes.
Check shell syntax and workflow YAML where tooling is available. Document exact
executed counts in docs/validation.md. Finish with fresh Jig profile/evidence/gates
and scripts/jig check test. No tests are weakened or silently skipped.

## Recovery and boundaries

Keep failed attempts as evidence. Use approved execution outside the sandbox for
loopback/process checks. Do not edit sources while tracked gates run. No commits,
publishing, remote execution or downstream changes are authorized by this task.
