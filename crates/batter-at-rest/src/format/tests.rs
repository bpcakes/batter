use super::*;

fn descriptor() -> ContentDescriptor {
    ContentDescriptor::new([0x11; NONCE_BYTES])
}

fn wrapped_key() -> WrappedKey {
    WrappedKey::new(
        KeyId::new("primary-2026").unwrap(),
        [0x22; NONCE_BYTES],
        [0x33; WRAPPED_DATA_KEY_BYTES],
    )
}

fn envelope() -> Envelope {
    Envelope::from_parts(descriptor(), wrapped_key())
}

#[test]
fn descriptor_codec_is_exact_and_canonical() {
    let mut expected = [0_u8; DESCRIPTOR_BYTES];
    expected[0] = FORMAT_VERSION;
    expected[1] = NONCE_BYTES as u8;
    expected[2..].fill(0x11);

    assert_eq!(descriptor().encode(), expected);
    assert_eq!(ContentDescriptor::decode(&expected).unwrap(), descriptor());

    let mut unknown = expected;
    unknown[0] = FORMAT_VERSION + 1;
    assert_eq!(
        ContentDescriptor::decode(&unknown).unwrap_err(),
        Error::UnsupportedVersion
    );
    let mut bad_nonce_length = expected;
    bad_nonce_length[1] -= 1;
    assert_eq!(
        ContentDescriptor::decode(&bad_nonce_length).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        ContentDescriptor::decode(&expected[..expected.len() - 1]).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        ContentDescriptor::decode(&[expected.as_slice(), &[0]].concat()).unwrap_err(),
        Error::TrailingData
    );
}

#[test]
fn wrapped_key_codec_is_exact_and_canonical() {
    let encoded = wrapped_key().encode();
    assert_eq!(encoded[0], 12);
    assert_eq!(&encoded[1..13], b"primary-2026");
    assert_eq!(encoded[13], NONCE_BYTES as u8);
    assert_eq!(&encoded[14..26], &[0x22; NONCE_BYTES]);
    assert_eq!(
        &encoded[26..28],
        &(WRAPPED_DATA_KEY_BYTES as u16).to_be_bytes()
    );
    assert_eq!(&encoded[28..], &[0x33; WRAPPED_DATA_KEY_BYTES]);
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &encoded).unwrap(),
        wrapped_key()
    );

    let mut bad_id = encoded.clone();
    bad_id[1] = b'/';
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &bad_id).unwrap_err(),
        Error::InvalidKeyId
    );
    let mut empty_id = encoded.clone();
    empty_id[0] = 0;
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &empty_id).unwrap_err(),
        Error::InvalidKeyId
    );
    let mut oversized_id = vec![(MAX_KEY_ID_BYTES + 1) as u8];
    oversized_id.extend(std::iter::repeat_n(b'k', MAX_KEY_ID_BYTES + 1));
    oversized_id.push(NONCE_BYTES as u8);
    oversized_id.extend_from_slice(&[0x22; NONCE_BYTES]);
    oversized_id.extend_from_slice(&(WRAPPED_DATA_KEY_BYTES as u16).to_be_bytes());
    oversized_id.extend_from_slice(&[0x33; WRAPPED_DATA_KEY_BYTES]);
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &oversized_id).unwrap_err(),
        Error::InvalidKeyId
    );
    let mut bad_nonce_length = encoded.clone();
    bad_nonce_length[13] = 11;
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &bad_nonce_length).unwrap_err(),
        Error::MalformedEncoding
    );
    let mut bad_wrapped_length = encoded.clone();
    bad_wrapped_length[27] = 47;
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &bad_wrapped_length).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &encoded[..encoded.len() - 1]).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        WrappedKey::decode_for(&descriptor(), &[encoded.as_slice(), &[0]].concat()).unwrap_err(),
        Error::TrailingData
    );
}

#[test]
fn envelope_codec_round_trips_nested_parts_and_rejects_bad_boundaries() {
    let envelope = envelope();
    let encoded = envelope.encode();
    assert_eq!(&encoded[..4], ENVELOPE_MAGIC);
    assert_eq!(&encoded[4..6], &(DESCRIPTOR_BYTES as u16).to_be_bytes());
    assert_eq!(Envelope::decode(&encoded).unwrap(), envelope);

    let mut bad_magic = encoded.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        Envelope::decode(&bad_magic).unwrap_err(),
        Error::MalformedEncoding
    );

    let mut bad_descriptor_length = encoded.clone();
    bad_descriptor_length[5] = (DESCRIPTOR_BYTES - 1) as u8;
    assert_eq!(
        Envelope::decode(&bad_descriptor_length).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        Envelope::decode(&encoded[..encoded.len() - 1]).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        Envelope::decode(&[encoded.as_slice(), &[0]].concat()).unwrap_err(),
        Error::TrailingData
    );
    assert_eq!(
        Envelope::decode(&vec![0_u8; MAX_ENVELOPE_BYTES + 1]).unwrap_err(),
        Error::MalformedEncoding
    );
}

#[test]
fn maximum_valid_key_id_produces_a_decodable_bounded_envelope() {
    assert_eq!(MAX_KEY_ID_BYTES, 128);
    assert_eq!(MAX_ENVELOPE_BYTES, 256);
    assert_eq!(crate::MAX_PLAINTEXT_BYTES, 64 * 1024 * 1024);
    assert_eq!(MAX_BODY_BYTES, 64 * 1024 * 1024 + TAG_BYTES);

    let wrapper = WrappedKey::new(
        KeyId::new(&"k".repeat(MAX_KEY_ID_BYTES)).unwrap(),
        [0x22; NONCE_BYTES],
        [0x33; WRAPPED_DATA_KEY_BYTES],
    );
    let envelope = Envelope::from_parts(descriptor(), wrapper);
    let encoded = envelope.encode();

    assert_eq!(encoded.len(), MAX_CANONICAL_ENVELOPE_BYTES);
    assert!(encoded.len() <= MAX_ENVELOPE_BYTES);
    assert_eq!(Envelope::decode(&encoded).unwrap(), envelope);
}

#[test]
fn composite_codec_round_trips_and_borrowed_parts_do_not_copy() {
    let envelope = envelope();
    let ciphertext = vec![0x44; TAG_BYTES + 3];
    let sealed = SealedPayload::from_parts(envelope, ciphertext).unwrap();
    let encoded = sealed.encode();
    let decoded = SealedPayload::decode(&encoded).unwrap();

    assert_eq!(&encoded[..4], SEALED_MAGIC);
    assert_eq!(decoded, sealed);
    assert_eq!(decoded.as_ref().envelope(), decoded.envelope());
    assert!(std::ptr::eq(
        decoded.as_ref().ciphertext().as_ptr(),
        decoded.ciphertext().as_ptr()
    ));

    let mut bad_body_length = encoded.clone();
    let envelope_length = usize::from(u16::from_be_bytes([encoded[4], encoded[5]]));
    let body_length_offset = 6 + envelope_length;
    bad_body_length[body_length_offset + 3] += 1;
    assert_eq!(
        SealedPayload::decode(&bad_body_length).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        SealedPayload::decode(&[encoded.as_slice(), &[0]].concat()).unwrap_err(),
        Error::TrailingData
    );

    let mut oversized_body = Vec::new();
    oversized_body.extend_from_slice(SEALED_MAGIC);
    push_u16_length(&mut oversized_body, &sealed.envelope().encode());
    oversized_body.extend_from_slice(&u32::try_from(MAX_BODY_BYTES + 1).unwrap().to_be_bytes());
    assert_eq!(
        SealedPayload::decode(&oversized_body).unwrap_err(),
        Error::MalformedEncoding
    );
}

#[test]
fn bodies_are_bounded_and_debug_output_is_redacted() {
    assert_eq!(
        SealedPayload::from_parts(envelope(), vec![0_u8; TAG_BYTES - 1]).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        SealedPayloadRef::new(&envelope(), &[0_u8; TAG_BYTES - 1]).unwrap_err(),
        Error::MalformedEncoding
    );

    let oversized_envelope = envelope();
    let oversized = vec![0_u8; MAX_BODY_BYTES + 1];
    assert_eq!(
        SealedPayloadRef::new(&oversized_envelope, &oversized).unwrap_err(),
        Error::MalformedEncoding
    );
    assert_eq!(
        SealedPayload::from_parts(oversized_envelope, oversized).unwrap_err(),
        Error::MalformedEncoding
    );

    let sealed = SealedPayload::from_parts(envelope(), vec![0x7a; TAG_BYTES]).unwrap();
    let rendered = format!("{sealed:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("122"));
}

#[test]
fn composite_decoder_rejects_bad_magic_and_every_truncation() {
    let sealed = SealedPayload::from_parts(envelope(), vec![0x44; TAG_BYTES + 3]).unwrap();
    let encoded = sealed.encode();

    let mut bad_magic = encoded.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        SealedPayload::decode(&bad_magic).unwrap_err(),
        Error::MalformedEncoding
    );
    for length in 0..encoded.len() {
        assert!(SealedPayload::decode(&encoded[..length]).is_err());
    }
}
