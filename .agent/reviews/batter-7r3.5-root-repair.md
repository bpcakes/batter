- [medium] [required.rs:114](/home/aa/Documents/batter/crates/batter-sqlx/src/verification/authority/required.rs:114) Temporary schemas use the wrong authority model.
  Source: Codex.
  Why it matters: PostgreSQL gives the connection's own temporary namespace USAGE and derives CREATE from database TEMP. The verifier uses ordinary namespace ACLs in both required and excess checks. An external public-API probe with a real nonsuperuser login and a reused one-slot pool confirmed native USAGE/CREATE=true, two incorrect MissingPrivilege findings when required, and WithinDeclaredPolicy with zero findings when both schema privileges were denied. The excess path is [requests.rs:280](/home/aa/Documents/batter/crates/batter-sqlx/src/verification/authority/requests.rs:280).
  Recommendation: classify session-dependent namespace authority at capture and implement the native rule in both paths, or report that selected surface as unsupported. Never silently treat it as an ordinary ACL. Tracked as **batter-7r3.12**, blocking batter-7r3.5 and its conditional commit.

- [low] [discovery.rs:273](/home/aa/Documents/batter/crates/batter-sqlx/src/verification/authority/discovery.rs:273) Exact PUBLIC relation denial leaves column defaults active.
  Source: Claude.
  Why it matters: Nonempty PUBLIC column defaults are expanded even with an exact empty PublicAllowance for the relation. A column grant can therefore be accepted despite an apparent relation-wide denial; role-side RelationPolicy suppresses these defaults. The source behavior is established, but the intended hierarchy needs a contract decision. No live reproduction was executed for this item.
  Recommendation: define and test relation/column PUBLIC precedence, keeping explicit column exceptions expressible, then align validation, expansion and documentation. Tracked as **batter-7r3.13**.

Open questions:
- Relation/column PUBLIC override hierarchy is the explicit decision in batter-7r3.13.
- Claude questioned ADMIN on a superuser role; this remains an unexecuted edge question, not an accepted finding.
- Claude and Cursor asked about combined ledger permission errors. The README already directs callers to authority-only inspection for structured missing-ledger privilege findings; no changed contract is inferred.
- Cursor asked about role/parameter work capacity and the pre-version ledger lock. These remain documented fail-closed bounds/order questions, not demonstrated wrong verdicts.

Test gaps:
- The temporary-schema probe confirms the missing differential test; it is external evidence, not a checked-in regression.
- No explicit relation PUBLIC deny/column default interaction test.
- Boundary tests for ordinary relation/routine/schema expanded ACL capacity, combined ledger SQLSTATE 42501, large superuser discovery, and below-row-cap live parameter work exhaustion remain absent.
- Production evaluation capacity has an existing offline regression with 600 parameters and 2,000 roles. That does not establish the separate live parameter scenario.
- Cursor also noted no pinned native case for row-type aliases or arbitrary nonmembership superuser SET ROLE.
- Codex inspected production, contracts, runner and selected tests; it did not inspect every added fixture/test line. No new macOS, hosted, full downstream or deployed validation is claimed.

Review notes:
- Reviewers requested: Claude, Codex, Cursor.
- Scope: working-tree; cumulative verifier implementation and root repair against HEAD ff507dcf48c688bbc31d99a517cfd5888de5910a.
- Claude review: completed, opus, effort inherited.
- Codex review: completed, native inherited model/effort.
- Cursor review: completed, cursor-grok-4.6-high, standard speed.
- Claude file access: restricted.
- Claude config: default.
- Scope fingerprint: verified; complete before/after captures and all reviewer checks matched 395baed2f44ab9d142797a16d9a4598b77c97f30a993e8fe68e7f4ebd45e6380. No capture issues.
- Tracked paths differing from index: 0; none; complete.
- Untracked paths absent from index: 0; none; complete.
- Dirty submodules: 0; none; complete.
- Excluded paths: .agent.
- .reviewignore source: ff507dcf48c688bbc31d99a517cfd5888de5910a:.reviewignore.
- Claude evidence coverage: reviewer-attested, 44/44 pages with valid receipts.
- Cursor evidence coverage: reviewer-attested, 44/44 pages with valid receipts.
- Receipts attest page access and coverage claims, not review quality. Scope stability is separate.
- Codex's first helper invocation included an accidental punctuation argument and failed before repository inspection. The corrected exact command matched before inspection and after drafting; no failed capture was counted as successful.

Adjudication:
- Claude's medium capacity claim is not retained as a defect: README explicitly bounds each catalog query including expanded ACL rows, documents cluster-wide growth failures, and distinguishes input-policy limits from captured-catalog/work limits. Valid policy syntax never promises arbitrary catalog cardinality. The practical object threshold and missing ordinary-ACL boundary tests remain limitations; the limit was not raised or hidden.
- Cursor reported no actionable findings. Its report independently supplied open questions and test gaps only.
- Parent verification of the temporary-schema case does not add another reviewer source. PostgreSQL 18 primary implementation: https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/aclchk.c, pg_namespace_aclmask_ext; namespace initialization: https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/catalog/namespace.c.

Validation and outcome:
- Both Rust 1.98.1 and 1.94.0 full verify.sh matrices, both 49-case SQLx live runners, separately built HTTP examples and five smoke modes each, normal/violation verifier outcomes, and five Jig gates passed before review. Final api:test receipt receipt_01M2DD7V13WA6Z3DBEG54F00AT.
- Three guard mutations were behaviorally rejected; restored sources passed final validation. Exact execution evidence is in docs/validation.md and /tmp/batter-root-final-results.json.
- Native temporary-schema probe source: /tmp/batter-temp-review-probe/src/main.rs; final exit-0 log: /tmp/batter-temp-review-probe-final.log. Initial admin-authenticated session-authorization probe established the required error but was unsuitable for the excess-root oracle. Corrected real-login execution confirmed both sides. An early external-probe dynamic-SQL compile error and cleanup race were corrected before the final successful execution; these did not modify repository sources.
- Root repair is implemented and verified, but the selected final review contains a confirmed medium issue. **No commit was made.** No further repair loop was started. The owning delivery Bead remains open; the bounded repair/review work ends with flagged findings.
- Post-review edits are limited to tracker and evidence/status documentation. The reviewed Rust, tests, scripts, dependency graph and contract implementation remain unchanged. Whole-repository policy checks are refreshed after those records settle; Rust receipts may be reused with unchanged inputs/toolchains.

Final evidence settlement: Jig marked Cargo.lock's input changed after staging and
executed all five gates again successfully (including api:test
receipt_01M2DEADN1AZXC4HY0XS0D9YDZ, validation
receipt_01M2DEAKJB8YKKCJB33NT6GEGM). Source bytes and the reviewed dependency
graph remain unchanged. The disposable PostgreSQL container and private
credential files were removed.

Final policy-only refresh passed, reusing all three Rust receipts and executing
repo:contract/repo:file-budget. Complete evidence/gates inspection reports every
required target passed and fresh; final validation receipt
receipt_01M2DECHB1GDCDCZ2HB382W08D. Inspection artifacts:
/tmp/batter-root-evidence-close.json and /tmp/batter-root-gates-close.json.
