# Import native Runlimit into Batter
Owning Bead: batter-isdr.1. Fixed PINNED_BASE: ab4bae198eabb426450f803d3d9638b9248d656a. Epic batter-isdr; source master 12e035dac504a1d348c2058ee7ade8e61f2e7974. This PR is stacked on Runledger PR #7.

## Progress
Task selected from ready tracker state; clean worktree and immutable base recorded. Import implemented. Both full verification matrices and all ten HTTP smoke profiles pass; native PostgreSQL tests pass on PostgreSQL 16.15 with default/all features on both toolchains. All five required Jig targets pass, including api:test receipt receipt_01M32XM1RZNVXWPG300TGSW867. Native cumulative review of the committed full task diff completed with no findings. Post-commit gates pass; final api:test receipt receipt_01M32Y6N9T027M9G726SEXF9TH. Task complete.

## Surprises & Discoveries
The first complete matrix reached its new native batch after all workspace tests and doctests passed, then the bounded process runner rejected five commands. Split that batch into four commands plus a separate release pass and added a batch-capacity assertion; runner controls pass. Native behavior and SQL remain unchanged. Minimum-toolchain Clippy requires a native configuration entry recognizing the PostgreSQL product name, plus narrowly scoped compatibility for the newer unused_async_trait_impl lint whose existing waiver preserves first-poll deferral. Only two Rust files gain lint-compatibility annotations; async bodies and original prose remain unchanged.

The existing adapter already selects this exact Runlimit master revision. The import changes source ownership and verification, not native runtime semantics. Five packages preserve 0.3.0 except runlimit-postgres 0.3.1, Rust 1.94, and MIT OR Apache-2.0. Native PostgreSQL tests require an explicit disposable database URL and are ignored in ordinary Cargo tests. Upstream also requires a release-mode fail-closed invariant test and default-feature checks.

## Decision Log
Import native packages to runlimit/runlimit-* as root members, without a nested workspace. Preserve native strict lint policy and exact source algorithms/migrations. Make all packages unpublished. Native core stays free of Tokio/HTTP/SQLx/Batter; native packages cannot depend on Batter. Root dependencies select local native paths; existing consumer probes must also use them. Preserve migration files byte-for-byte and record checksums in import provenance. Keep PostgreSQL provisioning external; execute live tests against a disposable fixture and retain equivalent CI coverage. Omit upstream release automation/backlog/administration; retain licenses, changelog, source examples and relevant developer guidance. Exported consumer proof is task batter-isdr.2.

## Outcomes & Retrospective
Both mutation oracles pass (six scheduling originals and six rejected mutants; HTTP originals and premature-close/producer-only mutants). Graph and asset controls (6), bounded-runner controls (23), and source archive/tool integration controls (11) pass. Source ZIP SQL and license bytes match imported provenance. No hosted execution or fresh-agent usability claim is made without executed evidence.

## Context and orientation
Cargo.toml owns workspace dependencies; crates/batter-runlimit owns operational composition while runlimit/* owns native policy/storage/HTTP behavior. scripts/test_matrix.py and feature-isolation helpers own compiled graph checks. scripts/package.py and mutation-copy tools own exported fixtures. .jig.toml and .agent/jig-contract.json must declare all new native inputs together. Upstream snapshot is available read-only at /tmp/batter-runlimit-upstream.

## Plan of work
Copy five native packages and licenses into runlimit/. Adapt inherited manifest fields and paths without changing features, versions or native ownership. Generate Cargo.lock using Cargo. Replace Git source declarations and pinned consumer fixture declarations with local workspace identities. Add graph and migration/license controls, source export inventory checks and exact inherited file-budget ceilings. Preserve default/all-feature/release/live upstream verification in root tools and CI. Update ADR, root/native/adapter guides, status and compatibility contracts.

## Concrete steps
Implement within this task only. Run formatting, relevant native and adapter tests, strict Clippy and rustdoc, both full verify.sh matrices, and five built HTTP smoke profiles per supported toolchain. Run native ignored PostgreSQL tests with default and all features against a disposable database; preserve original semantic assertions. Run source archive and mutation-copy controls. Inspect and refresh required Jig gates. Commit implementation, verify clean including untracked files, then codex review --base ab4bae198eabb426450f803d3d9638b9248d656a. Append every finding to the external mktemp findings file; fix root causes in new commits and repeat cumulative review against the same base. Close only after clean review and passing validation; closure metadata gets its own commit.

## Validation and acceptance
Exactly five native workspace identities, unpublished, no native-to-Batter dependency and no runtime/HTTP/database dependency from native core. Root foundation remains adapter-free. Migrations and licenses match source provenance. Existing isolation tests and independent negative graph/assets controls execute. Both Rust versions pass and live PostgreSQL evidence records actual server version. API invalid-state review: no new Rust API or operational composition is introduced; source migration retains existing canonical and explicitly low-level boundaries.

## Idempotence and recovery
Preserve the original dirty API-hardening worktree and upstream repositories. Revert only task commits for rollback; no database migration rewrite, publication, source repository archive or automatic retry of uncertain native effects. Keep the findings file outside Git; print its current summary and delete it before any early exit. Escalate architectural redesign or recurrent findings instead of silently expanding scope.

## Interfaces and dependencies
Native ownership and facade optionality are unchanged. Task batter-isdr.2 depends on the reviewed import. Native PostgreSQL owns migrations and maintenance; application authorities, credentials, proxy policy and response meaning remain outside Runlimit.
