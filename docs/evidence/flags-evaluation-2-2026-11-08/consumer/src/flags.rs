//! The service's one flags catalog.
//!
//! Settings use the `FLAG_` prefix: `FLAG_CHECKOUT_HALT=engaged|released` and
//! `FLAG_EXPRESS_RECEIPT=on|off`. In control-plane mode those settings are the
//! declared base the process serves until the first publication, and an
//! `engaged` halt is a settings floor no store write can release.

use batter::{
    BoxError,
    flags::{
        core::{
            Catalog, CatalogBuilder, CivilDate, EnvironmentClass, Expiry, KillSwitch, ProcessKind,
            ReleaseFlag, SafeState, SettingsPrefix, StaleRule, Toggle,
        },
        declared::{self, Resolved},
    },
    settings::SettingsSource,
};
use chrono::Datelike;

/// Kill switch that disables the checkout route group and holds receipts.
pub const CHECKOUT_HALT: &str = "checkout_halt";
/// Release flag selecting the express receipt for newly accepted checkouts.
pub const EXPRESS_RECEIPT: &str = "express_receipt";

/// The typed handles the service evaluates through.
#[derive(Clone, Debug)]
pub struct Handles {
    pub checkout_halt: KillSwitch,
    pub express_receipt: ReleaseFlag<Toggle>,
}

/// The built catalog, the kind and class this process runs as, and the handles.
pub struct Declared {
    pub catalog: Catalog,
    pub serving: ProcessKind,
    pub hosted: EnvironmentClass,
    pub handles: Handles,
}

/// Declares every flag and kill switch once.
pub fn declare() -> Result<Declared, BoxError> {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    let serving = catalog.process_kind("serving")?;
    let hosted = catalog.environment_class("hosted")?;
    // Released unless an operator engages it. While the control plane cannot be
    // reached the last known state is held, so a flags-database outage neither
    // takes checkout down nor silently releases an engaged halt.
    let checkout_halt = catalog
        .kill_switch(CHECKOUT_HALT, SafeState::Released, StaleRule::HoldLastKnown)
        .declare()?;
    // Deliberately not governed by the halt: a receipt replays the variant pinned
    // at acceptance, and the halt already stops the job type as a whole.
    let express_receipt = catalog
        .release_flag(
            EXPRESS_RECEIPT,
            Toggle::Off,
            Expiry::on(CivilDate::new(2027, 3, 31)?),
        )
        .declare()?;
    Ok(Declared {
        catalog: catalog.build()?,
        serving,
        hosted,
        handles: Handles {
            checkout_halt,
            express_receipt,
        },
    })
}

/// Resolves the catalog once against the root's unnarrowed settings capture.
pub fn resolve(
    settings: &SettingsSource,
    today: CivilDate,
) -> Result<(Resolved, Handles), BoxError> {
    let declared = declare()?;
    let resolved = declared::resolve(
        declared.catalog,
        settings,
        declared.serving,
        declared.hosted,
        today,
    )?;
    Ok((resolved, declared.handles))
}

/// Today's civil date in UTC, which only the expiry report reads.
pub fn today() -> Result<CivilDate, BoxError> {
    let today = chrono::Utc::now().date_naive();
    Ok(CivilDate::new(
        u16::try_from(today.year())?,
        u8::try_from(today.month())?,
        u8::try_from(today.day())?,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> SettingsSource {
        SettingsSource::from_pairs(pairs.iter().map(|(k, v)| ((*k).into(), (*v).into()))).unwrap()
    }

    #[test]
    fn settings_supply_the_startup_values() {
        let (resolved, handles) = resolve(
            &settings(&[
                ("FLAG_EXPRESS_RECEIPT", "on"),
                ("FLAG_CHECKOUT_HALT", "engaged"),
            ]),
            today().unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolved
                .startup_view()
                .evaluate(&handles.express_receipt)
                .value(),
            Toggle::On
        );
        let report = resolved.report().to_string();
        assert!(report.contains("express_receipt=on"), "{report}");
        assert!(report.contains("checkout_halt=engaged"), "{report}");
    }

    #[test]
    fn a_mistyped_flag_setting_fails_startup() {
        assert!(
            resolve(
                &settings(&[("FLAG_EXPRESS_RECIEPT", "on")]),
                today().unwrap()
            )
            .is_err()
        );
        assert!(resolve(&settings(&[("FLAG_CHECKOUT_HALT", "on")]), today().unwrap()).is_err());
    }
}
