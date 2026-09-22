use std::sync::Arc;

use super::*;

fn decode_hex<const N: usize>(value: &str) -> [u8; N] {
    assert_eq!(value.len(), N * 2);
    let mut output = [0_u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).expect("valid hex");
    }
    output
}

#[test]
fn imported_key_matches_known_hmac_sha256_answer() {
    // The classic HMAC-SHA256 "key"/quick-brown-fox vector. HMAC pads keys
    // shorter than its 64-byte block with zeroes, so the required 32-byte
    // final key below has the same effective key block as the published key.
    let mut bytes = [0_u8; MAC_BYTES];
    bytes[..3].copy_from_slice(b"key");
    let key = MacKey::from_bytes(bytes);
    let expected =
        decode_hex::<MAC_BYTES>("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8");

    assert_eq!(
        key.sign(b"The quick brown fox jumps over the lazy dog"),
        expected
    );
    key.verify(b"The quick brown fox jumps over the lazy dog", &expected)
        .expect("published tag must verify");
}

#[test]
fn explicit_hkdf_matches_rfc_5869_case_one_prefix() {
    let input_key_material = [0x0b_u8; 22];
    let salt = decode_hex::<13>("000102030405060708090a0b0c");
    let info = decode_hex::<10>("f0f1f2f3f4f5f6f7f8f9");
    let expected =
        decode_hex::<MAC_BYTES>("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf");

    let key = MacKey::derive_hkdf(&input_key_material, Some(&salt), &info);

    // The RFC vector publishes 42 output bytes. MacKey deliberately owns the
    // first 32 bytes because HMAC-SHA256 keys in this crate are fixed-size.
    assert_eq!(key.bytes.as_ref().as_ref(), &expected);
}

#[test]
fn segments_are_exact_concatenation_and_verification_rejects_changes() {
    let key = MacKey::from_bytes([7_u8; MAC_BYTES]);
    let tag = key.sign_segments([b"alpha".as_slice(), b"beta".as_slice()]);

    assert_eq!(tag, key.sign(b"alphabeta"));
    key.verify_segments([b"alpha".as_slice(), b"beta".as_slice()], &tag)
        .expect("exact segments must verify");
    assert_eq!(
        key.verify_segments([b"alpha".as_slice(), b"betA".as_slice()], &tag),
        Err(Error::AuthenticationFailed)
    );
    assert_eq!(
        key.verify(b"alphabeta", &tag[..MAC_BYTES - 1]),
        Err(Error::AuthenticationFailed)
    );
}

#[test]
fn import_rejects_wrong_lengths_and_clone_shares_redacted_secret() {
    assert!(matches!(
        MacKey::try_from_slice(&[1_u8; MAC_BYTES - 1]),
        Err(Error::InvalidMacKey)
    ));
    assert!(matches!(
        MacKey::try_from_slice(&[1_u8; MAC_BYTES + 1]),
        Err(Error::InvalidMacKey)
    ));

    let key = MacKey::from_bytes([0xa5_u8; MAC_BYTES]);
    let clone = key.clone();
    assert!(Arc::ptr_eq(&key.bytes, &clone.bytes));
    assert_eq!(format!("{key:?}"), "MacKey(<redacted>)");
    assert!(!format!("{key:?}").contains("165"));
}
