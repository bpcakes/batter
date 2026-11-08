//! The `BFSN` v1 format and how a file is judged against a base.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_flags_core::{
    CatalogBuilder, SafeState, SettingsPrefix, StaleRule,
    low_level::{Base, BaseBuilder, HoldCause, Provenance, SnapshotRejection},
};
use sha2::{Digest, Sha256};

use super::{
    FileProblem,
    format::{EntryValue, FileEntry, FileSnapshot, LIMIT, decode, encode},
    load, scope_label,
};

fn base() -> Base {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    catalog.environment_class("hosted").unwrap();
    catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    BaseBuilder::new(catalog.build().unwrap(), serving)
        .unwrap()
        .finish()
}

fn halt(base: &Base, state: &str) -> FileEntry {
    FileEntry {
        key: "halt".into(),
        kind: "switch".into(),
        value_type: "switch".into(),
        fingerprint: *base
            .catalog()
            .declaration("halt")
            .unwrap()
            .fingerprint()
            .as_bytes(),
        updated_version: 7,
        value: crate::last_good::format::EntryValue::State(format!(
            "{{\"encoding\":1,\"switch\":\"{state}\"}}"
        )),
    }
}

fn file(base: &Base, entries: Vec<FileEntry>) -> FileSnapshot {
    FileSnapshot {
        scope: scope_label(base),
        catalog_digest: *base.catalog().digest().as_bytes(),
        manifest_digest: *base.catalog().manifest_digest().as_bytes(),
        version: 9,
        entries,
    }
}

/// The bytes the format document defines, assembled field by field without
/// the encoder.
fn by_hand(file: &FileSnapshot) -> Vec<u8> {
    let mut out = b"BFSN\x01".to_vec();
    out.extend_from_slice(&(file.scope.len() as u16).to_be_bytes());
    out.extend_from_slice(file.scope.as_bytes());
    out.extend_from_slice(&file.catalog_digest);
    out.extend_from_slice(&file.manifest_digest);
    out.extend_from_slice(&file.version.to_be_bytes());
    out.extend_from_slice(&(file.entries.len() as u32).to_be_bytes());
    for entry in &file.entries {
        for text in [&entry.key, &entry.kind, &entry.value_type] {
            out.push(text.len() as u8);
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(&entry.fingerprint);
        out.extend_from_slice(&entry.updated_version.to_be_bytes());
        let state = match &entry.value {
            EntryValue::State(state) => state.as_str(),
            EntryValue::Absent => "",
        };
        out.extend_from_slice(&(state.len() as u16).to_be_bytes());
        out.extend_from_slice(state.as_bytes());
    }
    let trailer = Sha256::digest(&out);
    out.extend_from_slice(&trailer);
    out
}

#[test]
fn the_encoder_and_parser_match_the_documented_layout() {
    let base = base();
    let mut without = halt(&base, "engaged");
    without.key = "other".into();
    without.value = EntryValue::Absent;
    let file = file(&base, vec![halt(&base, "engaged"), without]);
    let bytes = by_hand(&file);
    assert_eq!(encode(&file).unwrap(), bytes);
    assert_eq!(decode(&bytes).unwrap(), file);
    // The fixed header: magic, format version, then the scope label's length.
    assert_eq!(&bytes[..7], b"BFSN\x01\x00\x0d");
    assert_eq!(&bytes[7..20], b"FLAG_:serving");
}

#[test]
fn damage_of_every_kind_is_refused_by_name() {
    let base = base();
    let bytes = encode(&file(&base, vec![halt(&base, "engaged")])).unwrap();
    let mut tampered = bytes.clone();
    tampered[30] ^= 1;
    assert_eq!(decode(&tampered), Err(FileProblem::Tampered));
    let mut trailer = bytes.clone();
    *trailer.last_mut().unwrap() ^= 1;
    assert_eq!(decode(&trailer), Err(FileProblem::Tampered));
    assert_eq!(
        decode(&bytes[..bytes.len() - 1]),
        Err(FileProblem::Truncated)
    );
    assert_eq!(decode(&bytes[..40]), Err(FileProblem::Truncated));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing), Err(FileProblem::TrailingBytes));
    let mut magic = bytes.clone();
    magic[0] = b'X';
    assert_eq!(decode(&magic), Err(FileProblem::Malformed));
    let mut version = bytes.clone();
    version[4] = 2;
    assert_eq!(decode(&version), Err(FileProblem::UnsupportedVersion));
    assert_eq!(decode(&vec![0; LIMIT + 1]), Err(FileProblem::TooLarge));
    assert_eq!(decode(&[]), Err(FileProblem::Truncated));
}

#[test]
fn a_usable_file_loads_as_persisted_and_is_matched_entry_by_entry() {
    let base = base();
    let mut other_catalog = file(&base, vec![halt(&base, "engaged")]);
    // Another catalog digest is not foreign: entries are matched one by one.
    other_catalog.catalog_digest = [0; 32];
    other_catalog.manifest_digest = [1; 32];
    let snapshot = load(&base, &encode(&other_catalog).unwrap()).unwrap();
    assert_eq!(snapshot.provenance(), Provenance::LastGoodFile);
    assert_eq!(snapshot.version().get(), 9);
    assert_eq!(
        snapshot.held(batter_flags_core::Freshness::Fresh).count(),
        0
    );
    assert_eq!(snapshot.unknown() + snapshot.not_owned(), 0);
}

#[test]
fn a_foreign_scope_a_changed_declaration_and_a_corrupt_entry_list_publish_nothing_of_theirs() {
    let base = base();
    let mut foreign = file(&base, vec![halt(&base, "engaged")]);
    foreign.scope = "FLAG_:worker".into();
    assert_eq!(
        load(&base, &encode(&foreign).unwrap()).unwrap_err(),
        FileProblem::Foreign
    );

    let mut changed = halt(&base, "engaged");
    changed.fingerprint = [7; 32];
    let snapshot = load(&base, &encode(&file(&base, vec![changed])).unwrap()).unwrap();
    let held: Vec<_> = snapshot
        .held(batter_flags_core::Freshness::Fresh)
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect();
    assert_eq!(held, [("halt".to_owned(), HoldCause::EntryTypeMismatch)]);

    let mut unknown = halt(&base, "engaged");
    unknown.key = "retired_switch".into();
    let snapshot = load(&base, &encode(&file(&base, vec![unknown])).unwrap()).unwrap();
    assert_eq!(snapshot.unknown(), 1);

    let twice = file(&base, vec![halt(&base, "engaged"), halt(&base, "released")]);
    assert_eq!(
        load(&base, &encode(&twice).unwrap()).unwrap_err(),
        FileProblem::Rejected(SnapshotRejection::DuplicateKey { row: 1 })
    );
}

#[test]
fn an_entry_without_a_value_loads_as_absent() {
    let base = base();
    let mut absent = halt(&base, "engaged");
    absent.value = EntryValue::Absent;
    let snapshot = load(&base, &encode(&file(&base, vec![absent])).unwrap()).unwrap();
    assert_eq!(
        snapshot.held(batter_flags_core::Freshness::Fresh).count(),
        0
    );
    assert_eq!(snapshot.carried().count(), 0);
}

#[test]
fn file_formatting_never_shows_a_state() {
    let base = base();
    let mut entry = halt(&base, "engaged");
    entry.value = EntryValue::State("{\"encoding\":1,\"switch\":\"SECRET-MARKER\"}".into());
    let rendered = format!("{:?} {:#?}", file(&base, vec![entry.clone()]), entry);
    assert!(!rendered.contains("SECRET-MARKER"), "{rendered}");
    assert!(rendered.contains("halt"));
}

#[test]
fn only_owned_declared_rows_are_written_and_a_held_one_without_its_value() {
    use batter_flags_core::{
        CivilDate, Expiry, Toggle, Version,
        low_level::{Candidate, CandidateRow, Snapshot},
    };
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let worker = catalog.process_kind("worker").unwrap();
    catalog.environment_class("hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let mine = catalog
        .release_flag("mine", Toggle::Off, expiry)
        .declare()
        .unwrap();
    let theirs = catalog
        .release_flag("theirs", Toggle::Off, expiry)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    let broken = catalog
        .release_flag("broken", Toggle::Off, expiry)
        .declare()
        .unwrap();
    let base = BaseBuilder::new(catalog.build().unwrap(), serving)
        .unwrap()
        .finish();
    let scope = base.validation_scope();
    let at = Version::new(5);
    let rows = vec![
        CandidateRow::flag(&mine, Toggle::On, at),
        CandidateRow::flag(&theirs, Toggle::On, at),
        CandidateRow::new("broken", "release", "toggle", at, "{\"encoding\":9}"),
        CandidateRow::new(
            "unknown",
            "release",
            "toggle",
            at,
            "{\"encoding\":1,\"variant\":\"on\"}",
        ),
    ];
    let _ = broken;
    let snapshot =
        Snapshot::validate(&scope, Candidate::new(&scope).at(at).rows(rows.clone())).unwrap();
    let file = super::file_of(&base, &snapshot, &rows).unwrap();
    let written: Vec<_> = file
        .entries
        .iter()
        .map(|entry| {
            (
                entry.key.as_str(),
                matches!(entry.value, EntryValue::State(_)),
            )
        })
        .collect();
    assert_eq!(written, [("mine", true), ("broken", false)]);
}
