use super::*;

#[test]
fn same_key_rewrap_authenticates_and_returns_the_exact_envelope() {
    let keyring = keyring("primary", &[("primary", 7)]);
    let context = row_context();
    let sealed = keyring.seal(&context, b"payload").unwrap();

    let replacement = keyring
        .rewrap_envelope(&context, sealed.envelope())
        .unwrap();
    assert_eq!(replacement, *sealed.envelope());
    assert_eq!(replacement.encode(), sealed.envelope().encode());
    let mut failing = FailingRandom;
    assert_eq!(
        keyring
            .rewrap_envelope_with_random(&context, sealed.envelope(), &mut failing)
            .unwrap(),
        *sealed.envelope()
    );
    assert_eq!(
        keyring.rewrap(&context, sealed.envelope()).unwrap(),
        *sealed.envelope().wrapped_key()
    );
    let wrong_context = Context::for_row(
        "example-service",
        "private-record",
        b"account-42",
        b"record-18",
        "record-bytes-v1",
    )
    .unwrap();
    assert_eq!(
        keyring
            .rewrap_envelope(&wrong_context, sealed.envelope())
            .unwrap_err(),
        Error::AuthenticationFailed
    );

    let mut encoded = sealed.envelope().wrapped_key().encode();
    let last = encoded.len() - 1;
    encoded[last] ^= 1;
    let tampered =
        WrappedKey::decode_for(sealed.envelope().content_descriptor(), &encoded).unwrap();
    let tampered_envelope = sealed.envelope().with_wrapped_key(tampered);
    assert_eq!(
        keyring
            .rewrap_envelope(&context, &tampered_envelope)
            .unwrap_err(),
        Error::AuthenticationFailed
    );
    let mut descriptor_bytes = sealed.envelope().content_descriptor().encode();
    descriptor_bytes[2] ^= 1;
    let tampered_descriptor = ContentDescriptor::decode(&descriptor_bytes).unwrap();
    let tampered_envelope =
        Envelope::from_parts(tampered_descriptor, sealed.envelope().wrapped_key().clone());
    assert_eq!(
        keyring
            .rewrap_envelope(&context, &tampered_envelope)
            .unwrap_err(),
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
            .rewrap_envelope(&wrong_context, sealed.envelope())
            .unwrap_err(),
        Error::AuthenticationFailed
    );
    assert_eq!(
        new_only
            .rewrap_envelope(&context, sealed.envelope())
            .unwrap_err(),
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

    let rotated_envelope = rotating
        .rewrap_envelope(&context, sealed.envelope())
        .unwrap();
    let another_replacement = rotating
        .rewrap_envelope(&context, sealed.envelope())
        .unwrap();
    assert_eq!(rotated_envelope.wrapped_key().key_id().as_str(), "new");
    assert_ne!(
        rotated_envelope.wrapped_key(),
        sealed.envelope().wrapped_key()
    );
    assert_ne!(
        rotated_envelope.wrapped_key(),
        another_replacement.wrapped_key()
    );

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

    // The old wrapper-only signature remains available for explicit split storage.
    let low_level_wrapper = rotating.rewrap(&context, sealed.envelope()).unwrap();
    let low_level_envelope = sealed.envelope().with_wrapped_key(low_level_wrapper);
    assert_eq!(
        new_only
            .open(
                &context,
                SealedPayloadRef::new(&low_level_envelope, &body_before).unwrap(),
            )
            .unwrap()
            .as_slice(),
        b"payload held in object storage"
    );
}

#[test]
fn canonical_rewrap_uses_one_fresh_nonce_and_rejects_extracted_cross_descriptor_wrapper() {
    let old = keyring("old", &[("old", 1)]);
    let rotating = keyring("new", &[("old", 1), ("new", 2)]);
    let context = row_context();
    let first = old.seal(&context, b"first").unwrap();
    let second = old.seal(&context, b"second").unwrap();
    let first_before = first.encode();
    let nonce = [0xA5; NONCE_BYTES];
    let mut random = FixedRandom::new(nonce.to_vec());

    let rotated = rotating
        .rewrap_envelope_with_random(&context, first.envelope(), &mut random)
        .unwrap();
    assert!(random.is_exhausted());
    assert_eq!(rotated.wrapped_key().nonce(), &nonce);
    assert_eq!(
        rotated.content_descriptor(),
        first.envelope().content_descriptor()
    );
    assert_eq!(first.encode(), first_before);

    let mismatched = Envelope::from_parts(
        second.envelope().content_descriptor().clone(),
        rotated.wrapped_key().clone(),
    );
    assert_eq!(
        rotating
            .open(
                &context,
                SealedPayloadRef::new(&mismatched, second.ciphertext()).unwrap(),
            )
            .unwrap_err(),
        Error::AuthenticationFailed
    );
}

#[test]
fn later_randomness_failures_and_changed_key_rewrap_return_only_errors() {
    let old = keyring("old", &[("old", 1)]);
    let rotating = keyring("new", &[("old", 1), ("new", 2)]);
    let context = row_context();
    // Exhaust the deterministic source at each of seal's three requests.
    for available in [0, SECRET_KEY_BYTES, SECRET_KEY_BYTES + NONCE_BYTES] {
        let mut random = FixedRandom::new(vec![0x42; available]);
        assert_eq!(
            old.seal_with_random(&context, b"payload", &mut random)
                .unwrap_err(),
            Error::RandomnessUnavailable
        );
        assert!(random.is_exhausted());
    }
    let sealed = old.seal(&context, b"payload").unwrap();
    let before = sealed.encode();
    assert_eq!(
        rotating
            .rewrap_envelope_with_random(&context, sealed.envelope(), &mut FailingRandom)
            .unwrap_err(),
        Error::RandomnessUnavailable
    );
    assert_eq!(sealed.encode(), before);
    assert_eq!(
        old.open(&context, sealed.as_ref()).unwrap().as_slice(),
        b"payload"
    );
}

#[test]
fn successful_rewrap_does_not_authenticate_a_corrupted_body() {
    let old = keyring("old", &[("old", 1)]);
    let rotating = keyring("new", &[("old", 1), ("new", 2)]);
    let context = row_context();
    let sealed = old.seal(&context, b"payload").unwrap();
    let mut corrupted = sealed.ciphertext().to_vec();
    corrupted[0] ^= 1;
    for keyring in [&old, &rotating] {
        let envelope = keyring
            .rewrap_envelope(&context, sealed.envelope())
            .unwrap();
        assert_eq!(
            keyring
                .open(
                    &context,
                    SealedPayloadRef::new(&envelope, &corrupted).unwrap()
                )
                .unwrap_err(),
            Error::AuthenticationFailed
        );
        assert_eq!(
            keyring
                .open(
                    &context,
                    SealedPayloadRef::new(&envelope, sealed.ciphertext()).unwrap()
                )
                .unwrap()
                .as_slice(),
            b"payload"
        );
    }
}
