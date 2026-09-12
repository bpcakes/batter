# Direct TCP peer registration

Owning delivery Bead: `batter-rme`. Related downstream adoption evidence remains
under `batter-7r3.6`. Implementation is restricted to Batter.

## Progress

- Implemented `register_http_with_connect_info_in` in the Axum adapter, using
  the existing private registration implementation and native make-service conversion.
- Added real socket middleware/handler peer checks and registration failures;
  reused the existing forced-abort and startup-abandonment scenarios for both paths.
- Updated rustdoc example, adapter guide/README, guarantees, integration contract,
  implemented status, testing guide and pinned upstream references.
- Focused operational target passes all 21 cases. Both complete verification
  matrices and all ten HTTP smokes pass on Linux x86_64. All five required Jig targets pass with fresh evidence.

## Surprises & Discoveries

Normal `br ready/show` and JSONL-mode creation failed import semantic validation.
Native `br sync --migrate-source-repo-path` dry-run showed one newer exported
issue with two additional comments and 16 stale local source paths. Applying its
hash-bound plan restored tracker operation without losing existing issue content.
The subsequent native export normalized those paths and reassigned the two imported
comment IDs (49/50 to 63/64); comment text, authors and times were retained.
An initial focused compile used the wrong error variant name, `DuplicateName`;
the test now checks the actual `RegistrationError::Duplicate("http")`.
The first full default-toolchain matrix passed runtime tests and doctests, then
Clippy rejected conditional path selection in two already-complex lifecycle tests.
Those shared scenarios now receive the registration function directly, removing
the branches while retaining all assertions. A second pass still exceeded the
startup helper's complexity budget; its complete-report assertions now have a
separate helper. Full-workspace Clippy passes after that extraction. Both full
matrices and all HTTP smokes were restarted against the final Rust sources.

## Decision Log

Use a specific opt-in companion instead of accepting arbitrary make-services or
changing plain-registration behavior. The consumer only needs a native
`ConnectInfo<SocketAddr>` in authentication admission. All public helpers share
registration, startup acknowledgement and graceful-drain control. Native connection
metadata comes from the accepted socket, never forwarded headers. No consumer
repository changes or authentication policy belong in this delivery.

The existing exhaustive `crates/*/src/**` and `crates/*/tests/**` scopes in both
Jig contract files already cover the new source. No scope/configuration change
is necessary. Cargo.lock and dependencies are unchanged.

## Outcomes & Retrospective

Both complete matrices, all HTTP smokes and all five required Jig targets pass.
Fresh final api:test receipt: `receipt_01M2BJDGXRDFAMGR1ZCCHTTZSR`.
Delivery Bead `batter-rme` is complete; documentation/tracker-only final edits
reuse unchanged Rust receipts and refresh whole-repository policy checks.
The intended consumer replacement
is `register_http_with_connect_info_in(scope, "http", listener, router)`.
Actual downstream adoption and macOS execution are unexecuted in this delivery.

## Validation and recovery

Run `bash scripts/verify.sh`, then `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
For each toolchain rebuild `batter-axum` example `http_service` with `--locked`
and execute all five `scripts/smoke_http.py` profiles in `docs/testing.md`.
Run `scripts/jig work check --plan-id plan_01M2BHP64PFFA99BPV57MT2WH2` and inspect
its evidence/gates. Record commands, lock hash, platform and limitations in
`docs/validation.md`; close/export the delivery Bead only after passing checks.
Re-run a failed check after fixing its cause without weakening semantic tests.
No live database is needed for this adapter-only change. Do not commit or publish.
