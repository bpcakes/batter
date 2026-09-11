#[path = "../../../test-support/temp_dir.rs"]
mod temp_dir;
use batter::settings::{
    RedactedError, SecretString, SettingsSource, bounded_u64, milliseconds, read_file, read_literal,
};
use std::{error::Error, ffi::OsString, io::Cursor, os::unix::ffi::OsStringExt, time::Duration};

fn source(pairs: &[(&str, &str)]) -> SettingsSource {
    SettingsSource::from_pairs(pairs.iter().map(|(k, v)| ((*k).into(), (*v).into()))).unwrap()
}

#[test]
fn precedence_parses_only_winners_but_validates_each_source() {
    let mut merged = source(&[("LIMIT", "1")]);
    merged.overlay(read_literal(Cursor::new("LIMIT=2"), 64).unwrap());
    merged.overlay(source(&[("LIMIT", "invalid-shadowed")]));
    merged.overlay(source(&[("LIMIT", "4")]));
    assert_eq!(
        bounded_u64(merged.required("LIMIT").unwrap(), "LIMIT", 1, 10).unwrap(),
        4
    );
    merged.overlay(source(&[("LIMIT", "invalid-winning")]));
    assert!(bounded_u64(merged.required("LIMIT").unwrap(), "LIMIT", 1, 10).is_err());
    let environment = source(&[("UNRELATED", "ignored"), ("LIMIT", "3")]);
    assert_eq!(
        environment
            .select(&["LIMIT"], &["APP_"], true)
            .unwrap()
            .required("LIMIT")
            .unwrap(),
        "3"
    );
    assert!(environment.select(&["LIMIT"], &["APP_"], false).is_err());
    assert!(
        source(&[("APP_UNKNOWN", "secret")])
            .select(&["LIMIT"], &["APP_"], true)
            .is_err()
    );
    assert!(
        SettingsSource::from_pairs([("LIMIT".into(), "1".into()), ("LIMIT".into(), "2".into())])
            .is_err()
    );
}

#[test]
fn missing_empty_and_non_unicode_are_distinct_and_retained() {
    let raw = OsString::from_vec(b"secret-marker\xff".to_vec());
    let input =
        SettingsSource::from_pairs([("EMPTY".into(), "".into()), ("RAW".into(), raw.clone())])
            .unwrap();
    assert_eq!(input.text("MISSING").unwrap(), None);
    assert_eq!(
        input.required("MISSING").unwrap_err().reason(),
        "missing value"
    );
    assert_eq!(input.text("EMPTY").unwrap(), Some(""));
    let error = input.text("RAW").unwrap_err();
    assert_eq!(error.reason(), "non-Unicode value");
    assert_eq!(
        error.source().unwrap().downcast_ref::<std::env::VarError>(),
        Some(&std::env::VarError::NotUnicode(raw))
    );
    assert!(!format!("{error:#?}").contains("secret-marker"));
}

#[test]
fn literal_dialect_is_bounded_and_does_not_expand() {
    let text =
        " # comment\r\nA = 'space $HOME \\ literal'\r\nB=\"literal # text\"\nC=\nD=$HOME\\x\n";
    let parsed = read_literal(Cursor::new(text), text.len()).unwrap();
    assert_eq!(parsed.required("A").unwrap(), "space $HOME \\ literal");
    assert_eq!(parsed.required("B").unwrap(), "literal # text");
    assert_eq!(parsed.required("C").unwrap(), "");
    assert_eq!(parsed.required("D").unwrap(), "$HOME\\x");
    assert!(read_literal(Cursor::new(text), text.len() - 1).is_err());
    assert!(read_literal(Cursor::new(""), usize::MAX).is_err());
    for text in [
        "export A=x",
        "A=x #comment",
        "A=a b",
        "1A=x",
        "A='open",
        "A=\"multi\nline\"",
        "A=x\nA=y",
        "A=x\rY",
        "A=\0",
        "A=x'quoted'",
    ] {
        let error = read_literal(Cursor::new(text), 1024).unwrap_err();
        assert_eq!(error.field(), "file");
        assert!(error.line().is_some());
    }
    let invalid = read_literal(Cursor::new(b"secret-marker\xff"), 128).unwrap_err();
    assert!(invalid.source().unwrap().is::<std::string::FromUtf8Error>());
    assert!(!format!("{invalid:#?}").contains("secret-marker"));
}

#[test]
fn exact_path_does_not_search_parent_and_io_causes_survive() {
    let directory = temp_dir::TempDir::new().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("child")).unwrap();
    std::fs::write(root.join(".env"), "LIMIT=7").unwrap();
    let missing = read_file(root.join("child/.env"), 64).unwrap_err();
    assert_eq!(
        missing
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(read_file(root, 64).is_err());
    assert_eq!(
        read_file(root.join(".env"), 64)
            .unwrap()
            .required("LIMIT")
            .unwrap(),
        "7"
    );
    directory.close().unwrap();
}

#[test]
fn fixture_directory_rejects_existing_paths_and_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = temp_dir::TempDir::new().unwrap();
    assert_eq!(
        std::fs::metadata(directory.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let existing = directory.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    let canary = existing.join("canary");
    std::fs::write(&canary, "preserved").unwrap();
    let link = directory.path().join("link");
    symlink(&existing, &link).unwrap();
    for path in [existing, link] {
        let error = temp_dir::TempDir::create(path)
            .err()
            .expect("existing path must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&canary).unwrap(), "preserved");
    }
    directory.close().unwrap();
}

#[test]
fn bounds_reject_signs_overflow_and_first_outside_values() {
    for value in ["", "-1", "+1", " 1", "0", "11", "18446744073709551616"] {
        assert!(bounded_u64(value, "LIMIT", 1, 10).is_err());
    }
    for value in ["1", "10"] {
        assert!(bounded_u64(value, "LIMIT", 1, 10).is_ok());
    }
    let overflow = bounded_u64("18446744073709551616", "LIMIT", 0, u64::MAX).unwrap_err();
    assert!(overflow.source().unwrap().is::<std::num::ParseIntError>());
    assert_eq!(
        milliseconds("1000", "TIME", Duration::from_secs(1)).unwrap(),
        Duration::from_secs(1)
    );
    assert!(milliseconds("1001", "TIME", Duration::from_secs(1)).is_err());
    assert!(milliseconds("0", "TIME", Duration::from_secs(1)).is_err());
    assert!(milliseconds("1", "TIME", Duration::MAX).is_err());
}

#[test]
fn automatic_formats_hide_secrets_and_keys_but_explicit_access_preserves_them() {
    let secret =
        SecretString::new("postgres://secret-marker:encoded%40secret@host/db?password=hidden");
    assert!(secret.expose_secret().contains("secret-marker"));
    let source = source(&[("secret-marker-key", secret.expose_secret())]);
    let cause = std::io::Error::other(secret.expose_secret());
    let error = RedactedError::new("configuration", "invalid", cause);
    for output in [
        format!("{secret}"),
        format!("{secret:#?}"),
        format!("{:?}", vec![secret.clone()]),
        format!("{source:#?}"),
        format!("{source}"),
        format!("{error:#?}"),
        format!("{error}"),
    ] {
        assert!(!output.contains("secret-marker"));
        assert!(!output.contains("encoded%40"));
        assert!(!output.is_empty());
    }
    assert!(error.cause().to_string().contains("secret-marker"));
    assert!(error.source().unwrap().is::<std::io::Error>());
}
