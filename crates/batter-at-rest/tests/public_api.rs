use batter_at_rest::{
    BorrowedSealedPayload, Context, Envelope, Error, KeyId, Keyring, MacKey, SealedPayload,
    SealedPayloadRef, SecretKey, TAG_BYTES,
};

const FIXTURE: &str = include_str!("fixtures/envelope-v1.txt");

fn fixture_bytes(name: &str) -> Vec<u8> {
    let hex = FIXTURE
        .lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
        .expect("fixture field is present");
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn independent_consumer_can_persist_open_rewrap_and_verify_stable_macs() -> Result<(), Error> {
    let old_key_id = KeyId::new("wrapping-2025")?;
    let old_keyring = Keyring::new(
        old_key_id.clone(),
        [(old_key_id.clone(), SecretKey::from_bytes([7_u8; 32]))],
    )?;
    let context = Context::for_row(
        "example",
        "private-record",
        b"account-42",
        b"record-17",
        "record-bytes-v1",
    )?;

    let identity_mac = MacKey::from_bytes([41_u8; 32]);
    let identity = b"\x00\x00\x00\x0aaccount-42\x00\x00\x00\x09record-17";
    let stable_tag = identity_mac.sign(identity);

    let sealed = old_keyring.seal(&context, b"portable secret")?;
    let stored_bytes = sealed.encode();
    let stored = SealedPayload::decode(&stored_bytes)?;
    let borrowed = BorrowedSealedPayload::decode(&stored_bytes)?;
    let body_offset = stored_bytes.len() - sealed.ciphertext().len();
    assert_eq!(borrowed.ciphertext().len(), sealed.ciphertext().len());
    assert!(std::ptr::eq(
        borrowed.ciphertext().as_ptr(),
        stored_bytes[body_offset..].as_ptr()
    ));
    assert_eq!(borrowed.envelope(), stored.envelope());
    assert_eq!(
        old_keyring.open(&context, borrowed.as_ref())?.as_slice(),
        b"portable secret"
    );
    let opened = old_keyring.open(&context, stored.as_ref())?;
    assert_eq!(opened.as_slice(), b"portable secret");

    let stored_header = stored.envelope().encode();
    let split_envelope = Envelope::decode(&stored_header)?;
    let split_storage = SealedPayloadRef::new(&split_envelope, stored.ciphertext())?;
    let opened_from_split_storage = old_keyring.open(&context, split_storage)?;
    assert_eq!(opened_from_split_storage.as_slice(), b"portable secret");

    let wrong_owner = Context::for_row(
        "example",
        "private-record",
        b"account-99",
        b"record-17",
        "record-bytes-v1",
    )?;
    assert_eq!(
        old_keyring.open(&wrong_owner, stored.as_ref()).err(),
        Some(Error::AuthenticationFailed)
    );

    let descriptor_before = stored.envelope().content_descriptor().encode();
    let body_before = stored.ciphertext().to_vec();
    let new_key_id = KeyId::new("wrapping-2026")?;
    let rotation_keyring = Keyring::new(
        new_key_id.clone(),
        [
            (old_key_id.clone(), SecretKey::from_bytes([7_u8; 32])),
            (new_key_id.clone(), SecretKey::from_bytes([19_u8; 32])),
        ],
    )?;
    let replacement_wrapper = rotation_keyring.rewrap(&context, stored.envelope())?;
    assert_eq!(replacement_wrapper.key_id(), &new_key_id);

    let rotated_envelope = stored.envelope().with_wrapped_key(replacement_wrapper);
    assert_eq!(
        rotated_envelope.content_descriptor().encode(),
        descriptor_before
    );
    let rotated = SealedPayload::from_parts(rotated_envelope, body_before.clone())?;
    assert_eq!(rotated.ciphertext(), body_before);
    let new_only_keyring = Keyring::new(
        new_key_id.clone(),
        [(new_key_id, SecretKey::from_bytes([19_u8; 32]))],
    )?;
    let opened_after_rotation = new_only_keyring.open(&context, rotated.as_ref())?;
    assert_eq!(opened_after_rotation.as_slice(), b"portable secret");
    let old_only_keyring = Keyring::new(
        old_key_id.clone(),
        [(old_key_id, SecretKey::from_bytes([7_u8; 32]))],
    )?;
    assert_eq!(
        old_only_keyring.open(&context, rotated.as_ref()).err(),
        Some(Error::KeyUnavailable)
    );

    assert_eq!(identity_mac.sign(identity), stable_tag);
    identity_mac.verify(identity, &stable_tag)?;
    assert_eq!(
        identity_mac.verify(b"different-record", &stable_tag),
        Err(Error::AuthenticationFailed)
    );

    Ok(())
}

#[test]
fn empty_plaintext_opens_from_both_composite_decoders() -> Result<(), Error> {
    let key_id = KeyId::new("primary")?;
    let keyring = Keyring::new(
        key_id.clone(),
        [(key_id, SecretKey::from_bytes([7_u8; 32]))],
    )?;
    let context = Context::for_row(
        "example",
        "private-record",
        b"account-42",
        b"record-17",
        "record-bytes-v1",
    )?;
    let encoded = keyring.seal(&context, b"")?.encode();
    let borrowed = BorrowedSealedPayload::decode(&encoded)?;
    let owned = SealedPayload::decode(&encoded)?;
    assert_eq!(borrowed.ciphertext().len(), TAG_BYTES);
    assert_eq!(owned.ciphertext().len(), TAG_BYTES);
    assert_eq!(keyring.open(&context, borrowed.as_ref())?.as_slice(), b"");
    assert_eq!(keyring.open(&context, owned.as_ref())?.as_slice(), b"");
    Ok(())
}

#[test]
fn independent_fixture_opens_from_borrowed_canonical_encoding() -> Result<(), Error> {
    let encoded = fixture_bytes("sealed_payload");
    let expected_body = fixture_bytes("body");
    let expected_plaintext = fixture_bytes("plaintext");
    let key_id = KeyId::new("primary-2026")?;
    let master_key: [u8; 32] = fixture_bytes("master_key").try_into().unwrap();
    let keyring = Keyring::new(
        key_id.clone(),
        [(key_id, SecretKey::from_bytes(master_key))],
    )?;
    let context = Context::for_row(
        "example-service",
        "private-record",
        b"account-42",
        b"record-17",
        "record-bytes-v1",
    )?;

    let borrowed = BorrowedSealedPayload::decode(&encoded)?;
    let owned = SealedPayload::decode(&encoded)?;
    let body_offset = encoded.len() - expected_body.len();
    assert!(expected_body.len() > TAG_BYTES);
    assert_eq!(borrowed.ciphertext(), expected_body);
    assert_eq!(borrowed.envelope(), owned.envelope());
    assert!(std::ptr::eq(
        borrowed.ciphertext().as_ptr(),
        encoded[body_offset..].as_ptr()
    ));
    assert_eq!(
        keyring.open(&context, borrowed.as_ref())?.as_slice(),
        expected_plaintext
    );
    assert_eq!(
        keyring.open(&context, owned.as_ref())?.as_slice(),
        expected_plaintext
    );

    for offset in [body_offset, encoded.len() - 1] {
        let mut tampered = encoded.clone();
        tampered[offset] ^= 1;
        let borrowed_tampered = BorrowedSealedPayload::decode(&tampered)?;
        let owned_tampered = SealedPayload::decode(&tampered)?;
        for view in [borrowed_tampered.as_ref(), owned_tampered.as_ref()] {
            assert_eq!(
                keyring.open(&context, view).err(),
                Some(Error::AuthenticationFailed)
            );
        }
    }
    let wrong_context = Context::for_row(
        "example-service",
        "private-record",
        b"different-owner",
        b"record-17",
        "record-bytes-v1",
    )?;
    assert_eq!(
        keyring.open(&wrong_context, borrowed.as_ref()).err(),
        Some(Error::AuthenticationFailed)
    );
    let missing_id = KeyId::new("unrelated-key")?;
    let missing_keyring = Keyring::new(
        missing_id.clone(),
        [(missing_id, SecretKey::from_bytes([9_u8; 32]))],
    )?;
    assert_eq!(
        missing_keyring.open(&context, borrowed.as_ref()).err(),
        Some(Error::KeyUnavailable)
    );
    Ok(())
}
