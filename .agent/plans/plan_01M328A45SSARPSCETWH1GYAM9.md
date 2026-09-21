# Add standalone batter-at-rest package

This plan delivers the upstream half of CreditKit Bead `creditkit-platform-at-rest-extraction-v2h.18`: Batter becomes the source owner of the completed synchronous envelope-encryption and stable-MAC leaf crate, while the `batter` facade exposes the identical public types only when its `at-rest` feature is selected. The cryptographic format and application identities do not change.

## Progress

- [x] (2026-09-21) Started Jig plan `plan_01M328A45SSARPSCETWH1GYAM9` at Batter baseline `398ed96e1a3ccd1ac892cedcad505897f0208c92` on branch `feat/batter-at-rest`.
- [x] (2026-09-21) Inspected Batter's workspace/facade policy and CreditKit's completed E17 detached-package evidence.
- [x] (2026-09-21) Transferred the current reviewed leaf, retained byte-identical production source/HKDF salt, added crate-local proprietary legal material, and replaced only upstream test/example scenario data with a generated application-neutral vector.
- [x] (2026-09-21) Added opt-in `at-rest` facade wiring plus graph, disabled-module and direct/facade type-identity checks; default and at-rest isolated consumers pass.
- [x] (2026-09-21) Adopted the user-selected Rust 1.94 minimum explicitly across the leaf manifest, detached gate, CI, and documentation instead of preserving the earlier 1.85 declaration.
- [x] (2026-09-21) Moved the exact Rust 1.94 detached source/package/public-consumer gate into Batter; source, verified package and unpacked artifact all passed, and the Jig contract recognizes the new action.
- [x] (2026-09-21) Synced the feature branch from baseline `398ed96` through current `origin/master` `c3a6304`, preserving the extraction while incorporating the merged Runledger CI pin.
- [x] (2026-09-21) Native review cycle 1 found two portability-gate defects; removed GNU-only `dirname --` usage and made the independent Node.js fixture generator part of the authoritative detached gate. The repaired exact Rust 1.94 gate passed in full.
- [ ] Run focused and plan-associated validation, stage all upstream changes, run native Codex review to convergence, then commit and push the reviewed immutable revision.

Restart checkpoint: upstream implementation, repaired portability gate, leaf Clippy, formatting, and Jig contract pass on current Batter master. The full facade runner reaches the coordinated Runledger foundation-pin guard because adding the workspace member changes root `Cargo.toml`. The next action is native review cycle 2, then commit/push this Batter revision, update the Runledger revision used by this Batter release line to that commit, repin Batter's clean-checkout Runledger revision, and finish the full gates.

## Surprises & Discoveries

- Batter's facade and package split already landed on `master`; the task only needs one additional optional leaf and must not redo the broader `batter-tmx` restructuring.
- The source leaf declared and proved Rust 1.85, but the owner explicitly selected Rust 1.94 for its Batter ownership. The leaf still keeps explicit package metadata and an exact detached 1.94 gate so the deliberate change is tested rather than inferred from workspace inheritance.
- Batter's root is MIT, but the transferred crate is `LicenseRef-CreditKit-Proprietary`. Hosting it here is an explicit crate-level exception, not relicensing authority.
- Runledger deliberately treats Batter's root `Cargo.toml` as a foundation input. Its build accepts companion-only descendants but cannot validate an uncommitted workspace-member change; the source-owner sequence must therefore commit/review Batter first, update Runledger's immutable pin, then record that Runledger revision in a companion-only Batter commit.
- Runledger `master` advanced during the task to a verifier paired with unmerged Batter commit `497e41e`; it is not a valid test fixture for current Batter `master`. Batter's synced CI intentionally remains paired to Runledger `58b02c2`, so the extraction follows that release line's immutable-pin sequence instead of importing unrelated foundation work.
- Batter's pinned Jig updater reported an all-or-nothing conflict but left newer template artifacts in the worktree. Those exact updater artifacts were restored to `HEAD`; only the authored `.jig.toml` action and matching v9 contract entry remain.

## Decision Log

### D-01 — Preserve one leaf implementation and one type identity

- Status: accepted
- Choice: `batter-at-rest` owns the implementation. `batter::at_rest` is an optional `pub use` of that package, and direct consumers import `batter_at_rest`.
- Why: this keeps default Batter consumers free of crypto dependencies, creates no reverse dependency, and prevents a facade copy or compatibility implementation.
- Rejected: duplicating types in the facade or making the leaf depend on `batter`/`batter-core`.

### D-02 — Treat relocation as source ownership, not a format migration

- Status: accepted
- Choice: copy the current reviewed source, tests, generic fixtures, format documentation, and legal files verbatim except for package/import/repository naming. Preserve all v1 magic, HKDF salt/info, AAD roles, limits, errors, and key semantics.
- Why: persisted envelopes and stable identities must remain byte-compatible; E18 authorizes no cryptographic redesign.

### D-03 — Deliberately set and verify Rust 1.94

- Status: accepted
- Choice: set the leaf's explicit MSRV to 1.94 and retain explicit edition, dependencies, lints, license and `publish = false`. Run the detached source, verified package, unpacked artifact, public consumer, and negative-sensitivity gate on exact Rust 1.94.0 from Batter.
- Why: the owner does not require the earlier 1.85 support and selected one minimum shared with Batter's packages. An exact 1.94 detached run still proves both the declared minimum and standalone packaging boundary.

## Outcomes & Retrospective

Not yet complete. No upstream revision is available until validation and native review pass and the reviewed commit is pushed. Publication to a registry is explicitly out of scope.

## Context and boundaries

CreditKit commit `84505f0178eafadf5354b418cefbc3c14febf7b7` contains the completed source package. Its `crates/at-rest/AGENTS.md`, `README.md`, `docs/format-v1.md`, tests, and fixtures are the transfer source; provenance commits `eca46b60` and `f911961e` record initial extraction and portability hardening but must not overwrite later work. CreditKit keeps application contexts, identity messages, serializers, configuration, persistence/object adapters, migrations, rotation/retirement, deletion, deployment, and recovery policy.

The protected data and attacker model remain those documented by the leaf and CreditKit's `docs/security/threat-model.md`: authenticated application-layer encryption mitigates read-only store disclosure and accidental cross-context substitution. This relocation adds no key, algorithm, trust boundary, KMS, runtime, persistence, or stronger attacker claim.

## Execution graph

### T-01 — Batter owns the standalone leaf

- Outcome: `crates/batter-at-rest` contains the complete current implementation under the new Cargo package name.
- Changes: root workspace membership/dependency inventory; leaf manifest, guide, README, source, tests, fixtures, format contract, and proprietary license.
- Depends on: none
- Verify: source/fixture comparison against CreditKit with only approved naming differences; `cargo +1.94.0 test` and `cargo +1.94.0 check --all-targets` in the detached gate.
- Recovery: discard the feature branch before adoption; no persisted data is touched.
- Done when: the leaf is independently buildable/packageable and no crypto/application identity byte changes exist.

### T-02 — The opt-in facade preserves isolation and identity

- Outcome: `batter::at_rest` exposes exactly `batter_at_rest`'s public types under feature `at-rest`, with no crypto package in the default graph.
- Changes: `crates/batter/Cargo.toml`, `crates/batter/src/lib.rs`, facade feature runner and documentation.
- Depends on: T-01
- Verify: isolated positive/negative facade consumers, normal dependency-graph assertions, and compile-time direct/facade type identity.
- Recovery: remove the optional feature without affecting the leaf's direct-consumer path.
- Done when: opt-in and all-feature consumers compile, the disabled module fails at the intended API, and default metadata excludes `batter-at-rest` and its crypto dependencies.

### T-03 — Batter owns continuous portability evidence

- Outcome: future changes to the leaf remain covered by Batter CI and Jig inputs.
- Changes: portability script, GitHub workflow, `.jig.toml`, generated Jig contract, agent map/guides, testing/status/usage documentation as applicable.
- Depends on: T-01, T-02
- Verify: gate self-sensitivity; exact Rust 1.94 detached/package/consumer run; Jig contract and repository policy checks.
- Recovery: fix ownership scopes at their source; do not weaken the detached oracle.
- Done when: all source, fixtures, scripts, workflows and config that affect the claim are included in continuous checks.

### T-04 — Produce an adoptable immutable upstream revision

- Outcome: CreditKit can resolve the reviewed package from the Batter remote by exact commit.
- Depends on: T-03
- Verify: Batter focused checks, supported-toolchain checks applicable to changed paths, plan-associated Jig gates, staged native Codex review with no findings, remote inspection after push.
- Recovery: if review exposes a design/ownership mismatch, stop before commit/push; after push, repair with a new commit and make CreditKit pin only the repaired revision.
- Done when: a reviewed commit exists on `origin/feat/batter-at-rest`, the remote commit contains `crates/batter-at-rest`, and its full SHA is recorded for CreditKit.

Critical path: T-01 -> T-02 -> T-03 -> T-04. The source transfer and facade edits share manifests and lockfile, so they are executed serially.

## Validation and acceptance

Run the narrow leaf and facade checks first, then the repository-required gates. At minimum: the Batter-owned portability script on exact Rust 1.94.0; leaf test/check/clippy/doc and fixed-vector checks; facade feature runner including type identity and default isolation; `cargo test -p batter --all-targets --locked`; formatting; and the fresh plan-associated Jig gates reported by `scripts/jig work gates`/`work evidence`. Run broader supported-toolchain verification only where the changed workspace graph invalidates it. Record exact commands and results in this plan.

Stage every upstream task change before `codex review --uncommitted`. Every review finding is also recorded in the workflow's external findings file and resolved at root cause; no review-fix-loop controller is used.

## Idempotence and recovery

All implementation steps affect source control only. Portability checks use exact temporary roots with cleanup traps. No registry publication, deployment, key operation, ciphertext rewrite, or object upload is permitted. Until CreditKit pins the pushed SHA, abandoning the feature branch fully contains the change. After adoption, roll forward with a new immutable upstream commit and downstream pin rather than rewriting the referenced commit.
