# Add composable browser credential transport to `batter-axum`

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` current while implementation
proceeds. Maintain it under `.agent/PLANS.md`.

The owning delivery record is the Batter feature Bead identified in
`Owning Beads` below. Beads owns scope, acceptance, priority, dependencies, and
completion. This file records the architecture, implementation route, and
verification needed to deliver that feature. The dependent adoption Bead is a
real code migration in an existing application; it is not a review or evidence
task.

## Purpose / Big Picture

After this change, an Axum application can use Batter for the mechanics that
recur around browser-carried credentials without adopting a Batter account,
session, authorization, or CSRF-token model.

The reusable boundary has four observable capabilities:

1. Parse one named opaque cookie across every `Cookie` field while rejecting
   ambiguous duplicates and reporting malformed input without exposing values.
2. Append and clear a host-only cookie with validated name/value, `Path=/`, an
   explicit SameSite mode, correct secure/development behavior, optional
   `HttpOnly`, and an optional bounded `Max-Age`.
3. Validate a non-empty application-selected set of browser mutation signals:
   an exact operator-configured Origin or an exact custom request marker. An
   origin-only policy may select optional Fetch Metadata; every marker policy
   automatically requires strict same-origin Fetch Metadata. JSON checks remain
   optional.
4. Apply deterministic private-response headers to success and rejection
   responses without selecting the application's response body or error
   envelope.

The feature lives in `batter-axum` because it consumes and produces Axum/http
headers and is useful only at the HTTP adapter boundary. It does not belong in
the Tokio foundation. It is a collection of validated transport primitives,
not a full web-security framework and not an authentication service.

The intended consumer flow is:

    operator configuration
             |
             v
      BrowserOrigin ---------> BrowserCookie
             |                       |
             v                       v
      MutationPolicy          Cookie header/value
             |                       |
             +----------+------------+
                        |
                        v
             application auth/session code
                        |
                        v
              application error renderer
                        |
                        v
               private-response middleware

`BrowserOrigin`, cookie policy, and mutation policy are constructed before the
router begins serving. Request-time operations are read-only over those values.
Applications keep database access, credential verification, principal creation,
authorization, session rotation, and error rendering in their own crates.

## Owning Beads

- Feature Bead: `batter-browser-credential-transport-oqd` — implement the complete
  `batter-axum` browser boundary described by this plan.
- Dependent adoption Bead: `batter-adopt-browser-credential-transport-mwy` — replace the
  duplicated browser transport in one actual downstream application after the
  feature is available at a concrete Batter revision.

There is no umbrella epic. Documentation, tests, examples, validation, and
review are acceptance obligations of the actual feature and migration, not
standalone delivery records.

## Progress

- [x] (2026-09-15) Read the Batter root, foundation, and Axum adapter guides and
  the repository ExecPlan contract.
- [x] (2026-09-15) Confirmed that Batter has no cookie, browser-origin, Fetch
  Metadata, CSRF, or private-response module at Studio's pinned source revision.
- [x] (2026-09-15) Compared three real Axum application roots and separated
  repeated transport mechanics from divergent authentication and CSRF policy.
- [x] (2026-09-15) Checked open and closed Batter Beads; no existing issue owns
  this browser transport domain.
- [x] (2026-09-15) Checked current primary specifications for cookies, Fetch
  Metadata, origins, and cache-control semantics.
- [x] (2026-09-15) Checked `cookie` 0.18.2 and `url` 2.5.8 metadata and source
  contracts against Batter's Rust 1.94 minimum.
- [x] (2026-09-15) Chose one feature Bead plus one actual-consumer migration
  Bead instead of an umbrella or separate design/test/evidence tasks.
- [x] (2026-09-15) Created feature Bead
  `batter-browser-credential-transport-oqd` and dependent real-code migration
  Bead `batter-adopt-browser-credential-transport-mwy`, flushed the tracker
  export, and verified the two-node graph with `bv --robot-plan`.
- [x] (2026-09-15) Implemented the feature Bead in `batter-axum`, including
  public rustdoc, the public browser integration target, repository contracts,
  and both supported-toolchain verification and HTTP smoke matrices.
- [x] (2026-09-15) Completed six full low-threshold implementation-review
  passes and five ordinary repairs covering marker normalization, RFC 9110
  parameter parsing, strict raw origin syntax, reserved cookie-prefix cases,
  private-response referrer policy composition, same-name response-cookie
  rejection, and supporting examples.
- [x] (2026-09-15) Classified the sixth pass's browser-added marker finding as
  an adapter design deficiency under ADR-010, recorded alternatives on the
  owning Bead, and redesigned every marker policy to require strict same-origin
  Fetch Metadata automatically. The focused 33-case target, doctests, and
  strict adapter Clippy pass on the redesigned bytes.
- [x] (2026-09-15) Repeated both complete toolchain matrices and all ten rebuilt
  HTTP smoke profiles on the redesigned bytes; every command passed.
- [x] (2026-09-15) Completed all five Jig targets; evidence and the required
  verify gate are fresh with no unresolved gate.
- [x] (2026-09-15) Completed the first restarted three-reviewer pass. It found
  no implementation defect or second design mismatch; the ordinary repair
  tightened one rejection description and added direct builder-order,
  multi-parameter, and non-UTF8 classification regressions. The focused target,
  doctests, and strict adapter Clippy pass after the repair.
- [x] (2026-09-15) Completed the second restarted three-reviewer pass. Retained
  the explicit all-field non-UTF8 rejection: accepting unrelated raw bytes would
  add an availability guarantee outside the credential contract and would not
  remove related-domain cookie/header exhaustion. Added exhaustive regressions
  for every browser-controlled marker name and both forbidden prefixes.
- [x] (2026-09-15) Completed the third restarted three-reviewer pass. Documented
  the potentially-trustworthy URL/client Fetch Metadata precondition and the
  same-origin page/target requirement for non-CORS form composition; added
  origin-only strict-mode and canonical IDNA mutation regressions and verified
  the adapter-local `url`/`idna` feature graph.
- [x] (2026-09-15) Completed the final restarted pass at fingerprint
  `08ea18d441ccdc1a27407e0eacc27191e067d11df5d507b90212424ddc58e028`;
  Claude, Codex, and Cursor reported no actionable findings through low
  severity. The closure-only chronology result is recorded below.
- [x] (2026-09-15) Verified the chronology-only closure delta with all three
  reviewers at fingerprint
  `b45e37d0ac8c28fe66110e407a00bf401df495f81aa41b8b38bad53c9372bace`;
  obligation `BCT-C1` was satisfied with no actionable finding.
- [x] (2026-09-15) Leave publication/push and dependent downstream implementation
  untouched because neither is authorized by this feature-only repository task;
  only its adoption precondition note was reconciled.

## Surprises & Discoveries

- Observation: the three consumers share cookie/header mechanics, but they do
  not share one CSRF policy.
  Evidence: one consumer requires exact configured Origin, rejects explicit
  cross-site Fetch Metadata, and requires JSON; another uses cookie-to-header
  comparison; the third requires `Sec-Fetch-Site: same-origin` plus a custom
  marker header.

- Observation: malformed-cookie behavior also differs.
  Evidence: one consumer ignores malformed unrelated pairs but rejects non-UTF8
  fields and duplicate target cookies; another has separate strict and tolerant
  policies; the third returns the first matching pair from one field.

- Observation: outgoing cookie construction is duplicated and already drifting.
  Evidence: all consumers set host-only `Path=/` cookies, but expiry clearing,
  `__Host-` names, `Max-Age`, `HttpOnly`, and SameSite choices differ.

- Observation: Batter's current Axum guide says application security stays at
  the composition root.
  Evidence: `crates/batter-axum/AGENTS.md` states that boundary directly.
  Consequence: the implementation must update that guide and architecture text
  to distinguish adapter-owned browser transport mechanics from application-
  owned credential meaning and authorization.

- Observation: a permissive general cookie jar would not enforce the target
  invariant by itself.
  Evidence: `cookie::Cookie::split_parse` parses individual pairs but does not
  detect a named cookie duplicated across multiple `Cookie` fields, and its
  parser intentionally accepts syntax broader than opaque credential values.
  Consequence: Batter should use the crate for standards-aware cookie building
  and removal while retaining its own strict target-cookie scan and validation.

- Observation: the current `cookie` release is compatible with Batter's MSRV.
  Evidence: `cargo info cookie@0.18.2` reports Rust 1.56 and MIT OR Apache-2.0;
  Batter requires Rust 1.94.

- Observation: the current `url` release is compatible with Batter's MSRV.
  Evidence: `cargo info url@2.5.8` reports Rust 1.63 and MIT OR Apache-2.0;
  Batter requires Rust 1.94.

- Observation: the latest cookie standard is still in the RFC Editor queue.
  Evidence: `draft-ietf-httpbis-rfc6265bis-22` is the current IETF document at
  planning time. Claims tied to cookie prefixes and SameSite therefore need the
  exact draft revision recorded rather than being called a final RFC.

- Observation: Fetch Metadata remains a W3C Working Draft.
  Evidence: the current published document is the 2025-04-01 Working Draft.
  Consequence: unknown syntactically valid `Sec-Fetch-Site` tokens must not be
  treated as permanently invalid; strict policies may reject them, while the
  defensive `RejectCrossSite` mode ignores them as the specification advises.

- Observation: `Cache-Control: no-store` is meaningful but not a complete
  privacy guarantee.
  Evidence: RFC 9111 requires conforming caches not to store the response but
  expressly warns that compromised caches and non-HTTP application history are
  outside that guarantee.

- Observation: no standalone example endpoint is needed to prove this feature.
  Evidence: the public operations are synchronous header transformations and
  checks. Compile-tested rustdoc plus public integration tests can show actual
  Axum composition without inventing a fake account/session service.

- Observation: the existing Jig input policy already includes new files under
  `crates/*/src/**`, `crates/*/tests/**`, workspace manifests, the lockfile, and
  relevant docs.
  Consequence: no Jig input expansion is expected unless implementation creates
  a source root outside those existing patterns.

## Decision Log

- Decision: add `pub mod browser` to `batter-axum`, not a new workspace crate
  and not a foundation module.
  Rationale: every proposed public operation uses HTTP headers or Axum response
  middleware. A new crate would add package/version/facade complexity without a
  non-Axum consumer, while foundation ownership would reverse the dependency
  direction.
  Date/Author: 2026-09-15 / Codex

- Decision: define browser transport primitives, not `AuthService`,
  `SessionStore`, `Authenticator`, or an account model.
  Rationale: consumers share transport failure modes but differ in principal,
  persistence, credential, role, invitation, and revocation semantics.
  Date/Author: 2026-09-15 / Codex

- Decision: make configured target origin a validated owned value and never
  derive it from `Host`, `Forwarded`, or `X-Forwarded-*` request fields.
  Rationale: origin comparison is an authority decision. Ambient proxy metadata
  is not trusted configuration and is already separated by Batter's metadata
  design.
  Date/Author: 2026-09-15 / Codex

- Decision: support HTTPS origins plus an explicitly selected loopback-HTTP
  constructor; reject arbitrary insecure origins.
  Rationale: secure cookies and Fetch Metadata assume a trustworthy context,
  while local application development needs a narrow non-TLS path. A distinct
  constructor makes the weakened mode visible instead of hiding it behind a
  boolean.
  Date/Author: 2026-09-15 / Codex

- Decision: make every managed cookie host-only with `Path=/`; do not expose
  Domain or arbitrary Path configuration in the first API.
  Rationale: all inspected consumers use that boundary, `__Host-` semantics
  require it, and broader scoping creates sibling-domain and path-shadowing
  hazards without consumer evidence.
  Date/Author: 2026-09-15 / Codex

- Decision: require a `__Host-` cookie name for HTTPS mode and reject that prefix
  in insecure loopback mode.
  Rationale: the prefix lets supporting user agents enforce Secure, host-only,
  root-path semantics. An insecure response cannot validly establish such a
  cookie, so accepting the name in development would create an environment-
  specific silent failure.
  Date/Author: 2026-09-15 / Codex

- Decision: keep SameSite and visibility explicit rather than selecting one
  universal value.
  Rationale: inspected applications legitimately use Lax for mail-link and
  navigation flows, Strict for tighter administrative/customer surfaces, and a
  script-readable CSRF cookie alongside an HttpOnly session cookie.
  Date/Author: 2026-09-15 / Codex

- Decision: allow `SameSite=None` only in HTTPS mode.
  Rationale: current cookie semantics require Secure for None. The constructor
  should reject an invalid combination before serving rather than emit a cookie
  browsers discard.
  Date/Author: 2026-09-15 / Codex

- Decision: expose opaque unquoted cookie values and do not percent-decode them.
  Rationale: the demonstrated values are base64url or hexadecimal credentials.
  Decoding or quoted-value normalization could change credential identity, and
  general structured cookie data is outside this feature.
  Date/Author: 2026-09-15 / Codex

- Decision: distinguish malformed entire-header input, malformed unrelated
  pairs, duplicate target cookies, and invalid target values.
  Rationale: applications need to map all credential ambiguity to their own
  safe authentication error while retaining enough typed cause for tests and
  trusted diagnostics. Raw values must never appear in those errors.
  Date/Author: 2026-09-15 / Codex

- Decision: provide `Strict` and `TargetOnly` handling for malformed unrelated
  pairs, but always reject non-UTF8 fields and duplicate target cookies.
  Rationale: this covers the two demonstrated compatibility policies without
  permitting a non-UTF8 field to conceal another copy of the target cookie.
  Date/Author: 2026-09-15 / Codex

- Decision: append `Set-Cookie` fields rather than insert them.
  Rationale: a response may establish or clear more than one independent cookie.
  Insertion would silently discard a previously produced field. Before append,
  reject an existing field with the same case-sensitive cookie name because the
  server profile forbids ambiguous same-name output.
  Date/Author: 2026-09-15 / Codex

- Decision: clearing emits both `Max-Age=0` and a fixed past `Expires`, retaining
  the original Path, SameSite, Secure, and HttpOnly attributes.
  Rationale: this is broadly understood by old and current user agents and
  targets exactly the cookie scope created by the corresponding set operation.
  Date/Author: 2026-09-15 / Codex

- Decision: use `cookie` 0.18.2 internally with default features disabled, but
  do not re-export its types.
  Rationale: the crate supplies standards-aware building/removal while Batter
  owns stricter credential invariants and public API stability. Avoiding a
  re-export prevents a transitive dependency version from becoming Batter's
  public type contract.
  Date/Author: 2026-09-15 / Codex

- Decision: use `url` 2.5.8 internally and do not re-export `Url`.
  Rationale: WHATWG URL parsing and origin serialization are easy to get wrong.
  Batter needs a narrow validated origin value, not a general URL abstraction.
  Date/Author: 2026-09-15 / Codex

- Decision: name the policy `MutationPolicy`, not `CsrfProtection`.
  Rationale: the checks validate browser request signals. They do not prove user
  intent, authenticate the caller, repair unsafe GET semantics, or protect
  client-side code that willingly sends authorized requests to attacker-chosen
  destinations.
  Date/Author: 2026-09-15 / Codex

- Decision: require at least one primary mutation signal at construction:
  exact configured Origin or one exact custom marker header.
  Rationale: a policy containing only optional defense-in-depth checks can
  silently accept requests when those headers are absent. Non-empty construction
  makes the primary boundary visible.
  Date/Author: 2026-09-15 / Codex

- Decision: allow both primary signals to be required together.
  Rationale: some applications may want exact Origin plus a non-simple custom
  header. Requiring all configured checks is simpler and safer than implicit OR
  semantics.
  Date/Author: 2026-09-15 / Codex

- Decision: keep Fetch Metadata modes explicit as `RejectCrossSite` and
  `RequireSameOrigin`.
  Rationale: the first is a compatible defense-in-depth guard that permits
  missing/unknown values; the second is the stricter behavior demonstrated by a
  consumer. Treating them as one boolean would obscure absence and same-site
  policy.
  Date/Author: 2026-09-15 / Codex

- Decision: every policy containing a custom marker automatically requires
  `RequireSameOrigin`; adding a marker strengthens an already configured
  `RejectCrossSite` check, and later Fetch Metadata configuration is rejected as
  a duplicate.
  Rationale: the terminal fifth-repair review demonstrated that a user agent can
  attach `Upgrade-Insecure-Requests: 1` to a navigation even though the name is
  neither CORS-safelisted nor browser-forbidden. A finite header blacklist cannot
  prove script provenance or preflight, and documenting another caller
  obligation would preserve a fragile marker-only protected path. Exact
  same-origin Fetch Metadata independently rejects the demonstrated cross-site
  navigation and fails closed when the field is absent. Non-browser clients can
  still forge both signals, so authentication and CORS remain application-owned.
  Date/Author: 2026-09-15 / Codex

- Decision: support an optional exact `application/json` media-type check and
  accept parameters after the media type.
  Rationale: disallowing browser-simple content types is useful for JSON APIs,
  but not every browser session endpoint consumes JSON. Parameters such as
  charset do not change the base media type.
  Date/Author: 2026-09-15 / Codex

- Decision: do not include double-submit comparison or token generation in the
  first feature.
  Rationale: naive double-submit is vulnerable to cookie injection, while a
  signed/session-bound design requires application secret and session semantics.
  The cookie and exact-header primitives let an application implement its chosen
  scheme without Batter blessing an incomplete protocol.
  Date/Author: 2026-09-15 / Codex

- Decision: expose a pure policy check, not a fixed-rejection middleware.
  Rationale: application error bodies, codes, request IDs, and response severity
  remain composition-root policy. A pure typed result can be used from an
  extractor or middleware without forcing a Batter wire envelope.
  Date/Author: 2026-09-15 / Codex

- Decision: provide private-response header mutation plus a thin Axum middleware.
  Rationale: the successful and short-circuit response path share the same
  deterministic header operation, and no application-specific body or status is
  selected.
  Date/Author: 2026-09-15 / Codex

- Decision: do not add CORS, HSTS, CSP, COOP/COEP/CORP, bearer parsing, OAuth,
  session storage, or token cryptography.
  Rationale: those mechanisms have different owners and deployment assumptions;
  none is required to remove the demonstrated duplicate boundary.
  Date/Author: 2026-09-15 / Codex

- Decision: do not block this feature on the facade extraction tasks.
  Rationale: `batter-axum` is already the supported adapter path. If the facade
  lands first, re-export the new module through its existing generic Axum
  namespace; otherwise the facade task can discover and re-export it later.
  Date/Author: 2026-09-15 / Codex

- Decision: use one feature Bead and one actual migration Bead.
  Rationale: the cookie, mutation, and private-response pieces form one coherent
  public boundary and one lockfile/API review. The second Bead changes real
  consumer code and proves the boundary removes duplication. Separate planning,
  review, test, evidence, and documentation beads would add ceremony without a
  distinct runtime capability.
  Date/Author: 2026-09-15 / Codex

## Outcomes & Retrospective

Implementation outcome as of 2026-09-15: `batter_axum::browser` now exposes
`BrowserOrigin`; `CookieName`, `CookieValue`, `CookieHeaderPolicy`,
`read_cookie`, and `BrowserCookie`; `RequiredHeader`, `FetchSitePolicy`,
`MutationPolicy`, and `MutationRejection`; plus
`apply_private_response_headers` and `private_response`. The implementation
kept the planned public behavior and split private source by responsibility as
`origin`, `cookie_transport`, `mutation`, and `private_response`.

The redesigned 33-case public integration target and 23 positive plus five
compile-fail adapter doctests pass, as does strict all-target adapter Clippy.
Complete Rust 1.98.1 and 1.94.0 verification matrices and all five rebuilt HTTP
smoke modes per toolchain pass on the redesigned bytes locally on macOS 26.6.2
arm64. No new Linux, hosted-CI, live-browser, or proxy execution is claimed.

The dependent downstream migration remains intentionally open. No Batter
revision has been published or otherwise pushed, no downstream mechanism has
been removed, and browser compatibility beyond the cited specifications and
executable header-level contracts remains unverified.

Review intentionally tightened the planned behaviors before release:
`RequiredHeader::new` is fallible and accepts only Fetch-stable printable-ASCII
values under a script-settable, non-safelisted name; it does not claim that the
browser received the field from script. Cookie reserved-prefix checks follow the
current case-insensitive user-agent rules and require HttpOnly for
`__Host-Http-`; strict raw `scheme://authority` origin input prevents path,
userinfo, and slash recovery from disappearing through URL normalization; and
JSON parameters follow the RFC 9110 byte grammar instead of accepting an
arbitrary tail. Valid UTF-8 in unrelated cookies is now classified by the
selected Strict/TargetOnly syntax policy rather than mislabeled non-UTF8.

The fourth review found that `Referrer-Policy: no-referrer` would make a
same-origin non-CORS form mutation carry `Origin: null`, conflicting with the
adapter's own exact-origin policy. The fixed private-response value is now
`same-origin`, which withholds cross-origin referrers while preserving the
serialized origin for a same-origin HTML form mutation.

The fifth review found that unconditional append still permitted two response
fields with the same cookie name. Set and removal now inspect existing fields
byte-wise, reject an exact case-sensitive duplicate with a sanitized typed
error, and leave the response unchanged; three operation-order regressions pin
that invariant while independent cookie names still append.

The complete pass over that repair found a recurring custom-marker invariant
failure: a browser can attach a non-safelisted, non-forbidden field such as
`Upgrade-Insecure-Requests` without script or preflight. Per ADR-010 this was
classified as an adapter design deficiency rather than repaired with another
open-ended denylist entry. `MutationPolicy::required_header` now installs strict
same-origin Fetch Metadata, and `and_required_header` strengthens any compatible
mode. Missing, same-site, cross-site, unknown, malformed, and duplicate values
therefore fail before the marker check; an explicit browser-added-header
regression pins the original failure. This root repair restarts the complete
review loop.

## Context and Orientation

The repository is a Unix-only virtual Cargo workspace. The native Tokio
foundation is in `crates/batter`; the optional Axum adapter is in
`crates/batter-axum`. The adapter currently owns request construction budgets,
lifecycle admission, readiness translation, correlation, observation, and
listener registration. It already accepts application-owned renderers for
infrastructure failures and deliberately keeps authentication and proxy trust
outside its current contract.

The new module belongs at:

    crates/batter-axum/src/browser.rs

If the file exceeds repository limits, split private implementation into:

    crates/batter-axum/src/browser.rs
    crates/batter-axum/src/browser/cookie.rs
    crates/batter-axum/src/browser/mutation.rs
    crates/batter-axum/src/browser/origin.rs
    crates/batter-axum/src/browser/private_response.rs

The public module path remains `batter_axum::browser`. Private file layout must
not leak through rustdoc or type paths.

Public integration coverage belongs at:

    crates/batter-axum/tests/browser.rs
    crates/batter-axum/tests/browser/cookies.rs
    crates/batter-axum/tests/browser/mutation.rs
    crates/batter-axum/tests/browser/origin.rs
    crates/batter-axum/tests/browser/private_response.rs

Use smaller files only where the current file-budget policy requires them. Do
not create a generic security test-support crate or a browser automation harness
for header-level behavior.

The root `Cargo.toml` owns workspace dependency versions. Add:

    cookie = { version = "0.18.2", default-features = false }
    url = { version = "2.5.8", default-features = false, features = ["std"] }

Then consume both through `workspace = true` in
`crates/batter-axum/Cargo.toml`. If `url` does not accept this exact feature
selection in the actual resolver, use its verified default `std` feature rather
than guessing a private feature combination. Do not enable cookie signing,
private jars, percent encoding, or key-expansion features.

Cargo.lock must be regenerated by Cargo. Do not hand-edit it. A dependency
change is not permission to upgrade unrelated packages.

## Existing Consumer Evidence

The consumer names and private repository paths are intentionally omitted from
public source, rustdoc, Beads, and contract documentation. The implementation
does not require access to them because this section preserves the generic
observable cases.

### Consumer A: exact-origin JSON application

Consumer A has one configured public origin and one host-only session cookie.

Production behavior:

- HTTPS is required outside explicitly enabled loopback development.
- The stored configured value is the canonical serialized origin.
- Credentials, path, query, and fragment in configuration are rejected.
- A credential-bearing mutation requires exactly one Origin field whose bytes
  equal the configured serialized origin.
- Explicit `Sec-Fetch-Site: cross-site` is rejected.
- Exactly one Content-Type field with base type `application/json` is required.
- The target session cookie is found across all Cookie fields.
- Duplicate target cookies and malformed target values are rejected.
- Unrelated cookies are ignored.
- Secure mode uses an `__Host-` session name and Secure.
- Local mode uses a distinct unprefixed name and omits Secure.
- The cookie is HttpOnly, Path `/`, SameSite Lax, and persistent for twelve
  hours.
- Clearing uses the same attributes with an empty value and zero Max-Age.
- Private routes overwrite Cache-Control, Referrer-Policy, and
  X-Content-Type-Options on every response.
- Application errors, retry headers, JSON extractor failures, and auth outcomes
  are mapped outside the browser helper.

### Consumer B: session plus cookie-to-header application

Consumer B maintains separate customer and administrator cookie names.

Production behavior:

- A session cookie is HttpOnly.
- A distinct CSRF cookie is intentionally readable by browser script.
- Secure administrative names use `__Host-`; local names do not.
- SameSite Strict is selected for these surfaces.
- Two cookies can be appended to one response.
- Clearing appends removal fields for both cookies.
- The parser rejects duplicate target cookies across fields.
- Some boundaries reject malformed unrelated pairs; another ignores them.
- Submitted CSRF headers must appear exactly once.
- Cookie and submitted token comparison is constant-time.
- The token protocol, signing/binding choice, and application error mapping are
  owned outside the generic transport.

### Consumer C: Fetch Metadata plus request-marker application

Consumer C protects session operations with two headers.

Production behavior:

- `Sec-Fetch-Site` must be exactly `same-origin`.
- A custom request marker must be present with one exact value.
- The session cookie is host-only, Secure, HttpOnly, Path `/`, and SameSite Lax.
- A sign-in response establishes the cookie.
- A revoke response clears it.
- Auth responses set Cache-Control no-store.
- Account/session/error behavior remains application-owned.

### Shared conclusion

The stable intersection is:

- validated trusted origin when origin checks are selected;
- strict named-cookie transport;
- host-only secure cookie construction and clearing;
- configurable SameSite and visibility;
- exact header multiplicity checks;
- configurable Fetch Metadata posture;
- optional JSON media-type restriction;
- private response headers;
- typed sanitized failures;
- application-owned authentication and rendering.

The stable intersection is not:

- one cookie name;
- one SameSite choice;
- one credential format;
- one CSRF token scheme;
- one status code or JSON body;
- one database session record;
- one principal or role type;
- one set of public/admin routes.

## Threat Model

The module addresses narrow server-side transport failures around ambient
browser cookies.

### Assets

- opaque session or device cookie values;
- state-changing operations authorized by an application after credential
  validation;
- trusted operator-configured browser origin;
- application response data that should not be cached or referred;
- deterministic rejection meaning for application rendering.

### In-scope attackers and faults

- a cross-site page causing a victim browser to submit ambient cookies;
- duplicate cookie fields or pairs creating parser disagreement;
- a malformed Cookie field hiding another target occurrence;
- cookie-name prefix or attribute misconfiguration;
- response construction accidentally replacing another Set-Cookie field;
- CRLF or separator injection through a server-supplied cookie value;
- a forged or duplicated Origin/custom marker/Content-Type field;
- explicit cross-site Fetch Metadata;
- middleware placement that omits private headers from short-circuit responses;
- diagnostic formatting leaking expected marker values or cookie values.

### Explicitly out of scope

- compromised same-origin JavaScript;
- client-side CSRF in which trusted code builds an attacker-selected request;
- XSS prevention;
- CORS selection and proxy configuration;
- an attacker controlling a trusted sibling domain;
- signed double-submit token design;
- synchronizer-token persistence;
- authentication, authorization, account recovery, OAuth, OIDC, or WebAuthn;
- credential entropy, hashing, rotation, storage, or revocation;
- TLS termination and HSTS deployment;
- malicious or nonconforming HTTP caches;
- browser bugs that permit forbidden headers to be forged;
- non-browser API clients that can freely construct these headers;
- request body streaming and body-size limits;
- WebSockets and cross-origin WebSocket hijacking;
- cookie Partitioned/CHIPS behavior;
- Public Suffix List policy;
- arbitrary Domain or Path cookie scoping;
- general-purpose cookie jars or signed/encrypted cookie storage.

### Security claims the API must not make

Do not claim that:

- a passed `MutationPolicy` authenticates a person;
- SameSite alone prevents CSRF;
- `Sec-Fetch-Site` is always present;
- an exact custom header is safe under arbitrary CORS configuration;
- an Origin field is trustworthy when copied from an untrusted proxy field;
- `no-store` prevents all browser history or malicious cache retention;
- HttpOnly prevents a compromised same-origin script from issuing requests;
- a cookie prefix is enforced by every historical user agent;
- clearing a client cookie revokes a server-side session;
- a constant-time string comparison would make naive double-submit safe;
- successful header validation authorizes replay or durable side effects.

## Public API Contract

Names below are the intended public contract. Implementation may make a small
name adjustment for Rust clarity or an existing repository convention, but must
record it in `Decision Log` and update every example and acceptance reference.
It must not materially widen the boundary without a new scope decision.

### Module export

Add to `crates/batter-axum/src/lib.rs`:

    pub mod browser;

Keep implementation helpers private. Re-exporting individual browser items from
the crate root is not planned; the namespace makes the security boundary visible
and avoids crowding the existing operational API.

If the facade extraction has landed when implementation starts, expose the same
module through the facade's selected `batter::axum` namespace using actual
re-exports. Do not create wrapper copies or duplicate types.

### `BrowserOrigin`

Intended shape:

    #[derive(Clone, Eq, PartialEq)]
    pub struct BrowserOrigin { /* private */ }

    impl BrowserOrigin {
        pub fn https(value: &str) -> Result<Self, BrowserOriginError>;
        pub fn loopback_http(value: &str) -> Result<Self, BrowserOriginError>;
        pub fn as_str(&self) -> &str;
        pub fn is_secure(&self) -> bool;
    }

`https` accepts only an absolute `https` URL whose parsed URL has:

- a host;
- no username;
- no password;
- root path `/` only;
- no query;
- no fragment.

It stores `url::Url::origin().ascii_serialization()`, not the input spelling.
The canonical value therefore omits the root slash, normalizes the host, and
normalizes a default port according to the URL implementation.

`loopback_http` applies the same structural checks and additionally requires:

- scheme `http`;
- an IPv4 address for which Rust `IpAddr::is_loopback()` is true;
- an IPv6 loopback address; or
- the domain `localhost` or a subdomain ending in `.localhost`.

`loopback_http` is an explicit development capability. The general parser must
not accept arbitrary plaintext hosts through a boolean option.

`BrowserOriginError` is public, non-exhaustive, implements `Error`, and never
contains the rejected input. It distinguishes enough causes to test policy:

    #[non_exhaustive]
    pub enum BrowserOriginError {
        InvalidUrl,
        MissingHost,
        UnsupportedScheme,
        InsecureNonLoopback,
        CredentialsNotAllowed,
        PathNotAllowed,
        QueryNotAllowed,
        FragmentNotAllowed,
    }

The exact variant set may combine `InvalidUrl` and `MissingHost` if `url` cannot
distinguish them without retaining the input. It must still preserve the policy
distinctions used by tests and never echo input in Display or Debug.

`Debug` for a successful `BrowserOrigin` may show the canonical origin because
it is trusted operator configuration rather than a credential. Request Origin
values must never be included in rejections or telemetry by this module.

### `CookieName`

Intended shape:

    #[derive(Clone, Eq, PartialEq, Hash)]
    pub struct CookieName { /* private */ }

    impl CookieName {
        pub fn new(value: impl Into<Box<str>>)
            -> Result<Self, CookieNameError>;
        pub fn as_str(&self) -> &str;
    }

Validate the RFC token character subset used by cookie-name. Reject:

- empty names;
- non-ASCII bytes;
- control bytes;
- space or horizontal tab;
- separators including `(`, `)`, `<`, `>`, `@`, comma, semicolon, colon,
  backslash, double quote, slash, `[`, `]`, question mark, `=`, `{`, and `}`.

Do not lowercase or otherwise normalize the name. Cookie names are compared
case-sensitively.

`CookieNameError` does not retain the invalid name. `Debug` and Display are
sanitized. A valid `CookieName` may show its name in Debug; names are not secret.

### `CookieValue`

Intended shape:

    #[derive(Clone, Copy, Eq, PartialEq)]
    pub struct CookieValue<'a>(&'a str);

    impl<'a> CookieValue<'a> {
        pub fn new(value: &'a str) -> Result<Self, CookieValueError>;
        pub fn as_str(self) -> &'a str;
    }

Validate the unquoted `cookie-octet` subset:

- ASCII `!`;
- ASCII `#` through `+`;
- ASCII `-` through `:`;
- ASCII `<` through `[`;
- ASCII `]` through `~`.

This excludes whitespace, control bytes, double quote, comma, semicolon,
backslash, DEL, and non-ASCII. It permits `=` inside a value, which preserves
padded base64 credentials. Empty values are valid at this transport layer;
credential parsing decides whether an empty value is meaningful.

Do not percent-decode, unquote, trim internal bytes, or allocate a decoded copy.

`CookieValue` uses a custom redacted Debug implementation such as
`CookieValue([REDACTED])`. `CookieValueError` contains no raw value or offset that
would encourage logging surrounding secret material.

### `CookieHeaderPolicy`

Intended shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum CookieHeaderPolicy {
        Strict,
        TargetOnly,
    }

`Strict` rejects any non-empty malformed cookie pair in any Cookie field.

`TargetOnly` ignores malformed unrelated pairs but rejects:

- a malformed pair whose apparent name equals the requested target;
- a non-UTF8 Cookie field;
- an invalid target value;
- a target duplicated in one or more fields.

Both modes ignore empty fragments between semicolons. Both modes preserve exact
case-sensitive target matching. Neither mode decodes values.

### `read_cookie`

Intended shape:

    pub fn read_cookie<'a>(
        headers: &'a HeaderMap,
        name: &CookieName,
        policy: CookieHeaderPolicy,
    ) -> Result<Option<CookieValue<'a>>, CookieHeaderError>;

The implementation scans every value returned by
`headers.get_all(header::COOKIE)`. It does not call `HeaderMap::get`, because
that would ignore additional field lines and could miss a duplicate.

For each UTF-8 field:

1. split on semicolon;
2. trim optional surrounding ASCII space/tab from each pair;
3. ignore an empty fragment;
4. split once on `=` so an opaque value may itself contain `=`;
5. apply the configured malformed-unrelated policy;
6. compare the name case-sensitively;
7. validate a target value as `CookieValue`;
8. reject if a target was already found;
9. return the one borrowed value or `None`.

If internal use of `cookie` parsing changes any of those steps, the public
behavior above wins. Do not adopt permissive jar semantics that erase duplicate
or malformed-target evidence.

Intended error shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    #[non_exhaustive]
    pub enum CookieHeaderError {
        NonUtf8,
        MalformedPair,
        DuplicateTarget,
        InvalidTargetValue,
    }

The error must not contain the Cookie field, target value, unrelated cookie
values, or a `cookie` crate error whose Display includes raw input.

### `SameSite`

Intended shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SameSite {
        Strict,
        Lax,
        None,
    }

Keep Batter's type independent from `cookie::SameSite`; convert privately.

Do not add an implicit default. The application must choose a value because
navigation and cross-site flow requirements differ.

### `CookieVisibility`

Intended shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum CookieVisibility {
        HttpOnly,
        ScriptReadable,
    }

The explicit `ScriptReadable` name documents that omission of HttpOnly is a
capability required by protocols such as cookie-to-header CSRF. Do not call the
variant `Default` or `None`.

The type does not claim that HttpOnly prevents same-origin scripts from causing
credential-bearing requests.

### `CookieLifetime` and `CookieMaxAge`

Intended shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum CookieLifetime {
        Session,
        Persistent(CookieMaxAge),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct CookieMaxAge(/* private */);

    impl CookieMaxAge {
        pub fn seconds(value: u64) -> Result<Self, CookiePolicyError>;
        pub fn get(self) -> u64;
    }

`CookieMaxAge::seconds` rejects zero and any value the selected `cookie`/time
representation cannot express without loss. It does not accept a subsecond
`Duration`, so there is no hidden rounding policy.

`Session` omits Max-Age and Expires. `Persistent` writes Max-Age in whole
seconds. Setting a persistent cookie does not create server-side expiry; the
application remains authoritative.

### `BrowserCookie`

Intended shape:

    #[derive(Clone, Eq, PartialEq)]
    pub struct BrowserCookie { /* private */ }

    impl BrowserCookie {
        pub fn new(
            origin: &BrowserOrigin,
            name: CookieName,
            same_site: SameSite,
            visibility: CookieVisibility,
        ) -> Result<Self, CookiePolicyError>;

        pub fn name(&self) -> &CookieName;
        pub fn same_site(&self) -> SameSite;
        pub fn visibility(&self) -> CookieVisibility;
        pub fn is_secure(&self) -> bool;

        pub fn append(
            &self,
            headers: &mut HeaderMap,
            value: CookieValue<'_>,
            lifetime: CookieLifetime,
        ) -> Result<(), SetCookieError>;

        pub fn append_removal(
            &self,
            headers: &mut HeaderMap,
        ) -> Result<(), SetCookieError>;
    }

`BrowserCookie::new` derives transport security from the validated origin:

- HTTPS requires a case-sensitive name beginning `__Host-`;
- loopback HTTP rejects `__Host-`;
- `SameSite::None` requires HTTPS;
- every cookie is host-only;
- every cookie uses Path `/`.

The constructor does not accept Domain or Path.

`append` builds a server response cookie with:

- exact configured name and opaque value;
- `Path=/`;
- no Domain attribute;
- exact configured SameSite;
- Secure when the origin is HTTPS;
- HttpOnly only for `CookieVisibility::HttpOnly`;
- no Max-Age/Expires for `CookieLifetime::Session`;
- positive Max-Age for `CookieLifetime::Persistent`.

Use `HeaderMap::append(header::SET_COOKIE, value)`. Do not use `insert`.

`append_removal` builds an empty-value cookie with:

- the same name;
- the same Path;
- the same SameSite;
- the same Secure choice;
- the same HttpOnly choice;
- `Max-Age=0`;
- an Expires timestamp in the past;
- no Domain.

Removal does not claim server-side revocation.

`SetCookieError` reports either an existing same-name response field or an
internal serialization failure. It must be typed and sanitized and must not
carry the cookie name or value. Do not use `expect` on a public request/response
path merely because the values were intended to be validated.

If the implementation can make append operations infallible from validated
construction without hiding allocation failure or using unsafe code, it may
return `()` instead. Record that proof in the Decision Log and keep tests which
show hostile values cannot construct `CookieValue`.

### `RequiredHeader`

Intended shape:

    #[derive(Clone)]
    pub struct RequiredHeader { /* private */ }

    impl RequiredHeader {
        pub fn new(name: HeaderName, value: HeaderValue)
            -> Result<Self, RequiredHeaderError>;
        pub fn name(&self) -> &HeaderName;
    }

The exact expected value is intentionally not exposed through a normal accessor.
Debug must show the header name and redact the expected value. This prevents a
marker that later becomes sensitive from entering ordinary diagnostics.

The check requires exactly one field occurrence and exact bytes. It does not
split comma-separated values or trim them. A combined field therefore does not
equal the configured marker unless its exact bytes were explicitly configured.

The documentation must say that accepted names are script-settable and
non-safelisted but do not prove script provenance or preflight. A user agent may
attach other accepted names itself. Every policy containing a marker supplies
the independent strict same-origin Fetch Metadata check; Batter still does not
authenticate non-browser clients or configure CORS.

### `FetchSitePolicy`

Intended shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum FetchSitePolicy {
        RejectCrossSite,
        RequireSameOrigin,
    }

`RejectCrossSite` behavior:

- no field: pass so another configured primary signal remains the fallback;
- exactly one `cross-site`: reject;
- exactly one `same-origin`, `same-site`, or `none`: pass;
- exactly one syntactically valid unknown token: pass for forward compatibility;
- malformed/non-UTF8/comma-list field: reject as ambiguous;
- more than one field: reject as ambiguous.

`RequireSameOrigin` behavior:

- exactly one `same-origin`: pass;
- missing, duplicate, other known token, unknown token, malformed, or non-UTF8:
  reject.

Comparison is ASCII case-sensitive because Fetch Metadata values are structured
field tokens with lower-case registered values. Do not silently lowercase
malformed input.

### `MutationPolicy`

Intended shape:

    #[derive(Clone)]
    pub struct MutationPolicy { /* private */ }

    impl MutationPolicy {
        pub fn exact_origin(origin: BrowserOrigin) -> Self;
        pub fn required_header(header: RequiredHeader) -> Self;
        pub fn and_exact_origin(self, origin: BrowserOrigin)
            -> Result<Self, MutationPolicyError>;
        pub fn and_required_header(self, header: RequiredHeader)
            -> Result<Self, MutationPolicyError>;
        pub fn with_fetch_site(self, policy: FetchSitePolicy)
            -> Result<Self, MutationPolicyError>;
        pub fn require_json(self) -> Result<Self, MutationPolicyError>;
        pub fn check(&self, headers: &HeaderMap)
            -> Result<(), MutationRejection>;
    }

The two constructors ensure a policy cannot be empty. Modifier calls reject a
second configuration in the same category instead of silently replacing it.
This catches accidental double configuration before serving.

`required_header` automatically installs `RequireSameOrigin`.
`and_required_header` also makes that strict policy effective, strengthening an
existing `RejectCrossSite` selection if necessary. Once a marker is present,
`with_fetch_site` rejects any later selection as duplicate configuration, so the
automatic policy cannot be weakened.

All configured checks are ANDed. There is no implicit OR/fallback policy.

`check` uses deterministic category order independent of construction order:

1. exact Origin;
2. Fetch Metadata;
3. required custom marker;
4. JSON Content-Type.

This preserves a stable rejection when several fields are wrong and matches the
demonstrated exact-origin application, which reports forbidden before unsupported
media type.

Exact-Origin behavior:

- exactly one field whose complete bytes equal `BrowserOrigin::as_str()`: pass;
- missing: reject;
- duplicate: reject;
- non-UTF8: reject;
- `null`: reject;
- a serialized origin list: reject;
- same host with a different scheme or port: reject;
- path suffix, prefix lookalike, or trailing junk: reject.

JSON behavior:

- exactly one Content-Type field is required;
- trim ASCII OWS around the base media type;
- compare `application/json` case-insensitively;
- allow parameters after the first semicolon;
- reject a missing field;
- reject duplicate field lines;
- reject non-UTF8;
- reject browser-simple types;
- reject `application/problem+json` and `application/*+json` in the first
  version because the demonstrated request contract is exact JSON;
- do not parse or consume the request body.

`MutationPolicyError` reports duplicate configuration categories without
retaining expected values.

Intended rejection shape:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    #[non_exhaustive]
    pub enum MutationRejection {
        OriginMissing,
        OriginAmbiguous,
        OriginMismatch,
        FetchSiteCrossSite,
        FetchSiteRequired,
        FetchSiteAmbiguous,
        RequiredHeaderMissing,
        RequiredHeaderAmbiguous,
        RequiredHeaderMismatch,
        JsonContentTypeMissing,
        JsonContentTypeAmbiguous,
        JsonContentTypeRequired,
    }

The exact variant names may be shortened, but callers must be able to map:

- origin/fetch/marker failures to their own forbidden response;
- content-type failures to their own unsupported-media-type response;
- all raw values to sanitized diagnostics.

Provide:

    impl MutationRejection {
        pub const fn status(&self) -> StatusCode;
        pub const fn code(&self) -> &'static str;
    }

`status` returns 403 for origin/fetch/marker failures and 415 for content-type
failures. `code` returns a small stable sanitized adapter classification. Do not
implement a domain JSON envelope in this module.

Whether `MutationRejection` implements `IntoResponse` should remain false in the
first version. Forcing callers to map the typed rejection keeps request ID,
message, body schema, retry metadata, and observation severity application-owned.

### Private response headers

Provide a pure operation:

    pub fn apply_private_response_headers(headers: &mut HeaderMap);

It overwrites these fields with exact server-selected values:

    Cache-Control: no-store
    Referrer-Policy: same-origin
    X-Content-Type-Options: nosniff

It does not add Pragma, Expires, Clear-Site-Data, CSP, HSTS, Vary, CORS, COOP,
COEP, or CORP.

Provide a thin Axum middleware:

    pub async fn private_response(
        request: Request,
        next: Next,
    ) -> Response;

It awaits the inner service, applies the pure operation to the returned response,
and returns it. It creates no span, task, deadline, or hidden state.

The middleware must apply to:

- successful handlers;
- application error responses;
- inner middleware short-circuit responses;
- fallbacks included before the layer is installed.

It cannot apply when an outer middleware returns without calling it. Rustdoc
must show the correct Axum layer order for mutation rejection and observation.

## Composition Contract

The canonical application shape is intentionally ordinary Axum composition.
The application owns the renderer:

    async fn require_browser_mutation(
        State(policy): State<MutationPolicy>,
        request: Request,
        next: Next,
    ) -> Response {
        if let Err(rejection) = policy.check(request.headers()) {
            return render_application_error(rejection, request.extensions());
        }
        next.run(request).await
    }

The application applies it only to credential-bearing mutation routes:

    let mutations = Router::new()
        .route("/session", post(change_session))
        .route("/account", patch(change_account))
        .layer(middleware::from_fn_with_state(
            mutation_policy,
            require_browser_mutation,
        ));

Private responses wrap that subrouter so short-circuit rejections receive the
headers:

    let private = Router::new()
        .merge(mutations)
        .route("/session", get(read_session))
        .layer(middleware::from_fn(private_response));

HTTP observation wraps the assembled router, matching Batter's existing
placement contract:

    let app = private.layer(middleware::from_fn(observe_http));

Exact nesting must be verified against actual Axum 0.8.9 behavior in integration
tests. Do not rely on visual method-call order without a response assertion.

The application reads credentials separately:

    let value = read_cookie(
        headers,
        browser_cookie.name(),
        CookieHeaderPolicy::TargetOnly,
    )?;

It then parses the returned opaque value into its own secret-token type and
looks up the session. `CookieHeaderError` and token parse failure normally map to
one indistinguishable invalid-credentials response.

On sign-in/session creation, the application supplies its already-generated
opaque token:

    browser_cookie.append(
        response.headers_mut(),
        CookieValue::new(token.expose())?,
        CookieLifetime::Persistent(max_age),
    )?;

On sign-out, it revokes server state under the application's transaction policy
and appends removal. Clearing alone is not successful revocation.

## API Boundaries and Ownership

### Batter owns

- origin syntax validation and canonical serialization;
- explicit HTTPS versus loopback-development mode;
- cookie name/value syntax validation;
- named-cookie scanning across all Cookie fields;
- duplicate target detection;
- malformed-unrelated policy selection;
- fixed host-only/root-path cookie building;
- `__Host-`, Secure, SameSite, HttpOnly, Max-Age, and removal coherence;
- appending Set-Cookie without erasing siblings;
- exact field multiplicity checks;
- selected Origin, Fetch Metadata, custom-marker, and JSON signal checks;
- typed sanitized transport rejections;
- private response header application;
- public rustdoc and adapter-level failure-path tests.

### Applications own

- deciding which routes are credential-bearing mutations;
- choosing the configured origin value;
- choosing names, SameSite, visibility, and lifetime;
- CORS and trusted proxy configuration;
- token generation, entropy, encoding, parsing, hashing, and redaction;
- CSRF token generation, signing, session binding, comparison, and rotation;
- credential precedence when cookie and bearer fields both appear;
- account/session persistence and transaction boundaries;
- authentication and authorization;
- principal/tenant/role request extensions;
- revocation and expiry authority;
- safe methods and whether any exceptional GET has side effects;
- response status/body/message/request ID and tracing severity;
- deciding whether malformed unrelated cookies are compatibility failures;
- downstream API/OpenAPI declarations.

### Native libraries own

- WHATWG URL parsing and canonical origin serialization (`url`);
- general cookie attribute serialization and removal timestamps (`cookie`);
- HTTP header storage/validation (`http` through Axum);
- router and middleware execution order (Axum/Tower).

### Batter explicitly does not own

- a cookie jar;
- encrypted or signed cookie storage;
- a general request-security policy engine;
- a callback graph of arbitrary checks;
- a CORS layer;
- a bearer-token extractor;
- session middleware that accesses a database;
- a principal cache;
- a generic authentication error enum;
- a secret store;
- a browser compatibility database.

## Error and Diagnostic Contract

All public error types must implement `std::error::Error` where they represent
construction or serialization failure. Request rejection enums may implement
Display and Error if useful to application mapping.

No error or Debug representation may contain:

- a Cookie header value;
- an opaque cookie value;
- an expected custom-marker value;
- an observed custom-marker value;
- an observed Origin value;
- an observed Content-Type value;
- an observed Fetch Metadata value;
- an invalid configured-origin input that might contain credentials;
- a transitive parser error containing any of those strings.

Safe diagnostic content includes:

- public enum variant names;
- the configured cookie name;
- the required header name;
- a static setting/category label;
- counts only when bounded and useful.

The module must not log automatically. Applications may record the sanitized
classification under their own trusted correlation context.

Do not attach request header values to tracing spans or events. Existing
`observe_http` remains responsible only for its sanitized method/route/status
fields.

## Detailed Behavior Matrix: Origin Construction

Each case below must be covered by a public integration test or a smaller table-
driven unit test that exercises the same public constructor.

| Input and constructor | Expected result | Reason |
| --- | --- | --- |
| `https://example.test`, `https` | accept `https://example.test` | canonical secure origin |
| `https://example.test/`, `https` | accept same canonical origin | root slash is URL syntax, not an app path |
| `https://EXAMPLE.test`, `https` | accept normalized host | URL canonicalization |
| `https://example.test:443`, `https` | accept canonical default port form | URL canonicalization |
| `https://example.test:8443`, `https` | accept with port retained | distinct exact origin |
| `https://bücher.example`, `https` | accept canonical ASCII host | URL implementation owns IDNA |
| `http://example.test`, `https` | reject | wrong scheme |
| `ftp://example.test`, `https` | reject | unsupported scheme |
| relative `/api`, `https` | reject | not absolute |
| `https:///path`, `https` | reject | no usable host |
| `https://user@example.test`, `https` | reject | credentials forbidden |
| `https://user:pass@example.test`, `https` | reject | credentials forbidden |
| `https://example.test/path`, `https` | reject | path forbidden |
| `https://example.test/?q=1`, `https` | reject | query forbidden |
| `https://example.test/#x`, `https` | reject | fragment forbidden |
| `http://localhost`, `loopback_http` | accept | explicit local development |
| `http://localhost:3000/`, `loopback_http` | accept with port | explicit local development |
| `http://app.localhost`, `loopback_http` | accept | localhost subdomain |
| `http://127.0.0.1`, `loopback_http` | accept | IPv4 loopback |
| `http://127.12.34.56`, `loopback_http` | accept if Rust parses loopback | loopback range |
| `http://[::1]`, `loopback_http` | accept | IPv6 loopback |
| `http://localhost.evil.test`, `loopback_http` | reject | suffix lookalike |
| `http://example.test`, `loopback_http` | reject | insecure non-loopback |
| `https://localhost`, `loopback_http` | reject | explicit constructor mismatch |
| `http://user@localhost`, `loopback_http` | reject | credentials forbidden |
| `http://localhost/path`, `loopback_http` | reject | path forbidden |
| `null`, either | reject | not configured web origin |
| empty, either | reject | invalid URL |

Do not snapshot exact third-party error strings. Assert Batter's sanitized typed
error and absence of the raw input from Debug/Display.

## Detailed Behavior Matrix: Cookie Name and Value

### Valid names

Accept representative values:

- `session`;
- `studio_session`;
- `identity-session`;
- `__Host-session`;
- `A`;
- digits after a non-empty start;
- all allowed punctuation individually.

Preserve case. `Session` and `session` are different target names.

### Invalid names

Reject:

- empty;
- a leading or embedded space;
- horizontal tab;
- newline;
- carriage return;
- NUL;
- Unicode;
- `name=value`;
- `name;value`;
- `name,value`;
- `name/value`;
- `name\\value`;
- `name\"value`;
- each remaining RFC separator;
- DEL.

Errors must not reproduce the invalid name if it contains control bytes.

### Valid opaque values

Accept:

- empty;
- base64url without padding;
- base64url with `=` padding;
- lower-case hexadecimal;
- upper-case hexadecimal;
- a value containing allowed punctuation;
- the minimum and a reasonably long value under the server's existing header
  size limit.

This module does not create a second header-size limit. Axum/server configuration
owns the overall request and response field bounds.

### Invalid opaque values

Reject:

- leading or trailing space;
- horizontal tab;
- newline;
- carriage return;
- NUL;
- double quote;
- comma;
- semicolon;
- backslash;
- DEL;
- Unicode;
- a quoted cookie representation;
- a percent-decoded representation containing a space.

Debug the successful wrapper and verify the source value is absent.
Debug and Display the error and verify the source value is absent.

## Detailed Behavior Matrix: Incoming Cookie Fields

Use one target called `session` in generic tests.

| Cookie fields | Mode | Expected |
| --- | --- | --- |
| no Cookie field | either | `Ok(None)` |
| `session=abc` | either | `Some("abc")` |
| `other=x; session=abc` | either | target found |
| `session=abc; other=x` | either | target found |
| `Session=abc` | either | no lower-case target |
| `session=` | either | present empty value |
| `session=a=b` | either | preserve `a=b` |
| ` session = abc ` | either | trim pair OWS and return `abc` |
| two fields, target only in second | either | target found |
| two fields, same target in both | either | duplicate error |
| same field, target twice | either | duplicate error |
| `session=abc; Session=def` | either | one exact target |
| `sessions=abc` | either | no target |
| `__Host-session=abc` for target `session` | either | no target |
| target plus malformed unrelated pair | TargetOnly | target found |
| target plus malformed unrelated pair | Strict | malformed error |
| malformed target without `=` | either | malformed error |
| unrelated pair without `=` only | TargetOnly | no target |
| unrelated pair without `=` only | Strict | malformed error |
| empty fragments `;;` | either | ignored |
| non-UTF8 field before target field | either | non-UTF8 error |
| non-UTF8 field after target field | either | non-UTF8 error |
| invalid target value with semicolon boundary | either | validate actual target fragment |
| target with quoted value | either | invalid target value |
| target containing comma | either | invalid target value |
| unrelated invalid value | TargetOnly | ignored |
| unrelated invalid value | Strict | malformed error |
| target with tab inside value | either | invalid target value |
| percent-encoded target | either | exact encoded bytes returned |

Add a regression proving the implementation scans all HeaderMap values instead
of using only `get`. Add a regression proving malformed input cannot turn two
targets into one accepted value.

Do not assert that unknown unrelated cookie syntax from all historical browsers
is accepted. The explicit `TargetOnly` policy is the compatibility escape hatch.

## Detailed Behavior Matrix: Outgoing Cookies

### Secure HttpOnly persistent cookie

Construct from:

- origin `https://example.test`;
- name `__Host-session`;
- SameSite Lax;
- HttpOnly;
- value `opaque-token`;
- Max-Age 43,200 seconds.

Assert semantically:

- one new Set-Cookie field is appended;
- name/value are exact;
- Path is `/`;
- Domain is absent;
- Secure is present;
- HttpOnly is present;
- SameSite is Lax;
- Max-Age is 43,200;
- value is not percent-decoded or re-encoded;
- no unrelated attribute is added.

Do not make attribute ordering a public contract unless the native cookie crate
requires exact output and there is a concrete consumer need.

### Secure script-readable session cookie

Construct from:

- HTTPS origin;
- `__Host-csrf`;
- SameSite Strict;
- ScriptReadable;
- session lifetime.

Assert:

- Secure present;
- HttpOnly absent;
- Max-Age absent;
- Expires absent;
- host-only/root-path invariants remain.

### Insecure loopback cookie

Construct from:

- `http://localhost:3000`;
- unprefixed `local_session`;
- SameSite Lax;
- HttpOnly.

Assert:

- Secure absent;
- host-only/root-path attributes present;
- no `__Host-` prefix;
- all other selected attributes retained.

### Invalid construction combinations

Reject before serving:

- HTTPS with unprefixed name;
- HTTPS with lower-case `__host-` lookalike;
- loopback HTTP with `__Host-` name;
- loopback HTTP with SameSite None;
- zero persistent Max-Age;
- Max-Age outside selected representation;
- invalid cookie name;
- invalid configured origin.

### Multiple response cookies

Start with one unrelated Set-Cookie field. Append session and CSRF cookies.
Assert all three fields remain separately visible through `get_all` in original
plus append order. A mutant using `insert` must fail this assertion.

### Removal

For secure and loopback policies, assert:

- empty target value;
- Max-Age exactly zero;
- Expires present and in the past;
- same name;
- same Path `/`;
- same SameSite;
- same Secure choice;
- same HttpOnly choice;
- no Domain;
- existing Set-Cookie fields retained.

Do not assert that a subsequent request omits the cookie without a real browser;
the header contract is the claim.

## Detailed Behavior Matrix: Mutation Policy Construction

Construct and accept:

- exact Origin only;
- exact required header only;
- exact Origin plus required header;
- exact Origin plus RejectCrossSite;
- exact Origin plus RequireSameOrigin;
- exact Origin plus JSON;
- exact header plus RequireSameOrigin;
- exact header plus JSON;
- all four configured categories.

Reject construction that attempts:

- a second exact Origin;
- a second required header;
- a second FetchSite policy;
- JSON requirement twice.

Modifier order must not change check order or result.

Debug must not show the required header value. Debug may name configured
categories and the trusted configured origin.

## Detailed Behavior Matrix: Exact Origin

For expected `https://example.test`:

| Request Origin fields | Expected |
| --- | --- |
| exactly `https://example.test` | pass |
| missing | `OriginMissing` |
| empty | mismatch |
| `null` | mismatch |
| `http://example.test` | mismatch |
| `https://example.test:443` raw | mismatch unless canonical bytes equal expected |
| `https://example.test:8443` | mismatch |
| `https://example.test/` | mismatch; request Origin serialization has no path |
| `https://example.test.evil` | mismatch |
| `https://evil.test https://example.test` | mismatch |
| one field plus a duplicate identical field | ambiguous |
| one matching plus one hostile field | ambiguous |
| comma-joined values | mismatch/ambiguous, never pass |
| non-UTF8 | ambiguous |

The error does not contain expected or observed raw field values.

## Detailed Behavior Matrix: Fetch Metadata

### `RejectCrossSite`

| `Sec-Fetch-Site` | Expected |
| --- | --- |
| missing | pass |
| `cross-site` | reject cross-site |
| `same-origin` | pass |
| `same-site` | pass |
| `none` | pass |
| syntactically valid future token | pass |
| `Cross-Site` | treat as unknown token and pass only if syntactically valid |
| token with whitespace | ambiguous/malformed |
| comma-list | ambiguous/malformed |
| non-UTF8 | ambiguous |
| duplicate identical fields | ambiguous |
| duplicate conflicting fields | ambiguous |

### `RequireSameOrigin`

| `Sec-Fetch-Site` | Expected |
| --- | --- |
| missing | required rejection |
| `same-origin` | pass |
| `cross-site` | reject |
| `same-site` | reject |
| `none` | reject |
| future token | reject |
| case variant | reject |
| malformed | ambiguous/reject |
| duplicate | ambiguous/reject |

Test that `RejectCrossSite` is never the only primary check because the public
constructors require Origin or an exact marker first.

## Detailed Behavior Matrix: Required Header

For header `x-browser-request` and expected value `1`:

| Request fields | Expected |
| --- | --- |
| exactly `1` | pass |
| missing | required-header missing |
| empty | mismatch |
| `0` | mismatch |
| ` 1` | mismatch |
| `1 ` | mismatch |
| one matching plus one matching duplicate | ambiguous |
| one matching plus one hostile duplicate | ambiguous |
| comma-joined `1, 1` | mismatch |
| non-UTF8 | mismatch/ambiguous, never pass |
| same value under different header name | missing |

The expected and observed values must be absent from errors and Debug output.

## Detailed Behavior Matrix: JSON Content Type

| Content-Type fields | Expected |
| --- | --- |
| exactly `application/json` | pass |
| `Application/Json` | pass |
| ` application/json ` | pass after base OWS trim |
| `application/json; charset=utf-8` | pass |
| `application/json;charset=UTF-8` | pass |
| `application/json;` | pass as one empty parameter slot |
| `application/json ; ; charset="utf-8" ;` | pass |
| quoted parameter containing `obs-text` | pass |
| `application/json; charset =utf-8` | JSON required |
| `application/json; charset= utf-8` | JSON required |
| missing | missing rejection |
| empty | JSON required |
| `text/plain` | JSON required |
| `application/x-www-form-urlencoded` | JSON required |
| `multipart/form-data; boundary=x` | JSON required |
| `application/problem+json` | JSON required |
| `application/vnd.example+json` | JSON required |
| `application/json-patch+json` | JSON required |
| `application/jsonx` | JSON required |
| `application/json, text/plain` | JSON required |
| duplicate identical fields | ambiguous |
| duplicate conflicting fields | ambiguous |
| non-UTF8 | ambiguous |

No test should imply that a passing media type alone prevents request forgery.

## Detailed Behavior Matrix: Check Precedence

When every configured field is invalid, assert Origin rejection wins.

When Origin passes but Fetch Metadata is cross-site and marker/content type are
invalid, assert Fetch Metadata rejection wins.

When Origin and Fetch Metadata pass but the marker is absent and content type is
wrong, assert marker rejection wins.

When all preceding checks pass but content type is wrong, assert the 415-class
content rejection wins.

Build the same policy in each legal modifier order and assert the same rejection
for the same headers.

This makes policy diagnostics deterministic and prevents refactoring a builder
from changing application-visible status precedence.

## Detailed Behavior Matrix: Private Responses

Use a real Axum Router and `tower::ServiceExt::oneshot`.

Assert the exact three headers on:

- an ordinary 204 handler;
- an application 400 response;
- an inner mutation middleware 403 response;
- an inner extraction-style 415 response;
- a merged fallback response assembled before the layer;
- a response that already contains conflicting Cache-Control;
- a response that already contains a different Referrer-Policy;
- a response that already contains a different X-Content-Type-Options.

Assert overwrite, not duplicate append, for these singleton policy fields.

Assert no unrelated response headers, body, status, or extensions are changed.

Assert a deliberately outer short-circuit middleware does not receive private
headers, and document that placement limitation rather than claiming otherwise.

Compose with `observe_http` and assert:

- the browser rejection is observed once when rejection middleware is inside
  observation;
- private header application does not create a second observation;
- status remains the application's status;
- no request header value enters the completion event.

## Test Layout and Oracles

Prefer public integration tests because the feature contract is public and
consumer-facing. Use unit tests only for private character/token helpers that
would otherwise require indirect combinatorial setup.

Suggested layout:

    crates/batter-axum/tests/browser.rs
    crates/batter-axum/tests/browser/support.rs
    crates/batter-axum/tests/browser/origin.rs
    crates/batter-axum/tests/browser/cookie_input.rs
    crates/batter-axum/tests/browser/cookie_output.rs
    crates/batter-axum/tests/browser/mutation.rs
    crates/batter-axum/tests/browser/private_response.rs

`browser.rs` declares the modules and contains no duplicate helper logic.

`support.rs` may contain:

- semantic Set-Cookie attribute parsing for assertions;
- generic request construction;
- a sanitized response body helper;
- tracing capture reuse only if the existing adapter support already exposes a
  suitable helper.

Do not create a fake browser process, Selenium/Playwright dependency, database,
session store, or token service. These tests prove the HTTP header contract only.

Mutation-resistant oracles should ensure at least these mistakes fail:

- using `HeaderMap::get` instead of scanning all Cookie fields;
- accepting a second target cookie;
- using prefix matching for cookie names or origins;
- lowercasing case-sensitive cookie names;
- trimming a required marker value;
- accepting any `+json` media type;
- permitting plaintext non-loopback origin;
- emitting `__Host-` without Secure or root Path;
- adding Domain;
- using `insert` for Set-Cookie;
- clearing without matching Path/security attributes;
- treating missing Fetch Metadata as same-origin in strict mode;
- treating unknown Fetch Metadata as cross-site in compatible mode;
- logging or formatting raw values;
- installing private middleware inside a rejection layer and then claiming the
  rejection is covered.

Do not add a mutation framework or script solely to flip these lines. Ordinary
focused tests with direct negative cases are sufficient.

## Rustdoc Examples

Every public type and operation needs useful rustdoc.

At minimum provide compile-tested examples for:

1. Parse an HTTPS `BrowserOrigin`.
2. Explicitly construct a loopback HTTP origin.
3. Construct a secure HttpOnly Lax `BrowserCookie`.
4. Read one named cookie with `TargetOnly` policy.
5. Append a persistent cookie without replacing an existing Set-Cookie field.
6. Append a removal cookie.
7. Build an exact-Origin plus RejectCrossSite plus JSON `MutationPolicy`.
8. Build an exact-marker plus RequireSameOrigin policy.
9. Map `MutationRejection::status()` into an application response without a
   Batter JSON envelope.
10. Apply private-response middleware at the correct layer.

Add compile-fail examples for invalid construction only where Rust types make a
meaningful promise. Runtime validation errors belong in ordinary tests rather
than contrived compile-fail examples.

Do not place real application names, cookie names, origins, or project-specific
error codes in rustdoc. Use generic names such as `__Host-session`,
`https://app.example`, and `x-browser-request`.

## Documentation Changes

Update `crates/batter-axum/README.md` with a Browser credential transport section
that explains:

- what the module does;
- one exact-origin JSON composition;
- one exact-marker/Fetch Metadata composition;
- strict versus target-only cookie parsing;
- secure versus loopback cookie construction;
- correct private-response layer placement;
- application-owned auth/session/CSRF-token behavior;
- explicit non-goals and security limits.

Update `docs/architecture.md` so the dependency/ownership text says:

- browser header and cookie transport mechanics may live in `batter-axum`;
- credential meaning, principal construction, authorization, CORS, and
  application wire errors remain at the composition root;
- the Tokio foundation still has no HTTP dependency.

Update `docs/guarantees.md` under the HTTP boundary with exact claims:

- validated origin construction;
- exact field multiplicity;
- duplicate-aware named-cookie parsing;
- fixed host-only cookie scope;
- append/removal behavior;
- typed mutation rejection;
- private response header behavior;
- absence of authentication/CSRF/CORS guarantees.

Update `docs/status.md` with one implemented row only after code/tests pass.
Before implementation it remains not implemented. The final row should not say
“secure authentication”; it should say “browser credential transport
primitives” and name its limits.

Update `docs/testing.md` with the public behavior inventory and exact target.
Do not paste the entire case table if links and grouped claims suffice.

Update `docs/references.md` with versions and access dates for:

- `draft-ietf-httpbis-rfc6265bis-22`;
- RFC 6454 or the current WHATWG origin definition used by `url`;
- W3C Fetch Metadata Working Draft dated 2025-04-01;
- RFC 9111 no-store response semantics;
- the current Referrer Policy specification for `same-origin` and the Fetch
  Origin-header interaction for non-CORS mutations;
- the Fetch/nosniff specification owning `X-Content-Type-Options` behavior;
- `cookie` 0.18.2 source/docs;
- `url` 2.5.8 source/docs;
- Axum 0.8.9 middleware/layer behavior.

Call drafts drafts. Do not upgrade a working-draft claim into a Recommendation or
an Internet-Draft into a final RFC.

Update `CHANGELOG.md` with the additive `batter-axum::browser` API and its limits.

Update root `AGENTS.md`, `agent-map.md` only if the new module changes their code
map. Update `crates/batter-axum/AGENTS.md` because its ownership statement and
entrypoint map definitely change.

Do not add a Markdown roadmap. Open delivery state remains in the two Beads.

## Implementation Plan

### Milestone 1: establish the module and validated configuration

Read the current worktree and reconcile any facade or Axum changes made after
this planning baseline. Do not reset concurrent work.

Add workspace dependencies with exact compatible minor versions selected above.
Regenerate Cargo.lock through Cargo.

Create `browser.rs` and private submodules as needed.

Implement:

- `BrowserOrigin`;
- sanitized `BrowserOriginError`;
- `CookieName`;
- `CookieValue`;
- `SameSite`;
- `CookieVisibility`;
- `CookieMaxAge`;
- `CookieLifetime`;
- their construction errors.

Write focused constructor/value tests first. Confirm invalid input never appears
in Debug/Display before building request operations on top.

Expected observable result: an application can prepare all trusted browser
configuration before router startup, and invalid secure/development combinations
fail without serving.

### Milestone 2: implement strict cookie transport

Implement `read_cookie` over every Cookie field with exact duplicate behavior.

Use `cookie` internally only where it preserves the public behavior. Do not let
its permissive parsing decide target duplication or opaque-value validity.

Implement `BrowserCookie::new`, `append`, and `append_removal`.

Ensure response operations append Set-Cookie.

Add the full incoming and outgoing behavior matrices.

Expected observable result: applications can remove their manual `split(';')`
and string-formatting code without changing credential meaning.

### Milestone 3: implement mutation-signal policy

Implement `RequiredHeader`, `FetchSitePolicy`, `MutationPolicy`, configuration
errors, and typed rejections.

Use deterministic category order.

Do not log or render a response body.

Add construction, signal, multiplicity, forward-compatibility, redaction, and
precedence tests.

Expected observable result: an application can configure its demonstrated
browser-request signals once and reuse the same check in route middleware or an
extractor without Batter knowing the application's auth/session types.

### Milestone 4: implement private-response behavior and composition proof

Implement `apply_private_response_headers` and `private_response`.

Add actual Router tests for success, rejection, fallback, overwrites, response
preservation, and observation composition.

Add rustdoc showing correct layer placement.

Expected observable result: private headers cover inner success and
short-circuit responses while `observe_http` emits exactly one completion with
the application-selected status.

### Milestone 5: reconcile public docs and repository maps

Update public rustdoc, adapter README, architecture, guarantees, status, testing,
references, changelog, and owning guides.

Run rustdoc with warnings denied. Inspect public API output for accidental
third-party type leakage.

Expected observable result: a fresh consumer can choose and compose the API
without reading the source or any private downstream repository.

### Milestone 6: adopt in one actual consumer

This begins only after the feature is available at a concrete Batter revision.
Creating a commit, pushing, or publishing requires explicit user authorization.

Advance all coordinated Batter Git dependencies in the downstream workspace to
the same exact revision. Record the old and new pins.

Replace the local mechanisms for:

- origin parsing/canonicalization;
- exact-origin/Fetch Metadata/JSON mutation checks;
- named-cookie scanning and duplicate rejection;
- Set-Cookie construction/removal;
- private-response headers.

Preserve application-owned mechanisms for:

- public invitation-link construction;
- local/secure cookie-name selection if still needed around Batter config;
- secret-token parsing and redaction;
- auth/session database operations;
- error envelope and JSON extractor mapping;
- rate limiting and trusted client identity;
- public/admin routing policy.

Remove the superseded local trait/functions rather than leaving wrappers that
merely call Batter. A thin application-owned configuration type may contain or
return Batter values when it also owns invitation-link and environment policy.

Run the downstream unit, HTTP, API-contract, Rust, SQLx, and structured gates
required by its own repository guidance. Record exact pin adoption and preserved
contracts in its adoption documentation.

Report any friction against the owning Batter feature Bead with exact old/new
revisions and a minimal generic reproduction. Do not add a workaround or another
issue before checking the current Batter source and tracker.

Expected observable result: one actual application no longer carries its local
cookie/origin/mutation/private-response implementation and all prior HTTP
behavior remains covered.

## Concrete Implementation Steps

Work from `/Users/aa/Documents/batter` for the feature.

Before editing production source:

    git status --short
    git rev-parse HEAD
    bv --robot-triage
    CI=1 br show <feature-bead-id> --json

Claim the feature Bead only when implementation starts. Planning does not mark it
in progress.

For substantial implementation, start one Jig plan owned by the feature Bead or
adopt this ExecPlan into the repository's current Jig mechanism without creating
a separate Bead. Record the then-current Git baseline. This is an execution
mechanism, not a delivery dependency.

Edit manifests through `apply_patch`, then let Cargo regenerate Cargo.lock:

    cargo check -p batter-axum --locked

The first locked check is expected to fail after manifest edits until Cargo.lock
is regenerated. Regenerate deliberately with the repository's selected Cargo
toolchain, inspect only the intended dependency additions, then return to locked
commands. Do not call the initial expected resolution failure a product defect.

Suggested narrow loop after source exists:

    cargo fmt --all -- --check
    cargo test -p batter-axum --test browser --locked
    cargo test -p batter-axum --doc --locked
    cargo clippy -p batter-axum --all-targets --locked -- -D warnings
    RUSTDOCFLAGS='-D warnings' cargo doc -p batter-axum --no-deps --locked

Run the complete package target because the new module shares Axum/http types
with existing adapter behavior:

    cargo test -p batter-axum --all-targets --locked

Run repository-required verification after focused checks pass:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Rebuild the existing HTTP example and run every smoke profile required by the
current repository guide. At planning time those are default, SIGINT, deadline,
WARN-filter, and WARN-filter plus deadline. Use the current documented commands
if they change before implementation.

Finish with the applicable Jig work check and inspect evidence/gates. Do not
create a new evidence-only Bead if an environmental prerequisite is absent;
record the limitation on the owning feature.

Do not commit, push, publish, or deploy without a separate explicit request.

## Validation and Acceptance

The feature Bead is complete only when all of the following are true.

### Public boundary

- `batter_axum::browser` is public and documented.
- No new browser type is added to the Tokio foundation.
- No `cookie`, `url`, or other third-party type appears in a public signature.
- No application account/session/principal type exists in Batter.
- No global subscriber, task, runtime, or background cleanup is introduced.
- All constructors validate before serving.
- All request operations are deterministic and bounded by header input already
  accepted by the server.

### Origin

- HTTPS and explicit loopback HTTP have distinct constructors.
- Arbitrary plaintext hosts are rejected.
- credentials/path/query/fragment are rejected.
- origin serialization is canonical and exact.
- request values never select the configured target origin.
- raw invalid config/request values are absent from errors.

### Cookie input

- every Cookie field is scanned;
- target names are exact and case-sensitive;
- duplicate targets are rejected;
- target values are unquoted, un-decoded, and syntax-validated;
- Strict versus TargetOnly malformed behavior is explicit;
- non-UTF8 fields cannot conceal a target;
- errors contain no cookie values.

### Cookie output

- every cookie is host-only and Path `/`;
- HTTPS requires an exact `__Host-` prefix and Secure;
- loopback mode rejects the secure prefix;
- SameSite is explicit;
- SameSite None cannot be emitted insecurely;
- HttpOnly versus ScriptReadable is explicit;
- session versus positive persistent lifetime is explicit;
- Set-Cookie uses append;
- an existing same-name Set-Cookie field rejects set or removal without mutation;
- removal matches scope and emits Max-Age zero plus past Expires;
- hostile values cannot inject attributes or fields;
- no Domain/Partitioned/general jar API is exposed.

### Mutation policy

- construction requires exact Origin or exact custom marker;
- every marker policy automatically requires exact same-origin Fetch Metadata;
- adding a marker strengthens an existing compatible Fetch Metadata check;
- all configured checks must pass;
- duplicate configuration is rejected;
- exact field multiplicity is enforced;
- Origin/Fetch/marker/content precedence is deterministic;
- RejectCrossSite and RequireSameOrigin have distinct absence/unknown behavior;
- JSON accepts parameters but not suffix media types;
- rejections expose safe status/code categories and no raw values;
- application rendering remains outside Batter;
- docs state that the policy is not authentication or a complete CSRF proof.

### Private response

- the pure header function writes the exact three selected fields;
- the middleware preserves status, body, extensions, and unrelated fields;
- inner success/rejections/fallbacks are covered;
- outer short-circuit limitation is documented;
- observation composition emits one event and no header values.

### Dependency and repository integrity

- `cookie` and `url` versions/MSRV/licenses are recorded;
- optional cookie crypto/encoding features remain disabled;
- Cargo.lock contains no unrelated broad upgrade;
- all new source/test files remain in existing Jig input scopes or those scopes
  are deliberately updated together;
- adapter/root guides and code maps agree;
- guarantees/status/testing/references/changelog agree with actual code;
- no Markdown backlog is added;
- the owning Bead contains actual feature acceptance rather than process steps.

### Executed evidence

- focused public browser tests pass;
- adapter doctests pass;
- rustdoc warnings are denied and pass;
- strict adapter Clippy passes;
- complete batter-axum all-target tests pass;
- both required Rust toolchain verification matrices pass;
- the existing HTTP example is rebuilt and all required smoke modes pass;
- applicable Jig gates have a fresh successful receipt;
- executed platform/toolchain limits are recorded honestly.

The dependent adoption Bead is complete only when:

- a concrete Batter revision contains the feature;
- every coordinated Batter dependency uses that same revision;
- old and new pins are recorded;
- duplicated local origin/cookie/mutation/private-response mechanics are removed;
- application auth/session/rendering/rate-limit policy remains local;
- existing observable public/admin behavior is preserved;
- the consumer's complete required checks pass;
- adoption friction is recorded on the owning Batter feature;
- no compatibility shim remains merely to defer cleanup.

## Idempotence and Recovery

Constructors and tests are deterministic and safe to rerun.

If manifest resolution selects unexpected unrelated upgrades, stop and inspect
Cargo.lock. Re-run the narrowest Cargo update that adds only the selected direct
dependencies. Do not restore the lockfile destructively over user work.

If the facade tasks land concurrently, rebase the module export onto their
actual source structure while preserving one type identity. Do not duplicate the
module in facade and adapter.

If the `cookie` crate cannot satisfy a required attribute contract at the exact
version, retain the public Batter contract and choose the smallest private
serialization supplement. Do not re-export the crate or silently weaken tests.
Record the mismatch in `Surprises & Discoveries` and the owning Bead.

If current `url` serialization differs from an assumed test spelling, inspect
the current primary contract and adjust the expectation to canonical URL origin
semantics. Do not fall back to string-prefix validation.

If a browser field case reveals a legitimate policy divergence, prefer a named
enum mode with precise behavior over a boolean or closure callback. Do not add a
general policy DSL.

If the downstream consumer needs a behavior not in the feature acceptance:

1. preserve the old behavior locally long enough to identify the exact gap;
2. verify Batter source and open/closed tracker state;
3. add generic reproduction evidence to the owning feature;
4. amend the API only when it is shared transport policy;
5. keep application-specific meaning downstream;
6. then complete the hard cutover and remove the temporary local path.

Do not create a dummy component, fake auth service, or no-op route to satisfy an
example or lifecycle report.

No commit, push, publication, deployment, or production resource creation is
authorized by this plan.

## Interface Sketch

The intended consumer imports are:

    use batter_axum::browser::{
        BrowserCookie,
        BrowserOrigin,
        CookieHeaderPolicy,
        CookieLifetime,
        CookieMaxAge,
        CookieName,
        CookieValue,
        CookieVisibility,
        FetchSitePolicy,
        MutationPolicy,
        MutationRejection,
        RequiredHeader,
        SameSite,
        apply_private_response_headers,
        private_response,
        read_cookie,
    };

Expected application configuration:

    struct BrowserConfig {
        origin: BrowserOrigin,
        session_cookie: BrowserCookie,
        mutation: MutationPolicy,
    }

Expected exact-origin JSON construction:

    let origin = BrowserOrigin::https("https://app.example")?;
    let session_cookie = BrowserCookie::new(
        &origin,
        CookieName::new("__Host-session")?,
        SameSite::Lax,
        CookieVisibility::HttpOnly,
    )?;
    let mutation = MutationPolicy::exact_origin(origin.clone())
        .with_fetch_site(FetchSitePolicy::RejectCrossSite)?
        .require_json()?;

Expected marker construction:

    let marker = RequiredHeader::new(
        HeaderName::from_static("x-browser-request"),
        HeaderValue::from_static("1"),
    )?;
    let mutation = MutationPolicy::required_header(marker);

Expected credential read:

    let token = read_cookie(
        &headers,
        session_cookie.name(),
        CookieHeaderPolicy::TargetOnly,
    )?
    .ok_or(ApplicationAuthError::InvalidCredentials)?;
    let token = ApplicationToken::parse(token.as_str())?;

Expected set:

    let max_age = CookieMaxAge::seconds(43_200)?;
    session_cookie.append(
        response.headers_mut(),
        CookieValue::new(token.expose())?,
        CookieLifetime::Persistent(max_age),
    )?;

Expected clear:

    session_cookie.append_removal(response.headers_mut())?;

Expected application rejection mapping:

    let result = config.mutation.check(request.headers());
    if let Err(rejection) = result {
        return application_error(
            rejection.status(),
            rejection.code(),
            request.extensions(),
        );
    }

These sketches are not permission to introduce application types into Batter.

## Dependency Graph

The delivery graph contains two nodes:

    browser feature in batter-axum
                 |
                 v
    actual downstream hard-cutover adoption

The downstream task depends on the feature because a Git consumer cannot pin an
unavailable revision. The feature does not depend on adoption to begin or to pass
its own public contract; adoption is the proof that the API removes an existing
implementation rather than adding an unused abstraction.

There is no hard dependency on:

- facade extraction;
- trusted request metadata work;
- Runlimit composition;
- provider delivery work;
- OpenAPI/client generation;
- SQLx verification;
- process ownership;
- publication.

If facade extraction changes the namespace first, it is a coordination surface,
not a reason to serialize unrelated implementation.

## Primary Sources Checked During Planning

Record these again in `docs/references.md` at implementation time with the
actual access date and any newer revision:

- IETF HTTP State Management Mechanism,
  `draft-ietf-httpbis-rfc6265bis-22`:
  https://datatracker.ietf.org/doc/draft-ietf-httpbis-rfc6265bis/
- W3C Fetch Metadata Request Headers, Working Draft 2025-04-01:
  https://www.w3.org/TR/fetch-metadata/
- RFC 9111 HTTP Caching:
  https://datatracker.ietf.org/doc/html/rfc9111
- WHATWG URL Standard:
  https://url.spec.whatwg.org/
- Rust `cookie` crate 0.18.2 documentation:
  https://docs.rs/cookie/0.18.2/cookie/
- Rust `url` crate 2.5.8 documentation:
  https://docs.rs/url/2.5.8/url/
- Axum 0.8.9 documentation/source selected by this workspace:
  https://docs.rs/axum/0.8.9/axum/
- OWASP CSRF Prevention Cheat Sheet, used only as non-normative threat-model
  guidance for origin/custom-header/Fetch Metadata composition and the warning
  against naive double-submit:
  https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html

External documents do not override the explicit Batter API contract. If a
standard advances before implementation, compare the actual semantic change and
update this plan rather than silently changing tests.

## Review-Round Record

The planning workflow requires iterative review before Beads conversion. These
reviews are recorded here, not as Beads and not as implementation work.

### Round 1: boundary review

Question: does the proposal accidentally create a generic authentication
framework?

Result:

- Removed account/session/principal interfaces.
- Kept token parsing and storage application-owned.
- Kept wire rendering application-owned.
- Located the module in `batter-axum`, not the foundation.
- Retained only header/cookie transport behavior shared by real consumers.

Steady-state assessment: structural scope became smaller and clearer; another
round was required.

### Round 2: security-policy review

Question: does one consumer's CSRF strategy become a universal claim?

Result:

- Renamed the boundary from CSRF protection to mutation-signal policy.
- Required an explicit primary signal.
- Split Fetch Metadata into compatible and strict modes.
- Excluded naive double-submit and token generation.
- Added explicit CORS, same-origin-script, safe-method, and browser-bug limits.
- Made exact field multiplicity and redaction part of the contract.

Steady-state assessment: public behavior changed materially; another round was
required.

### Round 3: Rust/API review

Question: are invalid states rejected before serving, and does the API leak
native dependency types?

Result:

- Added validated `BrowserOrigin`, `CookieName`, `CookieValue`, and Max-Age
  witnesses.
- Replaced a secure boolean with distinct origin constructors.
- Coupled cookie security to the validated origin.
- Required `__Host-` for HTTPS and rejected it for local HTTP.
- Kept `cookie` and `url` types private.
- Made modifier duplication fallible rather than silently replacing policy.
- Kept request checks pure so application rendering stays outside the adapter.

Steady-state assessment: API shape stabilized; another round was required for
execution and dependency completeness.

### Round 4: execution/dependency review

Question: can a fresh implementation agent deliver and validate this without a
clarifying decision or fake project work?

Result:

- Added exact module, manifest, test, and documentation entrypoints.
- Added complete origin/cookie/mutation/private-response matrices.
- Added deterministic error precedence.
- Added Axum layer-order and observation composition proof.
- Selected verified dependency versions compatible with the MSRV.
- Added downstream hard-cutover mechanics and exact ownership split.
- Reduced the Beads graph to the feature and one real migration.
- Kept reviews, docs, tests, and evidence inside feature acceptance.

Steady-state assessment: changes were additive detail, not architectural
restructuring. The plan is ready for Beads conversion.

## Self-Containment Check

The most obscure implementation slice is `FetchSitePolicy::RejectCrossSite`.
An implementer working only from that slice knows:

- the exact header name;
- the four registered values;
- how absence behaves;
- how future valid tokens behave;
- how malformed/comma/non-UTF8/duplicate fields behave;
- why unknown tokens are different from malformed fields;
- why this cannot be the only primary policy;
- which error category to return;
- which raw values must remain out of diagnostics;
- which tests prove the contract.

No human policy choice remains for that slice.

## Dependency-Graph Check

The graph is acyclic.

- The feature has no artificial prerequisite.
- The adoption depends on the feature.
- The adoption consumes and removes duplicated real code.
- Neither node is orphaned.
- Facade work is coordination only.
- Publication is an authorization boundary, not a task node.

## Justification Check

Five sampled architectural choices all have explicit rationale:

1. Adapter rather than foundation ownership: based on HTTP/Axum types and
   dependency direction.
2. Validated configured origin: prevents authority from untrusted request/proxy
   metadata.
3. Fixed host-only root-path cookies: demonstrated across consumers and avoids
   unsupported domain/path hazards.
4. Pure mutation check: preserves application wire/error ownership.
5. Excluding double-submit: avoids blessing a protocol whose security depends
   on application signing/session binding.

## Steady-State Check

Round 1 changed the feature boundary.

Round 2 changed security semantics.

Round 3 changed the public Rust API shape.

Round 4 added execution detail without changing the architecture.

The design has reached planning steady state. Implementation discoveries must be
recorded, but there is no known unresolved structural decision.
