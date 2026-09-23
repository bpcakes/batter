# batter-at-rest

`batter-at-rest` is a synchronous Rust library for portable application-layer envelope encryption and independently owned stable HMAC keys. It encrypts every payload with a random 32-byte data key, encrypts that data key under a purpose-derived master key, and authenticates an explicit application context. The same header-plus-body contract works for relational columns and object storage.

The envelope side deliberately has one format and three operations: seal, open, and metadata-only rewrap. `MacKey` separately signs stable application identities with HMAC-SHA256; it does not define their message encoding or rotation policy. The crate has no SQL, object-store, configuration, async-runtime, network, serde, Batter-runtime, or application dependency.

The fixed-byte encryption and MAC keys below are **demonstration-only**. Provision
independent high-entropy keys using an operating-system cryptographic random
source and load them through your application's protected key-management path.

```rust
use batter_at_rest::{Context, KeyId, Keyring, SecretKey};

let key_id = KeyId::new("primary-2026")?;
let keyring = Keyring::new(
    key_id.clone(),
    [(key_id, SecretKey::from_bytes([7_u8; 32]))],
)?;
let context = Context::for_row(
    "example-service",
    "private-record",
    b"account-42",
    b"record-17",
    "record-bytes-v1",
)?;

let sealed = keyring.seal(&context, b"sensitive bytes")?;
let plaintext = keyring.open(&context, sealed.as_ref())?;
assert_eq!(plaintext.as_slice(), b"sensitive bytes");
# Ok::<(), batter_at_rest::Error>(())
```

Stable keyed identities use final key material explicitly. Imported bytes are
never silently derived again, and segmented input is authenticated as exact
concatenation so the consuming application remains responsible for canonical
framing:

```rust
use batter_at_rest::MacKey;

let key = MacKey::from_bytes([9_u8; 32]);
let tag = key.sign_segments([b"purpose".as_slice(), b"message".as_slice()]);
key.verify_segments([b"purpose".as_slice(), b"message".as_slice()], &tag)?;
# Ok::<(), batter_at_rest::Error>(())
```

The encoded `Envelope` contains metadata covered by authentication tags. Decoding
and construction validate structure only; `open` authenticates the wrapper and
body, while `rewrap` authenticates only the wrapper. Its `ContentDescriptor` is
immutable across rewrap and contains the format version and payload nonce. Its
`WrappedKey` contains the wrapping-key ID, wrapping nonce, and encrypted data key.
`SealedPayload` combines an envelope with the ciphertext body; `SealedPayloadRef`
lets a storage adapter decrypt borrowed split columns or an object body without
concatenating them.

See [`docs/format-v1.md`](docs/format-v1.md) for normative bytes and limits.

## Detached portability and packaging

From the Batter checkout, run the complete standalone-package gate with:

```console
scripts/check-batter-at-rest-portability.sh
```

The command requires Rustup, `jq`, Node.js, and Python 3.11 or newer (for the
standard library's TOML parser). Install the minimum toolchain once with `rustup
toolchain install 1.94.0 --profile minimal`; Rustup's minimal profile includes
Cargo.

The gate selects the exact Rust 1.94.0 minimum explicitly, copies the tracked
and unignored `crates/batter-at-rest` commit candidate to a fresh directory under
`TMPDIR` (defaulting to `/tmp`) outside the checkout and any Cargo project,
checks source-manifest structure and detached dependency metadata, runs tests
and the independent Node.js fixed-vector generator plus an all-target build,
creates a standalone lockfile and verified local Cargo
package, and repeats the tests and build against the unpacked `.crate`
artifact. A separate consumer manifest then runs public-API tests against only
the unpacked artifact, with default features and without package dev-dependencies.
Negative probes also prove that workspace-inherited metadata, dependency
source overrides, and a parent-relative dependency fail for their intended
reasons, and that crate/caller Cargo configuration cannot enter the isolated
build.

The directory is therefore the complete package unit: copy it without the
repository root manifest, lockfile, toolchain file, Cargo configuration, source,
or fixtures. The gate launches Cargo from a neutral temporary directory with
an isolated Cargo home, environment, and target directory, and refuses Cargo
configuration on that launch directory's ancestor path. In the detached
directory the corresponding manual commands are:

```console
cargo +1.94.0 test
cargo +1.94.0 check --all-targets
cargo +1.94.0 package --allow-dirty
```

Packageability is local validation, not evidence of a registry upload. Version
0.0.1 targets crates.io and is licensed under the MIT license carried in its
crate-local `LICENSE`. Consumers may store `Envelope::encode()` and
the ciphertext body in separate row columns or store `SealedPayload::encode()`
as one external object. Both layouts use the same authenticated format and
must reconstruct only through the public decoders.

## Security and lifecycle boundary

The protected data is recoverable secrets and sensitive consumer content persisted in PostgreSQL, object storage, or backups. The in-scope attacker can read one of those stores or accidentally substitute ciphertext, wrappers, or descriptors between application owners, resources, purposes, and payload schemas. The trust boundary is the trusted application process, which holds keys and plaintext, to storage adapters, which hold authenticated metadata and ciphertext. AES-256-GCM prevents plaintext recovery from a read-only disclosure and rejects accidental cross-context or cross-part substitution. HKDF-SHA256 separates wrapping keys by namespace and purpose.

Authorization must succeed before decryption. Database constraints, transactions, retention, deletion, access control, and storage encryption remain necessary; this crate does not replace them. Storage-only encryption cannot authenticate the application's owner/resource/schema meaning and does not mitigate a store disclosure at the application boundary, which is why this control exists.

Reconstruct the expected context from the authenticated principal and authoritative
application state, not solely from metadata stored alongside ciphertext. Define
canonical identifier bytes and separate purposes or resource-bound contexts for
credential slots that must not be interchangeable. Tenant-token binding does not
distinguish slots sharing tenant, namespace, purpose, and schema. Authentication
does not establish freshness: rollback-sensitive records need an independently
trusted expected revision. Schema changes require explicit old-context reads and
resealing under the new context; never reinterpret a persisted schema label.

Persist rewrap results using a compare-and-swap against the complete old envelope
or a revision covering every envelope update. Comparing only the content
descriptor misses competing rotations. Also fence obsolete key policies using a
checked generation or exclusive maintenance that excludes stale writers. Reload
both the record and current key policy after conflict. Track wrapper migration
separately from body authentication; metadata-only rewrap cannot attest to a body
it never reads.

Apply application-specific input limits before buffering and bound concurrent
synchronous crypto work by bytes and execution capacity. The crate's 64 MiB
plaintext limit is a format ceiling, not an endpoint default. Retaining encoded
input, decoded ciphertext, and plaintext can use roughly three payload-sized
buffers. Prefer borrowed split storage when available. Bound deserialization and
decompression after authentication as well.

Operators must provision a nonempty keyring, retain old keys until no wrapper or recoverable backup references them, rewrap metadata before retirement, and preserve keys needed for backup recovery. Losing a referenced key makes the payload unavailable. Stable MAC keys are a separate lifecycle: changing one changes every derived identity, and importing a replacement cannot recover identities whose original input no longer exists. Historical format migration, identity message contracts, configuration update safety, retirement audits, deletion, and recovery orchestration belong to consuming systems.

A key ID is a permanent name for one exact master-key value. Never replace the bytes behind an existing ID or reuse that ID for another key: old wrappers select solely by ID and become unavailable if the binding changes. Rotation adds a fresh ID and key, rewraps and audits references, and only then retires the old binding. The consuming configuration or secret-management owner must reject same-ID/different-key updates across process restarts; one in-memory `Keyring` cannot validate historical configurations.

Rewrap is routine key-rotation metadata work, not master-key compromise recovery: it preserves the existing data key and ciphertext body. If a master key is disclosed, an attacker with an old wrapper copy can still recover that data key after rewrap. Recovery therefore requires authorized open-and-reseal under an uncompromised master, producing a fresh data key and body, plus explicit handling of replicas, snapshots, and backups that retain old wrappers or plaintext.

Version 1 uses random 96-bit GCM nonces. A consuming system must keep wrapper encryptions below `2^32` for each derived wrapping key—that is, each master-key, namespace, and purpose combination—and rotate the master key before reaching that bound. Aliases containing identical master bytes share one budget. Seal and different-key rewrap each perform one wrapper encryption; same-key rewrap performs none. This is an operational counter across every process using the same derived key, not a per-process allowance. The crate does not keep durable counts: the consuming key-configuration and provisioning boundary must assign and enforce a conservative lifetime or traffic ceiling before enabling writers. Payload encryption uses a fresh random data key for each seal.

This crate does not protect against a compromised application process, live-memory or allocator forensics, a root-equivalent host or deployment operator, a malicious database superuser able to rewrite the complete graph and observe application behavior, a compromised build/dependency path, or data already sent to an external provider. Zeroizing directly owned buffers does not claim erasure of copies in callers, crypto libraries, allocators, kernels, storage drivers, or backups.
