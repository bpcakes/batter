Replace hasher construction counters with private logical parameter traversal/index accounting; preserve semantic fixtures and prove scan/reindex regressions are rejected. Preserve the pre-existing staged verifier work. Run focused tests, both verify.sh toolchains, ten rebuilt HTTP smoke profiles and final Jig evidence/gates. Record contracts and execution evidence; no public API, dependency or database changes.

## Progress

The private catalog now carries a test observer across traversal and name lookup.
ParameterIndex encapsulates the native HashMap without hasher generics. Both
scale regressions retain existing verdicts and add ACL/grant-option checks; the
evaluation case exercises the full production evaluator. Four temporary scan or
rebuild mutations fail their work-count assertions. Forced hash-table growth
passes with unchanged counts. All temporary mutations are removed.

Both toolchain matrices, ten HTTP smokes and all five final Jig targets pass.
Evidence and gates readback reports fresh passing receipts, including api:test
receipt_01M2DTD1MD1VNCM405R1618XE9. Closeout refreshes only affected policy checks.
Exact commands and outcomes belong to docs/validation.md's batter-akt section.

## Surprises & Discoveries

The initial ACL assertion named the fixture role incorrectly; its actual identity
is `login`. Correcting that expected identity passed without changing production
semantics. The existing exhaustive `crates/*/src/**` scopes in .jig.toml and
.agent/jig-contract.json already include the new private parameter_index module.

## Decision Log

Count visits inside the catalog iterator and lookups inside the private index,
so scans and repeated builds are observed without counting native hashing. Remove
the catalog's slice Deref to keep evaluation traversal at that boundary. Keep the
counter state test-only and leave the policy-validation observer unchanged.

## Outcomes & Retrospective

Focused acceptance, four mutation controls, forced table growth, both complete
verification matrices, ten HTTP smokes and final Jig gates pass. The changed
oracles reject scan/reindex regressions without requiring a hashing schedule.
No live PostgreSQL or new macOS execution is claimed. All pre-existing staged
changes were preserved; no commit or publication was performed.
