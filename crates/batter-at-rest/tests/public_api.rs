use batter_at_rest::{
    Context, Envelope, Error, KeyId, Keyring, MacKey, SealedPayload, SealedPayloadRef, SecretKey,
};

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
