//! What a resolution records for the control plane, which no public accessor
//! shows: the enabling variants whose required settings this process lacked.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle, Version,
    low_level::{Candidate, CandidateRow, HoldCause, Snapshot},
};

use super::resolve;

/// Resolves a catalog whose `new_checkout` needs a payment URL when it is on,
/// from `pairs`, and returns the keys a control-plane row selecting `on` would
/// be held for.
fn held_for_on(pairs: &[(&str, &str)]) -> Vec<(String, HoldCause)> {
    let (mut builder, serving, hosted) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let new_checkout = builder
        .release_flag("new_checkout", Toggle::Off, expiry)
        .requires_settings(Toggle::On, &["PAYMENT_API_URL"])
        .declare()
        .unwrap();
    let fast_path = builder
        .release_flag("fast_path", Toggle::Off, expiry)
        .declare()
        .unwrap();
    let source = SettingsSource::from_pairs(
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    )
    .unwrap();
    let catalog = builder.build();
    let resolved = resolve(
        catalog,
        &source,
        serving,
        hosted,
        CivilDate::new(2026, 11, 5).unwrap(),
    )
    .unwrap();
    let scope = resolved.base.validation_scope();
    let at = Version::new(5);
    let candidate = Candidate::new(&scope, at)
        .row(CandidateRow::flag(&new_checkout, Toggle::On, at))
        .row(CandidateRow::flag(&fast_path, Toggle::On, at));
    let snapshot = Snapshot::validate(candidate).unwrap();
    let held: Vec<_> = snapshot
        .held(batter_flags_core::Freshness::Fresh)
        .map(|(key, cause)| (key.to_string(), cause))
        .collect();
    held
}

#[test]
fn an_enabling_variant_whose_settings_were_absent_is_held_not_served() {
    assert_eq!(
        held_for_on(&[]),
        [("new_checkout".to_owned(), HoldCause::RequiresSettingsAbsent)]
    );
}

#[test]
fn an_empty_required_setting_counts_as_absent() {
    assert_eq!(
        held_for_on(&[("PAYMENT_API_URL", "")]),
        [("new_checkout".to_owned(), HoldCause::RequiresSettingsAbsent)]
    );
}

#[test]
fn present_settings_leave_the_variant_servable() {
    assert!(held_for_on(&[("PAYMENT_API_URL", "https://payments.example")]).is_empty());
}

/// A root that narrows its settings to its own schema first drops the flag
/// names; resolving from that source used to serve every flag's default
/// silently. It is refused, in any layer of an overlay.
#[test]
fn a_narrowed_source_is_refused_instead_of_read_as_defaults() {
    let capture = || {
        SettingsSource::from_pairs([
            ("FLAG_FAST_PATH".into(), "on".into()),
            ("APP_LIMIT".into(), "4".into()),
        ])
        .unwrap()
    };
    let resolve_from = |source: &SettingsSource| {
        let (mut catalog, serving, hosted) = batter_flags_core::CatalogBuilder::new(
            batter_flags_core::SettingsPrefix::new("FLAG_").unwrap(),
            "serving",
            "hosted",
        )
        .unwrap();
        catalog
            .mode_flag("fast_path", Toggle::Off)
            .declare()
            .unwrap();
        resolve(
            catalog.build(),
            source,
            serving,
            hosted,
            CivilDate::new(2026, 11, 5).unwrap(),
        )
    };
    let narrowed = capture()
        .select_environment(&["APP_LIMIT"], &["APP_"])
        .unwrap();
    assert!(matches!(
        resolve_from(&narrowed),
        Err(crate::declared::DeclaredError::NarrowedSource)
    ));
    let mut layered = capture();
    layered.overlay(narrowed);
    assert!(matches!(
        resolve_from(&layered),
        Err(crate::declared::DeclaredError::NarrowedSource)
    ));
    assert!(resolve_from(&capture()).is_ok(), "the capture resolves");
}
