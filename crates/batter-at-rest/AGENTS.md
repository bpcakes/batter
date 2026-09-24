# batter-at-rest crate guide

## Purpose

`batter-at-rest` owns Batter's portable leaf format for application-layer envelope encryption and stable MAC keys. It is synchronous and independently packageable. It knows nothing about consuming application domains, SQL, object stores, configuration files, async runtimes, or historical application ciphertext.

## Key entrypoints

- `src/lib.rs` exports the complete public boundary and format limits.
- `src/key.rs` owns master-key lifetimes, validated key IDs, and immutable keyring construction.
- `src/context.rs` owns canonical row and tenant-token application bindings.
- `src/format.rs` owns validated descriptor, wrapper, envelope, body, and composite codecs.
- `src/crypto.rs` owns HKDF-SHA256, AES-256-GCM, operating-system randomness, seal/open/rewrap, and private deterministic test seams.
- `src/mac.rs` owns imported or explicitly HKDF-derived HMAC-SHA256 keys and constant-time tag verification; consuming applications own message framing.
- `docs/format-v1.md` is the normative byte-format specification.

## Edit here for X

- Change public types or operations in the owning source module and update `README.md`.
- Change persisted bytes, AAD, key derivation, sizes, or rejection behavior only through a new format specification and version. Never reinterpret version 1.
- Add portable fixed evidence under `tests/fixtures`; keep generators or deterministic randomness test-only.
- Keep historical format readers and migration policy in the consuming application, not this crate.

## Invariants

- `Cargo.toml` must remain explicit and standalone: no workspace inheritance, `../` path, Batter/application dependency, database client, serializer, network client, or async runtime. Keep version 0.0.1, Rust 1.94, publication restricted to crates.io, `license = "MIT"`, and the crate-local copy of Batter's MIT `LICENSE` so the detached package carries its license.
- Version 1 is one fixed AES-256-GCM/HKDF-SHA256 suite. Do not add an algorithm flag, direct-encryption mode, compatibility shim, or second envelope path.
- A caller cannot construct an absent or raw context. Row contexts bind owner, resource, and payload schema; tenant-token contexts bind tenant and payload schema without an invented row ID.
- Payload AAD excludes physical storage location and wrapping-key ID. Wrapper AAD includes its key ID and the payload nonce. The two roles are domain-separated.
- Rewrap authenticates the wrapped data key and context, preserves the content descriptor and body, returns only replacement wrapper metadata, and does not claim unread body integrity.
- No raw data key or master-key accessor exists. Keyring clones share immutable secret storage. Directly owned key/plaintext buffers are zeroized where their lifecycle is clear.
- Imported MAC bytes are final key material and never receive an implicit KDF or prefix. MAC-key clones share immutable secret storage; identity encodings and rotation policy stay with callers.
- Public errors and `Debug` implementations must not disclose key bytes, plaintext, context identifiers, or source payloads.

## Common commands

- Local checks use the repository's pinned toolchain. CI installs Python 3.11 or newer, Node.js, `jq`, and exact Rust 1.94.0 for `scripts/check-batter-at-rest-portability.sh`, the complete detached-source, verified-package, unpacked-artifact, and public-consumer gate. Run that MSRV gate locally only when explicitly asked to reproduce a CI failure.
- `cargo test -p batter-at-rest`
- `cargo check -p batter-at-rest --all-targets`
- `cargo clippy -p batter-at-rest --all-targets --locked -- -D warnings`
- `node crates/batter-at-rest/tests/fixtures/generate-envelope-v1.mjs --check`
- For local detached checks, explicitly select the repository's pinned compiler; the detached copy does not inherit its toolchain file. CI's portability script selects exact Rust 1.94.0.
