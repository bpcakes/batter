# Repair reference ConnectInfo evidence and live request harness

Owning Bead: `batter-in2` (reopened and reclaimed before repair). This plan repairs the evidence/harness gap discovered after the first implementation without changing the selected product boundary. The completed historical plan `plan_01M2NAC48CCB639V71F8BWNZBZ` remains immutable.

## Progress

- [x] Reopen and reclaim `batter-in2`; verify its dependent open work waits again.
- [x] Inspect the current shared live request helper, production-root raw socket helper, trusted metadata rustdoc/config and repository state.
- [x] Repair in-process live requests and body/header correlation assertions.
- [x] Add a real production-socket business-route regression.
- [x] Clarify private construction, code-selected policy and three-layer evidence contracts.
- [x] Pass focused checks and the complete external 64-entry reference live inventory.
- [x] Pass both complete Rust matrices, ten rebuilt HTTP smokes and final Jig gates; close/export the owning Bead and complete the staging audit.

## Surprises & Discoveries

The first implementation correctly made missing `ConnectInfo<SocketAddr>` fail closed, but the shared database-backed in-process request helper still called the router without inserting that trusted native extension. Those ignored tests compiled while their real execution would return missing-peer 500 before intended handlers. The production root selected `register_http_with_connect_info_in` statically, but its live process probe exercised only `/live` and `/ready`, which bypass the business metadata layer. Validation therefore overstated closure.

Review finding fingerprint: complete pre-repair working-tree fingerprint `d1d5e98fbd8f9dd94c8cda858b992bdc2f19476985e4abd8b1d20c4d284ad301`, computed from the binary HEAD diff plus every untracked path hash after reopening/reclaiming the Bead. The initial invalid empty-input attempt is not evidence and is excluded.

## Decision Log

- `TrustedPeerPolicy::direct()` remains selected in code and carried through serving preparation. It is not an environment setting and implies no planned proxy mode.
- `TrustedRequestMetadata` keeps private fields and no public/test constructor. Only middleware combining server `CorrelationId` with accepted-socket `ConnectInfo` can construct it.
- In-process tests insert the actual `ConnectInfo<SocketAddr>` extension and explicitly own that synthetic transport trust assertion. `MockConnectInfo` is extractor fallback and cannot satisfy middleware that reads request extensions directly.
- No settings key, proxy/CIDR mode, forwarded-header parser, public trusted-metadata constructor or production test-only constructor will be added.
- The full external reference inventory is the closure gate. Compilation or ignored execution does not substitute for it; unavailable endpoints leave the Bead open.

## Outcomes & Retrospective

Implemented and verified. The in-process live helper inserts exact native
`ConnectInfo`, and every parsed response requires body/header request-ID
agreement. The production child reaches authentication through the real socket
path and distinguishes the expected 401 from the missing-peer 500 control.

All four focused reference commands passed. The task-owned PostgreSQL 18.4 run
passed the exact 64-entry inventory with zero failed, ignored or filtered cases;
both supported Rust verification matrices and all ten rebuilt HTTP smoke
profiles passed. Final Jig verification passed with fresh target validation
receipt `receipt_01M2NGYNN3JFS0768QF24KSBM0` and no unresolved gate. The
task-owned database containers were removed; no dependency or Jig input-scope
change was needed. The source modules remain deliberately untracked because
staging and commit were not authorized. `batter-in2` was closed with the repaired
evidence and its tracked export is current.

## Context and orientation

`examples/reference-service/tests/support/delivery.rs` owns the shared database-backed in-process router request helper. `tests/support/production_root.rs` owns the real production child and raw TCP probes. `src/request.rs` directly reads `ConnectInfo<SocketAddr>` from request extensions. `src/runtime.rs` selects `register_http_with_connect_info_in`. `src/config.rs` pins and carries the direct policy.

## Plan of work and interfaces

Build each in-process request as mutable, insert `ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0)))`, and document that the fixture asserts trusted transport provenance. Capture `x-request-id` before body consumption and require parsed JSON `request_id` equality in the shared helper so all live outcomes inherit the check. Do not insert trusted metadata directly.

Extend the production root after liveness with one unauthorized `GET /delivery-commands/transport-probe` over its actual TCP listener. Parse status, headers and bounded body with existing Tokio I/O; require 401, `authentication_required`, generated `x-request-id`, and matching body identity. Preserve the missing-peer unit test as negative control.

Update request/config rustdoc and reference compatibility, guarantees, integrations, testing, status, references and validation to distinguish adapter native provenance, explicit in-process `ConnectInfo` injection and actual production-socket proof. Correct prior evidence attribution; do not claim the live inventory until it executes.

## Validation and acceptance

Run the four focused reference commands, then `bash scripts/test_reference_live.sh` against the required two disposable PostgreSQL 18 endpoints. If unavailable or any case fails, retain the open Bead and report the exact blocker. After live success run `bash scripts/verify.sh`, `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, rebuild `http_service` with each toolchain and run default, SIGINT, deadline, warn-filter and combined profiles. Finish through this plan with `work check`, evidence and gates. Confirm Cargo.lock unchanged and Jig scopes unchanged.

## Idempotence and recovery

The in-process helper uses a documentation-only loopback socket address and opens no socket. The production probe owns its child/listener and existing bounded shutdown. The live runner owns external fixture databases but not server shutdown. Never log endpoint secrets. Preserve untracked `src/request.rs` and `src/http/tests.rs`; no staging, commit, push, publication or deployment is authorized.

## Dependencies

Use locked Axum 0.8.9 semantics: native make-service inserts real `ConnectInfo`, while `MockConnectInfo` is only an extractor fallback. Existing workspace dependencies and Jig exhaustive paths are sufficient.
