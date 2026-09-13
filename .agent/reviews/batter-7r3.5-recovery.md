# Frozen recovery review, 2026-09-13

Both selected reviewers completed against verified fingerprint
`9e33a03f327acd6e0b3ee07a952d2cfe26ea9098231fc4f39e3fad3322e44a91`,
HEAD `ff507dcf48c688bbc31d99a517cfd5888de5910a`. Parent pre/post
captures complete=true, issues=[], pathInventoryComplete=true. Claude used default
configuration, opus, restricted read-only tools; evidence coverage is
reviewer-attested, 64/64 pages with valid receipts, not proof of review quality.
Codex independently inspected final source. Trusted HEAD .reviewignore excluded
only .agent. No explicit exclusions or submodules. Inventories are complete:
24 tracked paths differ from the index; 17 untracked paths are absent from it.
The pre-existing index remains untouched and differs from reviewed source;
any future commit must include intended final files, not blindly commit the index.

## Consolidated findings and adjudication

- High, Claude/Codex: serving-controlled SQL resolution can redirect catalog
  helpers and hide objects/authority. Native current_database shadow reproduced;
  candidate regression fails before trusted-local-path fix and passes after.
- Medium, Codex: ONLY parent ledger lock disagrees with descendant-inclusive read.
  Native child TRUNCATE hides a pre-snapshot row. Candidate recursive-lock control
  passes; original parent-only guard is being mutation-checked. This recurring
  invariant triggered the ADR-010 assessment in the owning execution plan.
- Medium, Claude: automatic array-type ACLs under discovery can cause false PUBLIC
  findings; verification against native element-type semantics pending.
- Medium, Claude: expanded PUBLIC defaults are repeatedly scanned and routine
  identities cloned, causing quadratic non-yielding evaluation near the cap;
  source confirmation and bounded indexing repair pending.
- Low, Claude: invisible superuser-only pg_settings names without ACL are treated
  as missing; native visibility/coverage behavior needs confirmation.
- Low, Claude: live prerequisite docs still mention TEMP although isolated ledger
  setup now needs CREATE on the database. Accepted documentation correction.
- Low, Claude: prior borrowed-API validation section lacks an explicit historical
  heading. Accepted documentation correction; latest evidence is separate above.

The fixed-cap suggestion is not accepted as an implementation defect. The user-
approved recovery explicitly chose finite row/byte capacities, fail-closed
CatalogCapacity, and no new reachable-only SQL optimizer. Cluster-wide limits are
an operational boundary, not policy-scoped truncation; documentation must state
that unrelated catalog growth can reject inspection. Making caps configurable or
adding recursive SQL is outside this agreed repair and is not used to evade the
capacity contract. The review's claim that no near-cap control exists is stale:
current live tests execute exactly 10,000 expanded ACL rows successfully then
reject 10,001, plus row/name overflow and a 2,048-role graph unit control. Discovery
performance specifically still needs a discriminating check.

Combined verification's lack-of-ledger-SELECT native error is intentional and
documented; authority-only inspection exists to obtain MissingPrivilege without
reading the ledger. Reports retain typed Native causes, so this is not an
unstructured discarded error. No claim of overall security, remote termination,
macOS/hosted execution, or deployed downstream upgrade follows from this review.

## Repairs after the frozen paired pass

All accepted findings above now have implemented repairs and focused evidence.
Trusted transaction-local resolution and descendant-inclusive ledger locking each
fail their isolated omitted-guard control and pass when restored. Dependent
array/multirange ACL checks agree with native revoked/granted effective USAGE;
discovery's complete clean verdict no longer invents their default PUBLIC ACLs.
Catalog vector identities remain selectable while effective ACLs follow native
element semantics. PUBLIC and object override indexes eliminate repeated full
policy scans; a synthetic 4,096-type plus 4,096-routine collection passes cleanly
then identifies exactly one forbidden definer grant after one object changes.

Parameter visibility uses current-role INHERIT privileges, not possible SET roles.
A restricted declared setting yields ParameterUnobservable/Incomplete, retains
known findings and does not invent missing-object/privilege results. Under proven
full current visibility a bounded boolean-only current_setting existence probe
distinguishes actual absence from existing NO_SHOW_ALL settings; values are not
returned and absent context remains incomplete. Native restricted/SET-only/
INHERIT/restored-grant/NO_SHOW_ALL controls pass. The CREATE prerequisite and
historical validation heading were corrected. The deliberate fixed catalog-cap
boundary is explicit in the current contracts.

Both complete workspace toolchain matrices, exact 46-case SQLx live runners,
ten HTTP smokes and both verifier example modes pass. The real-consumer profiles,
six unchanged privilege regressions and strict Clippy passed again. Current Jig
api:test receipt is receipt_01M2D9F22BN104C2DVJD9QJTGP, with all required gates fresh.
Final single-reviewer Codex assessment of the repaired source is underway; it is
not represented as a second paired Claude/Codex result.

## Final full native pass and required-parameter correction

The independent full Codex pass completed with matching complete pre/post
fingerprints 30d44538e8153ba2f6a277da67a1b6ee4cbf3e411cca2c87adf18b2cf8dd7400.
It identified one medium finding: an unknown custom parameter could satisfy
required SET through the conservative placeholder assumption. No other actionable
finding was identified. The reviewer inspected primary PostgreSQL source and did
not execute tests or access the owned cluster. The parent reproduced the false
pass in the isolated source copy with PL/pgSQL's reserved prefix before repair.
The native rejection plus passing old verifier is retained in
/tmp/batter-snapshot-mutant/reserved-parameter-before.log.

The repair retains conservative potential authority but requires observable
context for a positive custom-parameter requirement. Required unknown names emit
ParameterUnobservable/Incomplete even with an ACL. A native case now distinguishes
rejected reserved-prefix names, successfully assigned but unobservable placeholders,
and visible loaded parameters; the latter still passes cleanly. A synthetic
unknown-context case covers both absent and explicit current-role ACLs. All 45
library tests, that native control and strict all-target adapter Clippy pass.
The first new test compilation exposed an incorrect nested import; correcting
the import passed. Its 104-line native scenario uses the existing long-scenario
Clippy allowance; assertions and failure semantics are unchanged.

A targeted read-only confirmation by the same independent reviewer is running
against complete fingerprint
8012edd5421b835d3a8e74a9264a7c4da2e79e399dfeffe5f78f5da0256ca804.
Both full toolchain matrices, now-exact 47-case live runners, HTTP/example modes,
Jig and the affected consumer controls are being repeated for this semantic edit.

## Final disposition

All confirmed findings are resolved. The final full native pass found only the
custom-parameter requirement defect; its targeted repair confirmation reports
that defect resolved and no confirmed coupled defects. It explicitly reviewed
the reserved-prefix, placeholder, loaded-context and synthetic ACL controls.
The hidden custom-definition case is synthetic, not a live compiled extension
fixture. The reviewer did not execute tests; parent execution is recorded
separately. This was a full Codex pass followed by targeted confirmation, not
a repeated full pass or a second paired Claude/Codex review.

Review notes:
- Reviewers requested for final pass/confirmation: Codex.
- Scope: working-tree, including staged, unstaged and untracked final files.
- Codex review: completed; targeted repair confirmation completed.
- Scope fingerprint: verified, complete=true, issues=[], matching reviewer and
  parent pre/post captures at 8012edd5421b835d3a8e74a9264a7c4da2e79e399dfeffe5f78f5da0256ca804.
- Tracked paths differing from index: 25; .beads/issues.jsonl, crates/batter-sqlx/AGENTS.md, crates/batter-sqlx/README.md, crates/batter-sqlx/examples/verification.rs, crates/batter-sqlx/src/verification.rs, crates/batter-sqlx/src/verification/authority.rs, crates/batter-sqlx/src/verification/authority/database.rs, crates/batter-sqlx/src/verification/authority/privileges.rs, crates/batter-sqlx/src/verification/authority/requests.rs, crates/batter-sqlx/src/verification/migration.rs, crates/batter-sqlx/src/verification/policy.rs, crates/batter-sqlx/src/verification/report.rs, crates/batter-sqlx/tests/postgres_live.rs, crates/batter-sqlx/tests/verification_fixture.rs, crates/batter-sqlx/tests/verification_live.rs, crates/batter-sqlx/tests/verification_live_capacity.rs, crates/batter-sqlx/tests/verification_live_cases.rs, crates/batter-sqlx/tests/verification_live_public.rs, docs/guarantees.md, docs/integrations.md, docs/references.md, docs/status.md, docs/testing.md, docs/validation.md, scripts/sqlx_live.py; complete.
- Untracked paths absent from index: 22; crates/batter-sqlx/src/verification/authority/database/parameters.rs, crates/batter-sqlx/src/verification/authority/discovery.rs, crates/batter-sqlx/src/verification/authority/discovery/scale_tests.rs, crates/batter-sqlx/src/verification/authority/privileges/public.rs, crates/batter-sqlx/src/verification/authority/required.rs, crates/batter-sqlx/src/verification/authority/required/tests.rs, crates/batter-sqlx/src/verification/authority/selection.rs, crates/batter-sqlx/src/verification/authority/tests.rs, crates/batter-sqlx/src/verification/policy/discovery.rs, crates/batter-sqlx/src/verification/policy/tests.rs, crates/batter-sqlx/tests/verification_fixture/provision.rs, crates/batter-sqlx/tests/verification_live_adoption.rs, crates/batter-sqlx/tests/verification_live_cases/narrow.rs, crates/batter-sqlx/tests/verification_live_cases/noinherit.rs, crates/batter-sqlx/tests/verification_live_cases/owners.rs, crates/batter-sqlx/tests/verification_live_cases/parameters.rs, crates/batter-sqlx/tests/verification_live_cases/roles.rs, crates/batter-sqlx/tests/verification_live_discovery_scale.rs, crates/batter-sqlx/tests/verification_live_ownership.rs, crates/batter-sqlx/tests/verification_live_parameters.rs, crates/batter-sqlx/tests/verification_live_recovery.rs, crates/batter-sqlx/tests/verification_live_visibility.rs; complete.
- Dirty submodules: 0; none; complete.
- Excluded paths: .agent (trusted policy only; no explicit exclusions).
- .reviewignore source: ff507dcf48c688bbc31d99a517cfd5888de5910a:.reviewignore.

The original index was preserved. Any future authorized commit needs intended
tracked changes re-staged and intended untracked files added; committing the
current index can retain defects superseded in the reviewed working files.
Final validation/status/tracker result updates follow this review freeze and
do not change Rust implementation or tests. No staging or commit is performed.
