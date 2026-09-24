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
    let mut oversized = encoded.clone();
    assert!(oversized.len() <= MAX_ENVELOPE_BYTES);
    oversized.resize(MAX_ENVELOPE_BYTES + 1, 0);
    assert_eq!(
        Envelope::decode(&oversized).unwrap_err(),
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
    let composite = SealedPayload::from_parts(envelope, vec![0_u8; TAG_BYTES])
        .unwrap()
        .encode();
    assert_eq!(
        BorrowedSealedPayload::decode(&composite)
            .unwrap()
            .envelope()
            .wrapped_key()
            .key_id()
            .as_str()
            .len(),
        MAX_KEY_ID_BYTES
    );
}

#[test]
fn composite_codec_round_trips_and_borrowed_parts_do_not_copy() {
    let envelope = envelope();
    let ciphertext = vec![0x44; TAG_BYTES + 3];
    let sealed = SealedPayload::from_parts(envelope, ciphertext).unwrap();
    let encoded = sealed.encode();
    let decoded = SealedPayload::decode(&encoded).unwrap();
    let borrowed = BorrowedSealedPayload::decode(&encoded).unwrap();
    let body_offset = encoded.len() - sealed.ciphertext().len();

    assert_eq!(&encoded[..4], SEALED_MAGIC);
    assert_eq!(decoded, sealed);
    assert_eq!(borrowed.envelope(), decoded.envelope());
    assert_eq!(borrowed.ciphertext(), &encoded[body_offset..]);
    assert_eq!(borrowed.ciphertext().len(), encoded.len() - body_offset);
    assert!(std::ptr::eq(
        borrowed.ciphertext().as_ptr(),
        encoded[body_offset..].as_ptr()
    ));
    assert!(std::ptr::eq(
        borrowed.as_ref().ciphertext().as_ptr(),
        borrowed.ciphertext().as_ptr()
    ));
    assert_eq!(borrowed.as_ref().envelope(), borrowed.envelope());
    assert_eq!(decoded.as_ref().envelope(), decoded.envelope());
    assert!(std::ptr::eq(
        decoded.as_ref().ciphertext().as_ptr(),
        decoded.ciphertext().as_ptr()
    ));

    let mut bad_body_length = encoded.clone();
    let envelope_length = usize::from(u16::from_be_bytes([encoded[4], encoded[5]]));
    let body_length_offset = 6 + envelope_length;
    bad_body_length[body_length_offset + 3] += 1;
    assert_composite_error(&bad_body_length, Error::MalformedEncoding);
    assert_composite_error(&[encoded.as_slice(), &[0]].concat(), Error::TrailingData);

    let mut oversized_body = Vec::new();
    oversized_body.extend_from_slice(SEALED_MAGIC);
    push_u16_length(&mut oversized_body, &sealed.envelope().encode());
    oversized_body.extend_from_slice(&u32::try_from(MAX_BODY_BYTES + 1).unwrap().to_be_bytes());
    assert_composite_error(&oversized_body, Error::MalformedEncoding);
}

fn assert_composite_error(bytes: &[u8], expected: Error) {
    assert_eq!(BorrowedSealedPayload::decode(bytes).unwrap_err(), expected);
    assert_eq!(SealedPayload::decode(bytes).unwrap_err(), expected);
}

#[test]
fn composite_decoder_preserves_specific_nested_and_body_errors() {
    let encoded = SealedPayload::from_parts(envelope(), vec![0x44; TAG_BYTES + 3])
        .unwrap()
        .encode();
    let envelope_len = envelope().encode().len();
    let descriptor_offset = 6 + 4 + 2;
    let wrapper_offset = descriptor_offset + DESCRIPTOR_BYTES + 2;
    let body_len_offset = 6 + envelope_len;

    let mut unknown_version = encoded.clone();
    unknown_version[descriptor_offset] = FORMAT_VERSION + 1;
    assert_composite_error(&unknown_version, Error::UnsupportedVersion);

    let mut invalid_id = encoded.clone();
    invalid_id[wrapper_offset + 1] = b'/';
    assert_composite_error(&invalid_id, Error::InvalidKeyId);

    let mut descriptor_length = encoded.clone();
    descriptor_length[6 + 5] = (DESCRIPTOR_BYTES - 1) as u8;
    assert_composite_error(&descriptor_length, Error::MalformedEncoding);

    let mut nonce_length = encoded.clone();
    nonce_length[descriptor_offset + 1] = (NONCE_BYTES - 1) as u8;
    assert_composite_error(&nonce_length, Error::MalformedEncoding);

    let mut wrapped_key_length = encoded.clone();
    wrapped_key_length[wrapper_offset + 12 + 1 + NONCE_BYTES + 1] =
        (WRAPPED_DATA_KEY_BYTES - 1) as u8;
    assert_composite_error(&wrapped_key_length, Error::MalformedEncoding);

    let mut empty_body = encoded.clone();
    empty_body[body_len_offset..body_len_offset + 4].copy_from_slice(&0_u32.to_be_bytes());
    assert_composite_error(&empty_body, Error::MalformedEncoding);

    let mut short_body = encoded.clone();
    short_body[body_len_offset..body_len_offset + 4]
        .copy_from_slice(&u32::try_from(TAG_BYTES - 1).unwrap().to_be_bytes());
    assert_composite_error(&short_body, Error::MalformedEncoding);

    let mut oversized_envelope = envelope().encode();
    assert!(oversized_envelope.len() <= MAX_ENVELOPE_BYTES);
    oversized_envelope.resize(MAX_ENVELOPE_BYTES + 1, 0);
    let mut oversized_header = Vec::from(SEALED_MAGIC.as_slice());
    push_u16_length(&mut oversized_header, &oversized_envelope);
    oversized_header.extend_from_slice(&u32::try_from(TAG_BYTES).unwrap().to_be_bytes());
    oversized_header.extend_from_slice(&[0_u8; TAG_BYTES]);
    assert_composite_error(&oversized_header, Error::MalformedEncoding);

    let mut oversized_body = encoded.clone();
    oversized_body[body_len_offset..body_len_offset + 4]
        .copy_from_slice(&u32::try_from(MAX_BODY_BYTES + 1).unwrap().to_be_bytes());
    assert_composite_error(&oversized_body, Error::MalformedEncoding);
}

#[test]
fn composite_accepts_tag_only_and_maximum_body() {
    let tag_only = SealedPayload::from_parts(envelope(), vec![0_u8; TAG_BYTES])
        .unwrap()
        .encode();
    let decoded = BorrowedSealedPayload::decode(&tag_only).unwrap();
    assert_eq!(decoded.ciphertext().len(), TAG_BYTES);
    assert_eq!(
        SealedPayload::decode(&tag_only).unwrap().ciphertext().len(),
        TAG_BYTES
    );

    let mut largest = Vec::from(SEALED_MAGIC.as_slice());
    push_u16_length(&mut largest, &envelope().encode());
    let body_len_offset = largest.len();
    largest.extend_from_slice(&u32::try_from(MAX_BODY_BYTES).unwrap().to_be_bytes());
    let body_offset = largest.len();
    largest.resize(body_offset + MAX_BODY_BYTES, 0);
    let decoded = BorrowedSealedPayload::decode(&largest).unwrap();
    assert_eq!(decoded.ciphertext().len(), MAX_BODY_BYTES);
    assert!(std::ptr::eq(
        decoded.ciphertext().as_ptr(),
        largest[body_offset..].as_ptr()
    ));
    let owned = SealedPayload::decode(&largest).unwrap();
    assert_eq!(owned.ciphertext().len(), MAX_BODY_BYTES);
    assert_eq!(owned.encode(), largest);

    drop(owned);
    largest.push(0);
    largest[body_len_offset..body_offset]
        .copy_from_slice(&u32::try_from(MAX_BODY_BYTES + 1).unwrap().to_be_bytes());
    assert_eq!(largest.len() - body_offset, MAX_BODY_BYTES + 1);
    assert_composite_error(&largest, Error::MalformedEncoding);
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
    let encoded = sealed.encode();
    let borrowed = BorrowedSealedPayload::decode(&encoded).unwrap();
    let rendered = format!("{borrowed:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("122"));
}

#[test]
fn composite_decoder_rejects_bad_magic_and_every_truncation() {
    let sealed = SealedPayload::from_parts(envelope(), vec![0x44; TAG_BYTES + 3]).unwrap();
    let encoded = sealed.encode();

    let mut bad_magic = encoded.clone();
    bad_magic[0] ^= 1;
    assert_composite_error(&bad_magic, Error::MalformedEncoding);
    for length in 0..encoded.len() {
        assert_composite_error(&encoded[..length], Error::MalformedEncoding);
    }
}

#[test]
fn composite_byte_mutations_either_reject_or_round_trip_canonically() {
    let sealed = SealedPayload::from_parts(envelope(), vec![0x44; TAG_BYTES + 3]).unwrap();
    let encoded = sealed.encode();
    for offset in 0..encoded.len() {
        for replacement in 0..=u8::MAX {
            let mut candidate = encoded.clone();
            candidate[offset] = replacement;
            assert_eq!(
                BorrowedSealedPayload::decode(&candidate).err(),
                SealedPayload::decode(&candidate).err()
            );
            if let Ok(decoded) = SealedPayload::decode(&candidate) {
                assert_eq!(decoded.encode(), candidate);
            }
        }
    }
}
