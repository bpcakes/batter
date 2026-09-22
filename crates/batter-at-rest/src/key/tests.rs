use super::*;

fn key(byte: u8) -> SecretKey {
    SecretKey::from_bytes([byte; SECRET_KEY_BYTES])
}

#[test]
fn secret_keys_require_exact_length_and_have_redacted_debug() {
    assert_eq!(
        SecretKey::try_from_slice(&[7_u8; SECRET_KEY_BYTES - 1]).unwrap_err(),
        Error::InvalidSecretKey
    );
    assert_eq!(
        SecretKey::try_from_slice(&[7_u8; SECRET_KEY_BYTES + 1]).unwrap_err(),
        Error::InvalidSecretKey
    );
    assert_eq!(format!("{:?}", key(7)), "SecretKey(<redacted>)");
}

#[test]
fn key_ids_accept_only_bounded_safe_ascii() {
    for accepted in ["a", "PRIMARY-2026", "primary_2026.09:key-1"] {
        assert_eq!(KeyId::new(accepted).unwrap().as_str(), accepted);
    }

    for rejected in ["", "has space", "slash/key", "unicode-é", "line\nbreak"] {
        assert_eq!(KeyId::new(rejected).unwrap_err(), Error::InvalidKeyId);
    }
    assert_eq!(
        KeyId::new(&"x".repeat(MAX_KEY_ID_BYTES + 1)).unwrap_err(),
        Error::InvalidKeyId
    );
}

#[test]
fn keyring_rejects_empty_missing_current_and_duplicate_ids() {
    let current = KeyId::new("current").unwrap();
    assert_eq!(
        Keyring::new(current.clone(), []).unwrap_err(),
        Error::CurrentKeyUnavailable
    );
    assert_eq!(
        Keyring::new(current.clone(), [(KeyId::new("old").unwrap(), key(1))],).unwrap_err(),
        Error::CurrentKeyUnavailable
    );
    assert_eq!(
        Keyring::new(
            current.clone(),
            [(current.clone(), key(1)), (current, key(2))],
        )
        .unwrap_err(),
        Error::DuplicateKeyId
    );
}

#[test]
fn cloned_keyrings_share_long_lived_secret_ownership() {
    let current = KeyId::new("current").unwrap();
    let keyring = Keyring::new(current.clone(), [(current, key(9))]).unwrap();
    let cloned = keyring.clone();

    assert!(Arc::ptr_eq(&keyring.keys, &cloned.keys));
    assert_eq!(keyring.current_key_id().as_str(), "current");
    assert_eq!(
        format!("{keyring:?}"),
        "Keyring { current_id: \"<redacted>\", key_count: 1 }"
    );
}

#[test]
fn key_id_debug_does_not_echo_the_identifier() {
    let rendered = format!("{:?}", KeyId::new("sensitive-id").unwrap());
    assert_eq!(rendered, "KeyId(<redacted>)");
    assert!(!rendered.contains("sensitive-id"));
}
