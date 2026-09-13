# batter-7r3.5 stopped-work handoff

Date: 2026-09-13

This is a durable post-loop metadata handoff, not a source repair. The Bead
remains `in_progress`; no commit, Bead close, task-2 work, staging change, Jig
mutation, or verification rerun was performed after the frozen implementation.
Repair round 2/2 is exhausted. The final review workflow stopped
non-converged: review passes 1 and 2 had Claude, Codex and Cursor reports;
review pass 3 had complete Claude and Cursor reports, while Native Codex
exceeded its 30-minute bound and was interrupted at 01:59:24 UTC. Therefore
provider coverage is incomplete and no clean all-reviewer conclusion may be
claimed.

The source/docs freeze before this metadata update was HEAD
`ff507dcf48c688bbc31d99a517cfd5888de5910a` with complete working-tree
fingerprint
`aa61591ea535ed2fc696d5b0af5a687658c71c12485be95cbea16580513c7c11` (scope
`working-tree`, `.agent` excluded). The fingerprint JSON is
`/tmp/batter-7r3.5-round2-final-fingerprint.json`; it reported no issues, 25
tracked paths differing from the index and two untracked SQLx test modules.
Those paths are implementation history, not a new repair after review.
After the authorized Bead comment and flush-only sync, the final post-loop
scope fingerprint is
`05850d31511938483c8393fd41d1f332865a4506b03b374077c0c60c45f3bb76` in
`/tmp/batter-7r3.5-stopped-final-fingerprint.json`; the only reviewed-scope
content change is Bead export metadata (comments 67 and 68), while `.agent`
remains excluded.
The index still contains the twelve earlier staged additions
(`examples/verification.rs`, `src/verification.rs`, the seven `src/verification`
modules, and `tests/verification_fixture.rs`, `tests/verification_live.rs`,
`tests/verification_live_cases.rs`). Their working-tree contents differ from
the index and must be restaged before any future commit; the two newer
`verification_live_capacity.rs` and `verification_live_public.rs` files remain
unstaged. No commit index is treated as final.

## Validation retained before the stopped review

- `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`:
  both passed the complete workspace matrices, strict Clippy, rustdoc and
  doctests.
- HTTP example plus five smoke profiles on each toolchain: 10/10 passed.
- `bash scripts/test_sqlx_live.sh` on each toolchain: 29/29 passed (11
  PostgreSQL lease/read-only-verification, 14 pool ownership, 4 authority).
- Restricted-login acceptance: 4/4, including PUBLIC, row type, columns,
  parameters, routines, ownership, two policies, nested role reachability,
  concurrent snapshot case, 48-object case and one-slot blocked cancellation.
- Feature/offline tests, strict format and direct file-budget passed; file
  budget had 0 errors, 6 warnings and 8 notices.
- The runnable generic example produced `WithinDeclaredPolicy` and a deliberate
  checksum violation produced `Violations`.
- There is no fresh post-review Jig `api:test` receipt. Earlier receipts are
  historical and must not be represented as post-review completion.

## Final-review finding ledger

Verification state and repair state are intentionally separate. `verified`
means the review evidence or static proof establishes the current behavior or
defect; `rejected` means the reviewer premise conflicts with the bounded
contract or primary PostgreSQL semantics; `unresolved` means verification
still needs a material answer, not necessarily that a concrete implementation
bug has been proven. Repair state is `fixed`, `pending`, or `NA`; pending items
cannot be repaired in this stopped round.

The effective review-loop request was exactly
`--fix-mode comprehensive --min-severity low --max-rounds 2 --scope
working-tree --all-reviewers --codex-model gpt-5.6-luna --codex-effort max`.
The reviewed scope used the trusted HEAD policy at
`ff507dcf48c688bbc31d99a517cfd5888de5910a`, with `.agent` as the only review
exclusion; `.beads` remained in scope.

| ID / attribution | Verification state | Repair state | Current evidence and remaining issue |
|---|---|---|---|
| C1 — Claude HIGH | Rejected as a universal-scope requirement; narrow disclosure wording remains unresolved | pending | `report.rs:62-93` explicitly labels declared schema/relation/column/type/routine/PUBLIC/ownership surfaces, while `requests.rs:98-165` inspects requested objects only. The generic contract is deliberately declared-only and does not default-deny every undeclared object. However, `SupportedSurface::PublicAcl`/`Ownership` wording is less explicit than the other declared labels, and there is no separate undeclared-object surface. Do not broaden scope or claim universal security. |
| R10-array — Claude MEDIUM, Cursor LOW | Verified defect by static review proof | pending; repeated R10 causal group | `privileges.rs:231-249` treats any nonzero `TypeName.element` as an array and repeatedly follows it. PostgreSQL 18 `pg_type` has non-array built-ins with nonzero element metadata, and multidimensional array OIDs can be misbound by the policy dimension loop. The structural API repair did not fully solve true-array discrimination. |
| R1/R1b-superuser — Claude MEDIUM | Verified defect by static review proof | pending; repeated R1/R1b causal group | `authority.rs:295-300` puts inherited subjects into the subject set; `privileges.rs:137-143` grants all ACL privileges whenever an inherited subject is superuser, and `requests.rs:58-61` skips it when `allow_superuser` is true. An INHERIT-only superuser group must convey ACL authority without making role attributes or every object ACL implicitly allowed. |
| R4-materialization — Claude MEDIUM | Verified performance/prevention gap by static review and test inspection | pending; repeated R4 causal group | `database.rs:201-230` loads every non-system relation column globally; `privileges.rs:128` clones an inherited-role set per subject, while `requests.rs:354-365` clones relation ACLs per column and type/public vectors. The 48-relation/8-column test at `verification_live_capacity.rs:144-180` checks only status and absence of findings; it has no query-count, bounded-work, or scaling oracle. |
| R3-prevention — Claude LOW, Cursor MEDIUM | Verified invalid prevention oracle; implementation snapshot remains verified | pending; repeated R3 causal group | `authority.rs:371-374` loads the complete catalog snapshot before `migration::inspect`. The live test blocks the later ledger lock and grants after the earlier ACL reads, so READ COMMITTED with the same query order could also omit the grant. The implementation's explicit snapshot remains verified, but this test does not prove it against a concurrent catalog mutation. |
| R2-undeclared-context — Cursor MEDIUM | Verified undeclared-ACL regression by static proof | pending; inherited R2 attempts 1 and 2 | `requests.rs:537-541` skips only when `configured.is_none() && parameter.acl.is_empty()`. After an unrelated role grants `work_mem`, the ACL is nonempty, `allowed` is empty at line 542-543, context defaults are unconditionally added at lines 547-562, and the unchanged login receives a ProtectedParameter finding at lines 564-571. Declared names intentionally union defaults and ACLs; Cursor's claim is not universal-GUC scope expansion. |
| N-policy-duplicates — Claude LOW | Unresolved policy-validation intent | pending | `requests.rs:251-260` builds relation/sequence maps with `collect`, so duplicate policy entries silently use the last entry; similar maps exist for requested objects. Migration duplicates are validated, but authority-policy duplicates are not. This new N-* ID is distinct from earlier R5 PUBLIC-accounting history; it is low-severity caller-policy ambiguity, not a verified privilege bypass. |
| N-role-attribute-diagnostics — Claude LOW | Verified diagnostic-quality limitation | pending | `requests.rs:28-39` emits `ObjectPrivilege::Create` for both CREATEDB and CREATEROLE and no privilege discriminator for several role attributes. Findings remain violations but do not identify the exact role attribute in all cases. This new N-* ID is distinct from earlier R6 row-type/PUBLIC history. |
| R13-doc-inventory — Claude LOW, Cursor LOW | Verified documentation defect | pending | `docs/testing.md:141-143` still says two restricted-login verification cases although the current runner has four. README/validation describe four/29 correctly. |
| R10-doc-contract — Claude LOW | Verified API/documentation mismatch | pending; repeated R10 disclosure | `RoutineType::new` at `policy.rs:73-79` accepts a string such as `pg_catalog.integer`; it later fails routine matching as MissingObject rather than rejecting the alias with `InvalidRoutineType`. README says aliases/named declarations/arbitrary SQL strings are rejected, which overstates construction-time validation. |
| R1b-admin — Cursor MEDIUM | Rejected as stated | NA | The current all-edge ADMIN result is intentional. PostgreSQL 18 `acl.c` distinguishes all-edge `is_admin_of_role` from inherited-grantor `has_admin_privs_of_role` (documented in `docs/references.md`); an all-edge membership-admin finding is not a claim that every GRANT operation has a usable inherited grantor. No third role-engine rewrite is authorized. |
| Open — Claude | Unresolved semantic/test coverage | pending | Allowlisted predefined roles' implicit object authority is intentionally policy-controlled but lacks an independent exhaustive matrix; `sighup`/superuser parameter semantics and actual `verify` paths for missing-root/unsupported/rollback are less thoroughly exercised than unit construction. These are evidence gaps, not claims of a clean review. |
| Open — Claude | Rejected/NA | NA | Public-struct compatibility is intentionally a new direct-cutover API; `RoutineType::new` accepting canonical catalog names is deliberate. These are not review defects under the approved design. |

## Primary evidence retained

The relevant PostgreSQL 18 sources remain recorded in `docs/references.md`:
role membership and GRANT option semantics, `pg_auth_members`, server
`acl.c` lines 4819-5117, `pg_type`/array metadata, `pg_attribute`, parameter
contexts and PUBLIC/default ACL behavior. The current source locations above
were read from the frozen worktree, not a superseded staged copy.

## Workflow handoff

The user's standing preference is preserved: Astra owns planning, milestone
reviews and blockers; Luna Max workers own routine coding, commands, testing,
logs and polling. Workers receive complete assignments, report only at a
milestone or blocker, reuse existing workers and wait for reports instead of
repeated status polling. This worker is stopped and must not start another
repair round or task 2.
