//! Encoding 1 of a row's state, against the shared vectors in
//! `support/state_encoding_1.json`.
//!
//! The writer and the persisted-file entries of the later packages reuse that
//! file, so it describes what a writer emits and what a reader observes, never
//! this crate's internals.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![cfg(feature = "serde")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::collections::HashSet;

use batter_flags_core::{
    Freshness, Layer, SwitchState, Toggle,
    low_level::{CandidateRow, HoldCause},
};
use fixtures::Checkout;
use serde_json::Value;
use world::{base, snapshot, version, world};

const FIXTURE: &str = include_str!("support/state_encoding_1.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn text<'a>(entry: &'a Value, field: &str) -> &'a str {
    entry[field].as_str().unwrap()
}

#[test]
fn the_fixture_is_the_encoding_this_build_reads() {
    let document = fixture();
    assert_eq!(document["encoding"], 1);
    let (writer, reader) = (
        document["writer"].as_array().unwrap(),
        document["reader"].as_array().unwrap(),
    );
    // A dropped vector is a change to the contract, so the counts are pinned.
    assert_eq!((writer.len(), reader.len()), (5, 63));
    let names: HashSet<_> = reader
        .iter()
        .map(|entry| (text(entry, "shape"), text(entry, "name")))
        .collect();
    assert_eq!(names.len(), reader.len(), "reader vector names are unique");
    let causes: HashSet<_> = [HoldCause::UnknownRowEncoding, HoldCause::UndeclaredVariant]
        .into_iter()
        .map(HoldCause::as_str)
        .collect();
    for entry in reader {
        if let Some(cause) = entry["reads"]["held"].as_str() {
            assert!(causes.contains(cause), "{cause}");
        }
    }
}

#[test]
fn a_writer_emits_exactly_the_canonical_text_and_a_reader_reads_it_back() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    for entry in fixture()["writer"].as_array().unwrap() {
        let expected = text(entry, "text");
        let row = match (
            text(entry, "shape"),
            entry["variant"].as_str(),
            entry["state"].as_str(),
        ) {
            ("flag", Some("on"), _) => CandidateRow::flag(&h.fast_path, Toggle::On, version(2)),
            ("flag", Some("off"), _) => CandidateRow::flag(&h.fast_path, Toggle::Off, version(2)),
            ("flag", Some("express"), _) => {
                CandidateRow::flag(&h.checkout_style, Checkout::Express, version(2))
            }
            ("switch", _, Some("engaged")) => {
                CandidateRow::switch(&h.checkout_halt, SwitchState::Engaged, version(2))
            }
            ("switch", _, Some("released")) => {
                CandidateRow::switch(&h.checkout_halt, SwitchState::Released, version(2))
            }
            other => panic!("a writer vector this test does not know: {other:?}"),
        };
        assert_eq!(row.state_text(), expected);
        let published = snapshot(&base, 3, [row]);
        assert_eq!(published.held().count(), 0, "{expected}");
    }
}

#[test]
fn a_reader_publishes_or_holds_every_text_as_the_vectors_say() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let mut counts = (0, 0);
    for entry in fixture()["reader"].as_array().unwrap() {
        let (name, shape, state) = (
            text(entry, "name"),
            text(entry, "shape"),
            text(entry, "text"),
        );
        let row = match shape {
            "flag" => CandidateRow::new("fast_path", "release", "toggle", version(2), state),
            "switch" => CandidateRow::new("checkout_halt", "switch", "switch", version(2), state),
            other => panic!("{other}"),
        };
        let snapshot = snapshot(&base, 3, [row]);
        let view = base
            .control_plane_view(&snapshot, Freshness::Fresh)
            .unwrap();
        let reads = &entry["reads"];
        let context = format!("{shape}: {name}");
        let held: Vec<_> = snapshot
            .held()
            .map(|(key, cause)| (key.as_str().to_owned(), cause.as_str()))
            .collect();
        if let Some(cause) = reads["held"].as_str() {
            let key = if shape == "flag" {
                "fast_path"
            } else {
                "checkout_halt"
            };
            assert_eq!(held, [(key.to_owned(), cause)], "{context}");
            counts.1 += 1;
        } else {
            assert_eq!(held, [], "{context}");
            if let Some(variant) = reads["variant"].as_str() {
                let decision = view.evaluate(&h.fast_path);
                assert_eq!(decision.variant(), variant, "{context}");
                assert_eq!(decision.layer(), &Layer::Entry, "{context}");
            } else {
                let decision = view.switch(&h.checkout_halt);
                assert_eq!(
                    decision.variant(),
                    reads["state"].as_str().unwrap(),
                    "{context}"
                );
                assert_eq!(decision.layer(), &Layer::Entry, "{context}");
            }
            counts.0 += 1;
        }
    }
    assert_eq!(counts, (10, 53), "published and held vectors");
}
