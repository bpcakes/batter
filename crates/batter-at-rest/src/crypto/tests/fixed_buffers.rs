use super::*;

const WRAPPING_KEY: [u8; SECRET_KEY_BYTES] = [0x11; SECRET_KEY_BYTES];
const WRAPPING_NONCE: [u8; NONCE_BYTES] = [0x22; NONCE_BYTES];
const AAD: &[u8] = b"ATRA\x02\x01wrapper aad under test";
const DATA_KEY: [u8; SECRET_KEY_BYTES] = [0x33; SECRET_KEY_BYTES];

#[test]
fn fixed_data_key_wrapping_produces_the_variable_size_primitive_bytes() {
    let wrapped = encrypt_data_key(&WRAPPING_KEY, &WRAPPING_NONCE, AAD, &DATA_KEY).unwrap();
    assert_eq!(
        wrapped.as_slice(),
        encrypt_bytes(&WRAPPING_KEY, &WRAPPING_NONCE, AAD, &DATA_KEY).unwrap()
    );
    assert_ne!(&wrapped[..SECRET_KEY_BYTES], DATA_KEY);
    assert_eq!(
        *decrypt_data_key(&WRAPPING_KEY, &WRAPPING_NONCE, AAD, &wrapped).unwrap(),
        DATA_KEY
    );
    assert_eq!(
        decrypt_bytes(&WRAPPING_KEY, &WRAPPING_NONCE, AAD, &wrapped)
            .unwrap()
            .as_slice(),
        DATA_KEY
    );
}

#[test]
fn fixed_data_key_unwrapping_rejects_every_byte_change_and_mismatched_inputs() {
    let wrapped = encrypt_data_key(&WRAPPING_KEY, &WRAPPING_NONCE, AAD, &DATA_KEY).unwrap();
    for index in 0..WRAPPED_DATA_KEY_BYTES {
        let mut tampered = wrapped;
        tampered[index] ^= 1;
        assert_eq!(
            decrypt_data_key(&WRAPPING_KEY, &WRAPPING_NONCE, AAD, &tampered).unwrap_err(),
            Error::AuthenticationFailed
        );
    }
    assert_eq!(
        decrypt_data_key(&[0x12; SECRET_KEY_BYTES], &WRAPPING_NONCE, AAD, &wrapped).unwrap_err(),
        Error::AuthenticationFailed
    );
    assert_eq!(
        decrypt_data_key(&WRAPPING_KEY, &[0x23; NONCE_BYTES], AAD, &wrapped).unwrap_err(),
        Error::AuthenticationFailed
    );
    assert_eq!(
        decrypt_data_key(&WRAPPING_KEY, &WRAPPING_NONCE, b"other aad", &wrapped).unwrap_err(),
        Error::AuthenticationFailed
    );
}
