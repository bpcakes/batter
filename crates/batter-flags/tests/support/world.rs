//! A catalog with every declaration shape declared mode checks, and helpers
//! that resolve it from injected settings.
#![allow(dead_code, reason = "shared support; each test crate uses part of it")]
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]

use batter_core::settings::SettingsSource;
use batter_flags::{
    core::{
        Catalog, CatalogBuilder, CivilDate, EnvironmentClass, Expiry, FlagValue, KillSwitch,
        ModeFlag, ProcessKind, ReleaseFlag, SafeState, SettingsPrefix, StaleRule, Toggle,
    },
    declared::{self, DeclaredError, Resolved},
};

/// A two-variant application enum.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Checkout {
    Classic,
    Express,
}

impl FlagValue for Checkout {
    const VARIANTS: &'static [&'static str] = &["classic", "express"];

    fn variant(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Express => "express",
        }
    }

    fn from_variant(variant: &str) -> Option<Self> {
        match variant {
            "classic" => Some(Self::Classic),
            "express" => Some(Self::Express),
            _ => None,
        }
    }
}

/// The handles of [`world`].
///
/// | declaration | shape |
/// | --- | --- |
/// | `checkout_halt` | kill switch, every kind, safe state released |
/// | `export_halt` | kill switch, `worker` only, safe state engaged, engages when stale |
/// | `new_checkout` | release toggle owned by `serving`, `pinned_by` `worker`, `on` needs `PAYMENT_API_URL` |
/// | `fast_path` | release toggle, every kind |
/// | `checkout_style` | mode flag over [`Checkout`], every kind |
/// | `region_lock` | mode toggle, every kind, setting required |
/// | `debug_panel` | mode toggle, every kind, only in `development` |
/// | `worker_mode` | mode toggle owned by `worker` alone |
/// | `ledger_v2`, `ledger_audit` | mode toggles: v2 `on` requires audit `on` |
/// | `legacy_export`, `bulk_export` | mode toggles that conflict while both `on` |
/// | `sync_in`, `sync_out` | mode toggles that are both `on` or neither |
/// | `worker_sync` | mode toggle owned by `worker`; `ledger_v2` `on` requires it `on` |
/// | `worker_region` | mode toggle owned by `worker`, setting required there |
/// | `dev_tools` | mode toggle only in `development`, setting required there |
#[derive(Debug)]
pub struct Handles {
    pub serving: ProcessKind,
    pub worker: ProcessKind,
    pub development: EnvironmentClass,
    pub hosted: EnvironmentClass,
    pub checkout_halt: KillSwitch,
    pub export_halt: KillSwitch,
    pub new_checkout: ReleaseFlag<Toggle>,
    pub fast_path: ReleaseFlag<Toggle>,
    pub checkout_style: ModeFlag<Checkout>,
    pub region_lock: ModeFlag<Toggle>,
    pub debug_panel: ModeFlag<Toggle>,
    pub worker_mode: ModeFlag<Toggle>,
    pub worker_region: ModeFlag<Toggle>,
    pub dev_tools: ModeFlag<Toggle>,
}

/// The expiry of every release flag of [`world`].
pub fn expiry() -> Expiry {
    Expiry::on(CivilDate::new(2027, 3, 31).unwrap())
}

/// The date the tests resolve on, before every expiry.
pub fn today() -> CivilDate {
    CivilDate::new(2026, 11, 5).unwrap()
}

/// The flags that exist to be related, the relationships between them and the
/// retired names: nothing the tests read through a handle.
fn declare_relationships(builder: &mut CatalogBuilder, worker: &ProcessKind) {
    let ledger_v2 = builder
        .mode_flag("ledger_v2", Toggle::Off)
        .declare()
        .unwrap();
    let ledger_audit = builder
        .mode_flag("ledger_audit", Toggle::Off)
        .declare()
        .unwrap();
    let legacy_export = builder
        .mode_flag("legacy_export", Toggle::Off)
        .declare()
        .unwrap();
    let bulk_export = builder
        .mode_flag("bulk_export", Toggle::Off)
        .declare()
        .unwrap();
    let sync_in = builder.mode_flag("sync_in", Toggle::Off).declare().unwrap();
    let sync_out = builder
        .mode_flag("sync_out", Toggle::Off)
        .declare()
        .unwrap();
    let worker_sync = builder
        .mode_flag("worker_sync", Toggle::Off)
        .owned_only_by(worker)
        .declare()
        .unwrap();
    builder
        .requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)
        .unwrap();
    builder
        .requires(&ledger_v2, Toggle::On, &worker_sync, Toggle::On)
        .unwrap();
    builder
        .conflicts(&legacy_export, Toggle::On, &bulk_export, Toggle::On)
        .unwrap();
    builder.both_or_none(&sync_in, &sync_out).unwrap();
    builder
        .retired("FLAG_OLD_CHECKOUT", "use FLAG_NEW_CHECKOUT instead")
        .unwrap();
    builder
        .retired("LEGACY_FEATURE_SWITCH", "removed; set FLAG_FAST_PATH")
        .unwrap();
}

/// Builds the catalog and its handles.
pub fn world() -> (Catalog, Handles) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    let development = builder.environment_class("development").unwrap();
    let hosted = builder.environment_class("hosted").unwrap();
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
        .requires_settings(Toggle::On, &["PAYMENT_API_URL", "PAYMENT_API_REGION"])
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
    let region_lock = builder
        .mode_flag("region_lock", Toggle::Off)
        .required()
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
    let worker_region = builder
        .mode_flag("worker_region", Toggle::Off)
        .owned_only_by(&worker)
        .required()
        .declare()
        .unwrap();
    let dev_tools = builder
        .mode_flag("dev_tools", Toggle::Off)
        .only_in(&development)
        .required()
        .declare()
        .unwrap();
    declare_relationships(&mut builder, &worker);
    let catalog = builder.build().unwrap();
    let handles = Handles {
        serving,
        worker,
        development,
        hosted,
        checkout_halt,
        export_halt,
        new_checkout,
        fast_path,
        checkout_style,
        region_lock,
        debug_panel,
        worker_mode,
        worker_region,
        dev_tools,
    };
    (catalog, handles)
}

/// Which process a resolution is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum As {
    ServingHosted,
    ServingDevelopment,
    WorkerHosted,
}

/// The settings a resolution of [`world`] as `process` needs: every required
/// flag where a value may be presented.
pub fn required(process: As) -> Vec<(&'static str, &'static str)> {
    let mut required = vec![("FLAG_REGION_LOCK", "off")];
    if process == As::WorkerHosted {
        required.push(("FLAG_WORKER_REGION", "off"));
    }
    if process == As::ServingDevelopment {
        required.push(("FLAG_DEV_TOOLS", "off"));
    }
    required
}

/// An injected source of `pairs` on top of `base`, later pairs winning.
pub fn source_over(base: &[(&str, &str)], pairs: &[(&str, &str)]) -> SettingsSource {
    let mut merged: Vec<(&str, &str)> = base.to_vec();
    for (name, value) in pairs {
        merged.retain(|(existing, _)| existing != name);
        merged.push((name, value));
    }
    SettingsSource::from_pairs(
        merged
            .into_iter()
            .map(|(name, value)| (name.into(), value.into())),
    )
    .unwrap()
}

/// Resolves a fresh [`world`] as `process` from `pairs` on top of what
/// `process` requires, and returns the handles of the catalog it resolved.
pub fn resolve_as(
    process: As,
    pairs: &[(&str, &str)],
) -> Result<(Resolved, Handles), DeclaredError> {
    let (catalog, h) = world();
    let (kind, class) = match process {
        As::ServingHosted => (h.serving.clone(), h.hosted.clone()),
        As::ServingDevelopment => (h.serving.clone(), h.development.clone()),
        As::WorkerHosted => (h.worker.clone(), h.hosted.clone()),
    };
    let source = source_over(&required(process), pairs);
    declared::resolve(catalog, &source, kind, class, today()).map(|resolved| (resolved, h))
}

/// Resolves a fresh [`world`] as `serving` in `hosted` from `pairs`.
pub fn resolve_serving(pairs: &[(&str, &str)]) -> Result<(Resolved, Handles), DeclaredError> {
    resolve_as(As::ServingHosted, pairs)
}

/// Resolves a fresh [`world`] as `worker` in `hosted` from `pairs`.
pub fn resolve_worker(pairs: &[(&str, &str)]) -> Result<(Resolved, Handles), DeclaredError> {
    resolve_as(As::WorkerHosted, pairs)
}

/// Resolves a fresh [`world`] as `serving` in `development` from `pairs`.
pub fn resolve_development(pairs: &[(&str, &str)]) -> Result<(Resolved, Handles), DeclaredError> {
    resolve_as(As::ServingDevelopment, pairs)
}
