//! A catalog with every declaration shape, and helpers that resolve and
//! publish it.
#![allow(dead_code, reason = "shared support; each test crate uses part of it")]

use std::sync::Arc;

use batter_flags_core::{
    Catalog, CatalogBuilder, KillSwitch, ModeFlag, ProcessKind, ReleaseFlag, SafeState,
    SettingsPrefix, StaleRule, SwitchState, Toggle, Version,
    low_level::{Base, BaseBuilder, Candidate, CandidateRow, Snapshot},
};

use crate::fixtures::{Checkout, expiry};

/// The handles of [`world`].
///
/// | declaration | shape |
/// | --- | --- |
/// | `checkout_halt` | kill switch, every kind, safe state released, holds its last state |
/// | `export_halt` | kill switch, `worker` only, safe state engaged, engages when stale |
/// | `new_checkout` | release toggle owned by `serving`, `pinned_by` `worker`, governed by `checkout_halt`, `on` needs `PAYMENT_API_URL` |
/// | `fast_path` | release toggle, every kind |
/// | `checkout_style` | mode flag over [`Checkout`], every kind |
/// | `debug_panel` | mode toggle, every kind, only in `development` |
/// | `worker_mode` | mode toggle owned by `worker` alone |
pub struct Handles {
    pub serving: ProcessKind,
    pub worker: ProcessKind,
    pub checkout_halt: KillSwitch,
    pub export_halt: KillSwitch,
    pub new_checkout: ReleaseFlag<Toggle>,
    pub fast_path: ReleaseFlag<Toggle>,
    pub checkout_style: ModeFlag<Checkout>,
    pub debug_panel: ModeFlag<Toggle>,
    pub worker_mode: ModeFlag<Toggle>,
}

/// Builds the catalog and its handles.
pub fn world() -> (Catalog, Handles) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    let development = builder.environment_class("development").unwrap();
    builder.environment_class("hosted").unwrap();
    let checkout_halt = builder
        .kill_switch(
            "checkout_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .declare()
        .unwrap();
    let export_halt = builder
        .kill_switch(
            "export_halt",
            SafeState::Engaged,
            StaleRule::EngageWhenStale,
        )
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    let new_checkout = builder
        .release_flag("new_checkout", Toggle::Off, expiry())
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .governed_by(&checkout_halt)
        .requires_settings(Toggle::On, &["PAYMENT_API_URL"])
        .declare()
        .unwrap();
    let fast_path = builder
        .release_flag("fast_path", Toggle::Off, expiry())
        .declare()
        .unwrap();
    let checkout_style = builder
        .mode_flag("checkout_style", Checkout::Classic)
        .declare()
        .unwrap();
    let debug_panel = builder
        .mode_flag("debug_panel", Toggle::Off)
        .only_in(&development)
        .declare()
        .unwrap();
    let worker_mode = builder
        .mode_flag("worker_mode", Toggle::Off)
        .owned_only_by(&worker)
        .declare()
        .unwrap();
    let catalog = builder.build().unwrap();
    let handles = Handles {
        serving,
        worker,
        checkout_halt,
        export_halt,
        new_checkout,
        fast_path,
        checkout_style,
        debug_panel,
        worker_mode,
    };
    (catalog, handles)
}

/// What a process's settings supplied.
#[derive(Clone, Copy, Debug)]
pub enum Literal<'a> {
    /// A flag's variant.
    Flag(&'a str, &'a str),
    /// A kill switch's state.
    Switch(&'a str, SwitchState),
    /// An enabling variant whose required settings were absent.
    SettingsAbsent(&'a str, &'a str),
}

/// Resolves `catalog` for `kind` with the given settings.
pub fn base(catalog: Catalog, kind: &ProcessKind, literals: &[Literal<'_>]) -> Base {
    let mut builder = BaseBuilder::new(catalog, kind.clone()).unwrap();
    let key = |builder: &BaseBuilder, name: &str| {
        builder.catalog().declaration(name).unwrap().key().clone()
    };
    for literal in literals {
        match *literal {
            Literal::Flag(name, variant) => {
                let key = key(&builder, name);
                builder.flag(&key, variant).unwrap();
            }
            Literal::Switch(name, state) => {
                let key = key(&builder, name);
                builder.switch(&key, state).unwrap();
            }
            Literal::SettingsAbsent(name, variant) => {
                let key = key(&builder, name);
                builder.settings_absent(&key, variant).unwrap();
            }
        }
    }
    builder.finish()
}

pub fn version(counter: u64) -> Version {
    Version::new(counter)
}

/// Validates `rows` read at `at` for `base`.
pub fn snapshot(
    base: &Base,
    at: u64,
    rows: impl IntoIterator<Item = CandidateRow>,
) -> Arc<Snapshot> {
    let scope = base.validation_scope();
    let candidate = Candidate::new(&scope).at(version(at)).rows(rows);
    Arc::new(Snapshot::validate(&scope, candidate).unwrap())
}

/// Validates `rows` read at `at` from the last-good file for `base`.
pub fn file_snapshot(
    base: &Base,
    at: u64,
    rows: impl IntoIterator<Item = CandidateRow>,
) -> Arc<Snapshot> {
    let scope = base.validation_scope();
    let candidate = Candidate::from_last_good_file(&scope)
        .at(version(at))
        .rows(rows);
    Arc::new(Snapshot::validate(&scope, candidate).unwrap())
}

/// A switch row at encoding 1.
pub fn switch_row(switch: &KillSwitch, state: SwitchState, updated: u64) -> CandidateRow {
    CandidateRow::switch(switch, state, version(updated))
}

/// A flag row at encoding 1.
pub fn flag_row<F: batter_flags_core::Flag>(
    flag: &F,
    variant: F::Value,
    updated: u64,
) -> CandidateRow {
    CandidateRow::flag(flag, variant, version(updated))
}

/// A row whose state this build cannot decode.
pub fn undecodable_row(key: &str, kind: &str, value_type: &str, updated: u64) -> CandidateRow {
    CandidateRow::new(
        key,
        kind,
        value_type,
        version(updated),
        r#"{"encoding":2,"variant":"on"}"#,
    )
}
