use std::collections::BTreeMap;

use super::*;

const FIXTURE: &str = include_str!("../../tests/fixtures/envelope-v1.txt");

fn key_id(value: &str) -> KeyId {
    KeyId::new(value).unwrap()
}

fn keyring(current: &str, entries: &[(&str, u8)]) -> Keyring {
    Keyring::new(
        key_id(current),
        entries
            .iter()
            .map(|(id, byte)| (key_id(id), SecretKey::from_bytes([*byte; SECRET_KEY_BYTES]))),
    )
    .unwrap()
}

fn row_context() -> Context {
    Context::for_row(
        "example-service",
        "private-record",
        b"account-42",
        b"record-17",
        "record-bytes-v1",
    )
    .unwrap()
}

#[test]
fn seal_open_round_trip_empty_and_nonempty_plaintext_with_fresh_randomness() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = row_context();

    for plaintext in [b"".as_slice(), b"sensitive payload".as_slice()] {
        let first = keyring.seal(&context, plaintext).unwrap();
        let second = keyring.seal(&context, plaintext).unwrap();

        assert_ne!(first.encode(), second.encode());
        assert_eq!(
            keyring.open(&context, first.as_ref()).unwrap().as_slice(),
            plaintext
        );
        assert_eq!(
            keyring.open(&context, second.as_ref()).unwrap().as_slice(),
            plaintext
        );
    }
}

#[test]
fn every_row_context_component_is_authenticated() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let sealed = keyring.seal(&row_context(), b"sensitive payload").unwrap();
    let mismatches = [
        Context::for_row(
            "another",
            "private-record",
            b"account-42",
            b"record-17",
            "record-bytes-v1",
        )
        .unwrap(),
        Context::for_row(
            "example-service",
            "other-purpose",
            b"account-42",
            b"record-17",
            "record-bytes-v1",
        )
        .unwrap(),
        Context::for_row(
            "example-service",
            "private-record",
            b"account-43",
            b"record-17",
            "record-bytes-v1",
        )
        .unwrap(),
        Context::for_row(
            "example-service",
            "private-record",
            b"account-42",
            b"record-18",
            "record-bytes-v1",
        )
        .unwrap(),
        Context::for_row(
            "example-service",
            "private-record",
            b"account-42",
            b"record-17",
            "record-bytes-v2",
        )
        .unwrap(),
    ];

    for mismatch in &mismatches {
        assert_eq!(
            keyring.open(mismatch, sealed.as_ref()).unwrap_err(),
            Error::AuthenticationFailed
        );
    }
}

#[test]
fn tenant_token_binding_authenticates_tenant_and_schema_and_differs_from_row_binding() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = Context::for_tenant_token(
        "example-service",
        "external-token",
        b"account-42",
        "token-v1",
    )
    .unwrap();
    let sealed = keyring.seal(&context, b"bearer-token").unwrap();
    assert_eq!(context.encoded(), fixture()["tenant_context"]);
    assert_eq!(
        keyring.open(&context, sealed.as_ref()).unwrap().as_slice(),
        b"bearer-token"
    );

    for mismatch in [
        Context::for_tenant_token("another", "external-token", b"account-42", "token-v1").unwrap(),
        Context::for_tenant_token(
            "example-service",
            "other-purpose",
            b"account-42",
            "token-v1",
        )
        .unwrap(),
        Context::for_tenant_token(
            "example-service",
            "external-token",
            b"account-43",
            "token-v1",
        )
        .unwrap(),
        Context::for_tenant_token(
            "example-service",
            "external-token",
            b"account-42",
            "token-v2",
        )
        .unwrap(),
        Context::for_row(
            "example-service",
            "external-token",
            b"account-42",
            b"token-row",
            "token-v1",
        )
        .unwrap(),
    ] {
        assert_eq!(
            keyring.open(&mismatch, sealed.as_ref()).unwrap_err(),
            Error::AuthenticationFailed
        );
    }
}

#[test]
fn wrong_and_unavailable_master_keys_fail_without_plaintext() {
    let context = row_context();
    let original = keyring("primary", &[("primary", 7)]);
    let sealed = original.seal(&context, b"sensitive payload").unwrap();

    assert_eq!(
        keyring("primary", &[("primary", 8)])
            .open(&context, sealed.as_ref())
            .unwrap_err(),
        Error::AuthenticationFailed
    );
    assert_eq!(
        keyring("replacement", &[("replacement", 9)])
            .open(&context, sealed.as_ref())
            .unwrap_err(),
        Error::KeyUnavailable
    );
}

#[test]
fn descriptor_wrapper_and_body_substitution_or_tampering_is_rejected() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = row_context();
    let first = keyring.seal(&context, b"first payload").unwrap();
    let second = keyring.seal(&context, b"second payload").unwrap();

    let descriptor_swap = Envelope::from_parts(
        second.envelope().content_descriptor().clone(),
        first.envelope().wrapped_key().clone(),
    );
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(&descriptor_swap, first.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );

    let wrapper_swap = Envelope::from_parts(
        first.envelope().content_descriptor().clone(),
        second.envelope().wrapped_key().clone(),
    );
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(&wrapper_swap, first.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(first.envelope(), second.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );

    let mut descriptor_bytes = first.envelope().content_descriptor().encode();
    descriptor_bytes[2] ^= 1;
    let tampered_descriptor = ContentDescriptor::decode(&descriptor_bytes).unwrap();
    let tampered_envelope =
        Envelope::from_parts(tampered_descriptor, first.envelope().wrapped_key().clone());
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(&tampered_envelope, first.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );

    let mut wrapped_bytes = first.envelope().wrapped_key().encode();
    let last = wrapped_bytes.len() - 1;
    wrapped_bytes[last] ^= 1;
    let tampered_wrapper =
        WrappedKey::decode_for(first.envelope().content_descriptor(), &wrapped_bytes).unwrap();
    let tampered_envelope = first.envelope().with_wrapped_key(tampered_wrapper);
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(&tampered_envelope, first.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );

    let mut tampered_body = first.ciphertext().to_vec();
    tampered_body[0] ^= 1;
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(first.envelope(), &tampered_body).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );
}

#[test]
fn wrapper_key_id_substitution_fails_even_when_both_ids_have_identical_key_bytes() {
    let keyring = keyring("primary", &[("primary", 7), ("another", 7)]);
    let context = row_context();
    let sealed = keyring.seal(&context, b"payload").unwrap();
    let mut wrapped_bytes = sealed.envelope().wrapped_key().encode();
    assert_eq!(wrapped_bytes[0], 7);
    wrapped_bytes[1..8].copy_from_slice(b"another");
    let substituted =
        WrappedKey::decode_for(sealed.envelope().content_descriptor(), &wrapped_bytes).unwrap();
    let substituted_envelope = sealed.envelope().with_wrapped_key(substituted);

    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(&substituted_envelope, sealed.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );
}

#[test]
fn same_key_rewrap_authenticates_and_returns_the_exact_wrapper() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = row_context();
    let sealed = keyring.seal(&context, b"payload").unwrap();

    let replacement = keyring.rewrap(&context, sealed.envelope()).unwrap();
    assert_eq!(replacement, *sealed.envelope().wrapped_key());
    let mut failing = FailingRandom;
    assert_eq!(
        keyring
            .rewrap_with_random(&context, sealed.envelope(), &mut failing)
            .unwrap(),
        *sealed.envelope().wrapped_key()
    );

    let mut encoded = sealed.envelope().wrapped_key().encode();
    let last = encoded.len() - 1;
    encoded[last] ^= 1;
    let tampered =
        WrappedKey::decode_for(sealed.envelope().content_descriptor(), &encoded).unwrap();
    let tampered_envelope = sealed.envelope().with_wrapped_key(tampered);
    assert_eq!(
        keyring.rewrap(&context, &tampered_envelope).unwrap_err(),
        Error::AuthenticationFailed
    );
}

#[test]
fn rewrap_rejects_wrong_context_and_an_unavailable_source_key() {
    let old = keyring("old", &[("old", 1)]);
    let rotating = keyring("new", &[("old", 1), ("new", 2)]);
    let new_only = keyring("new", &[("new", 2)]);
    let context = row_context();
    let sealed = old.seal(&context, b"payload").unwrap();
    let wrong_context = Context::for_row(
        "example-service",
        "private-record",
        b"account-42",
        b"record-18",
        "record-bytes-v1",
    )
    .unwrap();

    assert_eq!(
        rotating
            .rewrap(&wrong_context, sealed.envelope())
            .unwrap_err(),
        Error::AuthenticationFailed
    );
    assert_eq!(
        new_only.rewrap(&context, sealed.envelope()).unwrap_err(),
        Error::KeyUnavailable
    );
}

#[test]
fn different_key_rewrap_changes_only_metadata_and_supports_split_storage() {
    let old = keyring("old", &[("old", 1)]);
    let rotating = keyring("new", &[("old", 1), ("new", 2)]);
    let new_only = keyring("new", &[("new", 2)]);
    let context = row_context();
    let sealed = old
        .seal(&context, b"payload held in object storage")
        .unwrap();
    let descriptor_before = sealed.envelope().content_descriptor().encode();
    let body_before = sealed.ciphertext().to_vec();

    let replacement = rotating.rewrap(&context, sealed.envelope()).unwrap();
    let another_replacement = rotating.rewrap(&context, sealed.envelope()).unwrap();
    assert_eq!(replacement.key_id().as_str(), "new");
    assert_ne!(replacement, *sealed.envelope().wrapped_key());
    assert_ne!(replacement, another_replacement);
    let rotated_envelope = sealed.envelope().with_wrapped_key(replacement);

    assert_eq!(
        rotated_envelope.content_descriptor().encode(),
        descriptor_before
    );
    assert_eq!(sealed.ciphertext(), body_before);
    assert_eq!(
        new_only
            .open(
                &context,
                SealedPayloadRef::new(&rotated_envelope, &body_before).unwrap(),
            )
            .unwrap()
            .as_slice(),
        b"payload held in object storage"
    );
    assert_eq!(
        old.open(
            &context,
            SealedPayloadRef::new(&rotated_envelope, &body_before).unwrap(),
        )
        .unwrap_err(),
        Error::KeyUnavailable
    );

    let decoded_descriptor = ContentDescriptor::decode(&descriptor_before).unwrap();
    let decoded_wrapper = WrappedKey::decode_for(
        &decoded_descriptor,
        &rotated_envelope.wrapped_key().encode(),
    )
    .unwrap();
    let decoded_envelope = Envelope::from_parts(decoded_descriptor, decoded_wrapper);
    assert_eq!(
        new_only
            .open(
                &context,
                SealedPayloadRef::new(&decoded_envelope, &body_before).unwrap(),
            )
            .unwrap()
            .as_slice(),
        b"payload held in object storage"
    );
}

#[test]
fn oversize_plaintext_and_randomness_failure_are_explicit() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = row_context();
    let oversized = vec![0_u8; MAX_PLAINTEXT_BYTES + 1];
    assert_eq!(
        keyring.seal(&context, &oversized).unwrap_err(),
        Error::PlaintextTooLarge
    );

    let mut failing = FailingRandom;
    assert_eq!(
        keyring
            .seal_with_random(&context, b"payload", &mut failing)
            .unwrap_err(),
        Error::RandomnessUnavailable
    );
}

#[test]
fn maximum_plaintext_size_is_accepted_exactly() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = row_context();
    let plaintext = vec![0x5a; MAX_PLAINTEXT_BYTES];

    let sealed = keyring.seal(&context, &plaintext).unwrap();
    assert_eq!(sealed.ciphertext().len(), MAX_PLAINTEXT_BYTES + TAG_BYTES);
    assert_eq!(
        keyring.open(&context, sealed.as_ref()).unwrap().as_slice(),
        plaintext
    );
}

#[test]
fn maximum_context_and_key_id_seal_decode_and_open() {
    let key_id = "k".repeat(crate::MAX_KEY_ID_BYTES);
    let keyring = Keyring::new(
        KeyId::new(&key_id).unwrap(),
        [(
            KeyId::new(&key_id).unwrap(),
            SecretKey::from_bytes([7_u8; SECRET_KEY_BYTES]),
        )],
    )
    .unwrap();
    let label = "x".repeat(crate::MAX_CONTEXT_LABEL_BYTES);
    let binding = vec![0x44; crate::MAX_BINDING_COMPONENT_BYTES];
    let context = Context::for_row(&label, &label, &binding, &binding, &label).unwrap();

    let encoded = keyring.seal(&context, b"payload").unwrap().encode();
    let decoded = SealedPayload::decode(&encoded).unwrap();
    assert_eq!(
        keyring.open(&context, decoded.as_ref()).unwrap().as_slice(),
        b"payload"
    );
}

#[test]
fn aes_256_gcm_matches_the_nist_known_answer() {
    let encrypted = encrypt_bytes(&[0_u8; 32], &[0_u8; 12], &[], &[0_u8; 16]).unwrap();
    assert_eq!(
        encrypted,
        hex("cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919")
    );
    assert_eq!(
        decrypt_bytes(&[0_u8; 32], &[0_u8; 12], &[], &encrypted)
            .unwrap()
            .as_slice(),
        &[0_u8; 16]
    );
}

#[test]
fn hkdf_sha256_matches_rfc_5869_appendix_a_one() {
    let ikm = vec![0x0b; 22];
    let salt = hex("000102030405060708090a0b0c");
    let info = hex("f0f1f2f3f4f5f6f7f8f9");
    let mut output = [0_u8; 42];
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(&info, &mut output)
        .unwrap();

    assert_eq!(
        output.as_slice(),
        hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
    );
}

#[test]
fn deterministic_envelope_matches_the_independent_node_fixture() {
    let fixture = fixture();
    let master_key: [u8; SECRET_KEY_BYTES] = fixture["master_key"].as_slice().try_into().unwrap();
    let key_id = std::str::from_utf8(&fixture["key_id"]).unwrap();
    let keyring = Keyring::new(
        KeyId::new(key_id).unwrap(),
        [(
            KeyId::new(key_id).unwrap(),
            SecretKey::from_bytes(master_key),
        )],
    )
    .unwrap();
    let context = row_context();
    assert_eq!(context.encoded(), fixture["context"]);
    let mut random = FixedRandom::new(
        [
            fixture["data_key"].as_slice(),
            fixture["payload_nonce"].as_slice(),
            fixture["wrapping_nonce"].as_slice(),
        ]
        .concat(),
    );

    let sealed = keyring
        .seal_with_random(&context, &fixture["plaintext"], &mut random)
        .unwrap();
    assert!(random.is_exhausted());
    assert_eq!(
        sealed.envelope().content_descriptor().encode().as_slice(),
        fixture["descriptor"]
    );
    assert_eq!(
        sealed.envelope().wrapped_key().encode(),
        fixture["wrapped_key"]
    );
    assert_eq!(sealed.envelope().encode(), fixture["envelope"]);
    assert_eq!(sealed.ciphertext(), fixture["body"]);
    assert_eq!(sealed.encode(), fixture["sealed_payload"]);

    let stored_descriptor = ContentDescriptor::decode(&fixture["descriptor"]).unwrap();
    let stored_wrapper =
        WrappedKey::decode_for(&stored_descriptor, &fixture["wrapped_key"]).unwrap();
    let stored_envelope = Envelope::from_parts(stored_descriptor, stored_wrapper);
    assert_eq!(
        keyring
            .open(
                &context,
                SealedPayloadRef::new(&stored_envelope, &fixture["body"]).unwrap(),
            )
            .unwrap()
            .as_slice(),
        fixture["plaintext"]
    );

    let stored_composite = SealedPayload::decode(&fixture["sealed_payload"]).unwrap();
    assert_eq!(
        keyring
            .open(&context, stored_composite.as_ref())
            .unwrap()
            .as_slice(),
        fixture["plaintext"]
    );
}

#[test]
fn errors_never_echo_sensitive_input_values() {
    let sensitive_values = [
        "sensitive-key-id",
        "sensitive-plaintext",
        "sensitive-owner",
        "sensitive-resource",
    ];
    for error in [
        Error::InvalidSecretKey,
        Error::InvalidKeyId,
        Error::DuplicateKeyId,
        Error::CurrentKeyUnavailable,
        Error::KeyUnavailable,
        Error::InvalidContext,
        Error::PlaintextTooLarge,
        Error::MalformedEncoding,
        Error::UnsupportedVersion,
        Error::TrailingData,
        Error::AuthenticationFailed,
        Error::RandomnessUnavailable,
        Error::CryptographicFailure,
    ] {
        let rendered = format!("{error} {error:?}");
        for sensitive in sensitive_values {
            assert!(!rendered.contains(sensitive));
        }
    }
}

#[test]
fn real_validation_and_open_errors_do_not_echo_rejected_values() {
    let invalid_id = "sensitive/key/id";
    let invalid_context = "Sensitive-Namespace";
    let key_error = KeyId::new(invalid_id).unwrap_err();
    let context_error = Context::for_row(
        invalid_context,
        "purpose",
        b"sensitive-owner",
        b"sensitive-resource",
        "schema",
    )
    .unwrap_err();

    let source = keyring("sensitive-key-id", &[("sensitive-key-id", 7)]);
    let context = row_context();
    let sealed = source.seal(&context, b"sensitive-plaintext").unwrap();
    let missing_error = keyring("another", &[("another", 8)])
        .open(&context, sealed.as_ref())
        .unwrap_err();
    let wrong_context = Context::for_row(
        "example-service",
        "private-record",
        b"sensitive-owner",
        b"sensitive-resource",
        "record-bytes-v1",
    )
    .unwrap();
    let authentication_error = source.open(&wrong_context, sealed.as_ref()).unwrap_err();

    let rendered = format!(
        "{key_error} {key_error:?} {context_error} {context_error:?} \
         {missing_error} {missing_error:?} {authentication_error} {authentication_error:?}"
    );
    for sensitive in [
        invalid_id,
        invalid_context,
        "sensitive-key-id",
        "sensitive-plaintext",
        "sensitive-owner",
        "sensitive-resource",
    ] {
        assert!(!rendered.contains(sensitive));
    }
}

struct FailingRandom;

impl RandomSource for FailingRandom {
    fn fill(&mut self, _destination: &mut [u8]) -> Result<(), Error> {
        Err(Error::RandomnessUnavailable)
    }
}

struct FixedRandom {
    bytes: Vec<u8>,
    position: usize,
}

impl FixedRandom {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, position: 0 }
    }

    fn is_exhausted(&self) -> bool {
        self.position == self.bytes.len()
    }
}

impl RandomSource for FixedRandom {
    fn fill(&mut self, destination: &mut [u8]) -> Result<(), Error> {
        let end = self.position + destination.len();
        let source = self
            .bytes
            .get(self.position..end)
            .ok_or(Error::RandomnessUnavailable)?;
        destination.copy_from_slice(source);
        self.position = end;
        Ok(())
    }
}

fn fixture() -> BTreeMap<&'static str, Vec<u8>> {
    FIXTURE
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, value) = line.split_once('=').unwrap();
            (name, hex(value))
        })
        .collect()
}

fn hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(pair, 16).unwrap()
        })
        .collect()
}
