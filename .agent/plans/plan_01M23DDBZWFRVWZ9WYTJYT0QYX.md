# Preserve overflow diagnostics and complete mutation fixtures

Owning Bead: batter-o5f. Git baseline: fbe77addaafc8709c95d7ecf4982dd3ceea3f41d.
The existing staged local verification optimization is authorized for this commit.
This plan follows .agent/PLANS.md. The user requested fixing the two actionable
Claude findings and committing the full change. Additional speculative review
concerns are not new delivery requirements.

## Progress

- [x] Confirmed existing staged source matches the previously validated snapshot.
- [x] Add opt-in bounded head/tail capture for matrix commands and focused tests.
- [x] Complete mutation script copies and test execution from an isolated directory.
- [x] Update contract/status/evidence and run required local and Jig checks.
- [x] Prepare all validated changes for the user-authorized commit.

## Surprises & Discoveries

The shared Capture is also used for machine-readable scheduling and mutation
records. Keep its existing prefix-only default; only matrix diagnostics opt into
head/tail retention. Overflow must remain failed evidence in either mode.
A first shared-tail implementation failed the two-stream regression: pipe read order
let one stream evict the other stream's final failure. Fair per-stream tails repair
that reproduced loss without weakening the assertion. All 22 focused tests pass.

## Decision Log

Use one shared byte budget split between initial reads and rolling tails. Share
tail space fairly between stdout/stderr, lending unused space to the other stream. Insert an explicit omission marker per affected stream;
marker bytes are a bounded addition to the source-byte budget. An under-limit log
must remain byte-for-byte identical. No streaming, timeout or assertion changes
are needed. Extract the existing mutation script copy loop into a helper so the
regression exercises exactly the production copy list.

## Outcomes & Retrospective

Both actionable findings are repaired. All 60 Python discovery tests passed;
22 runner regressions include five new review controls. The isolated mutation copy
passed all 24 binary-independent controls in four shards (6.353s). Both supported
Rust toolchains passed all 445 test/doctest executions plus the 22 runner controls,
formatting, compilation, Clippy and rustdoc (28.394 / 27.307s). The rebuilt HTTP
example passed all five smoke profiles. Agent-map and four package guide checks
passed. Logs and commands: /tmp/batter-o5f-validation/.
Final Jig profile run run_01M23DRQWFMBSWAQCPN9CVK1WC passed all five targets;
work evidence and gates confirmed fresh passing receipts with no missing, stale,
unknown or failed requirements. The api:test receipt
receipt_01M23DSGVTMEVTE6PN3VKCPM3H satisfies final backend verification without
another invocation. Source, configuration, pinned toolchain and environment stayed
unchanged after verification. Only plan/tracker completion metadata follows these
checks. All implementation and validation work is complete for the authorized
commit; no push is requested.

## Implementation and validation

Extend scripts/scheduling_process.py Capture with optional head/tail retention,
thread that option through scripts/parallel_process.py, and enable it only in
scripts/test_matrix.py. Test below/at/above the limit, both streams, and a real
child producing more than the 8 MiB matrix limit through the actual matrix renderer.
Keep omitted output explicit and overflow unsuccessful. Add scheduling_controls.py
and parallel_process.py to the copy helper in scripts/check_scheduling_mutation.py;
run the copied full process-control entrypoint from a temporary directory with
Python isolated mode so repository imports cannot mask omissions.

Update docs/testing.md, docs/guarantees.md, docs/status.md and docs/validation.md.
Run focused controls, both bash scripts/verify.sh toolchains (1.98.1 and 1.94.0),
the HTTP build and five smoke profiles, then final Jig work check/evidence/gates.
Retain logs outside the repository. Fresh final gate evidence satisfies backend
completion without another duplicate run. Stage all authorized changes and commit
with a description of the final concurrent verification behavior and validation.

## Recovery and boundaries

No dependencies, application APIs or fixture deadlines change. Prefix-only capture
continues preserving machine evidence. Tests create isolated temporary subjects
and retain bounded logs; no live database is needed. On failure fix the cause and
repeat affected checks. Preserve append-only Jig memory and the existing staging.
No push or publication is authorized.
