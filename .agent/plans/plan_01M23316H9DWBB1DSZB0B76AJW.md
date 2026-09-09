# Implementation-readiness audit — 2026-09-09

This is a planning-session evidence record, not an implementation ExecPlan or a second backlog. Beads remains the owner of delivery scope, acceptance, status, priority and dependencies.

## Scope and baseline

Audit the current delivery tasks at Git `22848ea6884f13b014c74850e7bd888e0b8a94d8` against actual Batter and candidate upstream APIs. Preserve inherited working-tree changes, closed delivery and concurrently added operational extraction work. The inventory grew from 45 to 52 records during the session; those seven additions belong to the concurrent planning session and were preserved and reviewed here.

This audit revised 23 open tasks and added 10 blocking edges. It created no Beads and changed no delivery status or priority. All 15 closed records and three deferred conditions were preserved. No application implementation, upstream mutation, commit, publication or deployment was performed.

## Findings incorporated

Source inspection corrected native transaction/migration probes, worker initialization evidence and shutdown visibility, SQLx close's unit return, harness lease Drop/deferred cleanup, and detached server-session observation. Runtime compatibility remains an executed output of the owning compatibility task, not a claim established by this audit. Immutable candidate references and the limits of inspection are in `docs/references.md`.

The reference command now specifies caller-known-key reconciliation after a lost commit acknowledgement. Fixtures precede real worker tests, final schema follows admission, and reference roots consume the operational helpers they need. Producer completion does not depend on a downstream adopter finishing first. Actual downstream access and an immutable dependency revision remain explicit external prerequisites.

Retry tasks have one composable opt-in execution path, explicit attempt-versus-total timeout outcomes, and token debit at an observed final preflight plus counted invocation. No-await is not described as atomicity with cancellation or the clock. Live PostgreSQL cases have an explicit ignored target/runner because ordinary verification enables all features and targets.

The staged worker host omits the real delivery handler until its provider implementation exists. A control witness can acknowledge its component while ordinary application readiness remains unavailable. A separate application termination gate distinguishes not-started, unproven and cooperatively stopped work. Native returned timeout errors do not magically trigger core cleanup skipping: the retained application cleanup owner carries the nested stack, resources and skipped/pending report even if the core skips its grouped hook.

## Review evidence

Four full review rounds and four isolated task handoff checks were performed. Round 1 found the API/oracle and producer gaps. Round 2 found combined retry and operational consumer ordering gaps. The isolated check after round 3 found the returned-error cleanup bridge and probe-stage readiness ambiguity missed by full review; these were incorporated before final review. Round 4 required only clarification of pre-spawn cleanup ownership and reached steady state with no remaining required amendment.

Final reviewed proposal SHA-256: `4107ae11a2cf17a9e74a87735590d6b7c87801e944ddd3df45d8c34471f3157e`.

Six graph polish checks covered live inventory equality, DAG/frontier, producer ownership, justified roots/terminal outcomes, heuristic alerts/priorities, and export round-trip. The final graph has 52 records, 83 blocking edges, no cycles and ten dependency-ready tasks. Graph readiness does not imply that hosted-event permissions, a live database or downstream access are already available. Shared-text duplicate/label suggestions and reverse-consumer dependency suggestions were examined and rejected on scope grounds.

Local detailed snapshots, independent notes and command outputs are under `/tmp/batter-implementation-ready-20260909/`; enduring delivery requirements are in the live Beads, not those scratch files. The JSONL export was generated through `br`, never hand-edited, and independently read through `br --no-db`. `bv` identified its chosen source and agreed with the live graph.

## Verification and limits

`git diff --check` passed. Required `scripts/jig work check --plan-id plan_01M23316H9DWBB1DSZB0B76AJW --json` verification passed Clippy, formatting, tests, repository contract and file-budget checks in run `run_01M233EGKW05Z6S8F9QY068K8H`. Its test receipt `receipt_01M233H23K0TSPVGFSZ22KNRTZ` records 425 passing test/doctest executions, zero failed and zero ignored, including repeated core profiles on default Rust 1.98.1 Linux. Final receipt freshness is re-established after tracker amendments before closing this planning session.

No Rust dependency/code changed. The Rust 1.94 matrix, HTTP smoke, macOS, hosted runs, live PostgreSQL, upstream compatibility and downstream adoption were not re-executed by this planning audit. Existing evidence keeps its recorded scope. Source/contract/status documents label the planned integrations as unimplemented/unexecuted where applicable.
