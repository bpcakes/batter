//! Flags as a root gets them in declared mode, and the refusal texts every gate
//! shares.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    CatalogBuilder, CivilDate, FlagKey, KillSwitch, SafeState, SettingsPrefix, Source, StaleRule,
    SwitchState,
};

use crate::{Flags, GateError, declared};

/// Flags of a one-switch catalog, resolved from `pairs` as `serving`.
fn declared_flags(pairs: &[(&str, &str)]) -> (Flags, KillSwitch) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    let hosted = builder.environment_class("hosted").unwrap();
    let halt = builder
        .kill_switch("shared_halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let source = SettingsSource::from_pairs(
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    )
    .unwrap();
    let today = CivilDate::new(2026, 11, 5).unwrap();
    let resolved =
        declared::resolve(builder.build().unwrap(), &source, serving, hosted, today).unwrap();
    (resolved.into_flags(), halt)
}

#[test]
fn a_gate_over_declared_flags_reads_the_setting_the_process_resolved() {
    let (flags, halt) = declared_flags(&[("FLAG_SHARED_HALT", "engaged")]);
    let entered = flags.gate(&halt).unwrap().enter();
    assert_eq!(entered.reading.decision().value(), SwitchState::Engaged);
    assert!(matches!(entered.view.source(), Source::Declared { .. }));

    // Declared mode has no waiting window: an absent setting is the safe state.
    let (flags, halt) = declared_flags(&[]);
    let entered = flags.gate(&halt).unwrap().enter();
    assert_eq!(entered.reading.decision().value(), SwitchState::Released);
    assert!(matches!(
        entered.reading,
        batter_flags_core::low_level::GateReading::Open(_)
    ));
}

#[test]
fn the_refusals_say_what_to_change() {
    let key = |name: &str| FlagKey::new(name).unwrap();
    assert_eq!(
        GateError::NotOwned {
            key: key("export_halt")
        }
        .to_string(),
        "kill switch `export_halt` is not owned by this process kind"
    );
    assert_eq!(
        GateError::ForeignHandle {
            key: key("shared_halt")
        }
        .to_string(),
        "kill switch `shared_halt` belongs to another catalog"
    );
    assert_eq!(
        GateError::DuplicateSwitch {
            key: key("shared_halt")
        }
        .to_string(),
        "kill switch `shared_halt` is listed more than once"
    );
    assert_eq!(
        GateError::DeferOnWorkflowStep.to_string(),
        "a job that may run as a workflow step cannot defer while its switch is engaged"
    );
}

mod fixture_order {
    use batter_flags_core::{Toggle, Version};

    use crate::test::TestFlags;

    /// Where in the chain the version is set.
    #[derive(Clone, Copy)]
    enum Placement {
        First,
        Last,
    }

    /// One fixture, built with the version set first or last.
    fn digest(order: Placement, version: u64) -> batter_flags_core::SnapshotDigest {
        let mut catalog = batter_flags_core::CatalogBuilder::new(
            batter_flags_core::SettingsPrefix::new("FLAG_").unwrap(),
        );
        let serving = catalog.process_kind("serving").unwrap();
        catalog.environment_class("hosted").unwrap();
        let expiry =
            batter_flags_core::Expiry::on(batter_flags_core::CivilDate::new(2027, 3, 31).unwrap());
        let flag = catalog
            .release_flag("fast_path", Toggle::Off, expiry)
            .declare()
            .unwrap();
        let halt = catalog
            .kill_switch(
                "halt",
                batter_flags_core::SafeState::Released,
                batter_flags_core::StaleRule::HoldLastKnown,
            )
            .declare()
            .unwrap();
        let builder = TestFlags::builder(catalog.build().unwrap(), serving).unwrap();
        let built = match order {
            Placement::First => builder
                .version(Version::new(version))
                .set(&flag, Toggle::On)
                .engage(&halt),
            Placement::Last => builder
                .set(&flag, Toggle::On)
                .engage(&halt)
                .version(Version::new(version)),
        };
        built
            .build()
            .unwrap()
            .flags()
            .reader
            .fixture_digest()
            .unwrap()
    }

    #[test]
    fn the_order_of_the_builders_calls_changes_nothing() {
        assert_eq!(digest(Placement::First, 7), digest(Placement::Last, 7));
        // The digest does cover the version, so the equality above is not vacuous.
        assert_ne!(digest(Placement::First, 7), digest(Placement::First, 8));
    }
}
