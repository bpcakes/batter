# batter-runlimit

Optional native atomic quota-before-work execution for Batter. No default
features; `memory`, `postgres` and `axum` can be selected independently.

```sh
cargo run -p batter --features runlimit-memory,runlimit-axum --example quota_service --locked
cargo test -p batter-runlimit --all-features --locked
python3 scripts/check_runlimit_features.py
```

`Quota::run(&context, Checks::new(&checks)?, work_factory)` keeps native quota
decisions separate from the work result. Checks run once; work shares the same
total budget. Quota denial/backend failure prevents factory invocation. Cancellation
after a grant does not refund or erase consumption. Native algorithms, atomicity,
policy validation, opaque keys and persistence remain in Runlimit, not Batter.
Allowed batches yield native validated `Allowance` values. Enforced and shadow
denials retain the native index and nonzero evaluated batch size as well as the
typed denial details.

`HttpQuota::new(quota, policies, authenticate, subject)?.prepare(policy, routes)`
guards every supplied route. It rejects empty and mixed-mode policy sets at
construction. Call `.with_public_probes(PublicProbes::new().get("/live", handler)?)`
before `prepare` to deliberately add a public GET/HEAD probe. Probes bypass
lifecycle/deadline admission, authentication and quota, and receive no protected
`OperationContext`. The builder accepts only literal GET paths and
returns a typed error for capture or wildcard patterns. It cannot accept an
arbitrary Router or fallback. A public and protected GET at the same path
panic during `prepare`, before serving. Use distinct GET paths.
Handler principals arrive through the
`Authenticated<P>` extractor, not `Extension<P>`; its value cannot be constructed
outside the adapter. Authentication receives no request body, and subject
selection uses its result and the actual peer, not forwarded headers. Closure
signatures are checked at `new`. Prepared serving installs peer metadata and one
retained HTTP observer; the alternative test transport requires an explicit
synthetic peer. See the
[facade compiling example](../batter/examples/quota_service.rs), [API docs](src/http.rs) and
[integration contract](../../docs/integrations.md#runlimit-optional-protected-native-quota-adapter).

The `postgres` feature only supplies native type/error compatibility. It does not
provision, migrate, prepare or maintain a database. Live PostgreSQL, Linux and
fresh-agent consumer evaluation are not claimed by this implementation's local
tests. Response-body streaming and arbitrary spawned descendants are not covered
by the response-construction boundary. Packages remain unpublished.
