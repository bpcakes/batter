//! Declared resolution records what a process could not serve for every flag
//! it replays pins of, so a pin taken elsewhere is refused where it would
//! serve a variant this process never could: a `pinned_by` kind lacking the
//! variant's settings, and any kind outside an `only_in` flag's class.

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, PinRejection, Pinned, PinnedResolution, ReleaseFlag,
    SettingsPrefix, Toggle,
};

use crate::declared::{Resolved, resolve};

/// Which process resolves the catalog.
#[derive(Clone, Copy)]
enum As {
    ServingHosted,
    ServingDevelopment,
    WorkerHosted,
}

/// `new_checkout` is owned by `serving`, `pinned_by` `worker`, and `on` needs
/// a payment URL; `preview` is owned by both and restricted to development.
fn resolve_as(process: As, pairs: &[(&str, &str)]) -> (Resolved, [ReleaseFlag<Toggle>; 2]) {
    let (mut builder, serving, hosted) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let worker = builder.process_kind("worker").unwrap();
    let development = builder.environment_class("development").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let new_checkout = builder
        .release_flag("new_checkout", Toggle::Off, expiry)
        .owned_only_by(&serving)
        .pinned_by(&worker)
        .requires_settings(Toggle::On, &["PAYMENT_API_URL"])
        .declare()
        .unwrap();
    let preview = builder
        .release_flag("preview", Toggle::Off, expiry)
        .only_in(&development)
        .declare()
        .unwrap();
    let (kind, class) = match process {
        As::ServingHosted => (serving, hosted),
        As::ServingDevelopment => (serving, development),
        As::WorkerHosted => (worker, hosted),
    };
    let source = SettingsSource::from_pairs(
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into())),
    )
    .unwrap();
    let today = CivilDate::new(2026, 11, 5).unwrap();
    let resolved = resolve(builder.build(), &source, kind, class, today).unwrap();
    (resolved, [new_checkout, preview])
}

fn pin(process: As, pairs: &[(&str, &str)], which: usize) -> Pinned<Toggle> {
    let (resolved, flags) = resolve_as(process, pairs);
    resolved.base.declared_view().pin(&flags[which]).unwrap()
}

/// Whether `pin` is honoured where `process` replays it, or why it is refused.
fn replay(
    process: As,
    pairs: &[(&str, &str)],
    which: usize,
    pin: &Pinned<Toggle>,
) -> Result<(), PinRejection> {
    let (resolved, flags) = resolve_as(process, pairs);
    match resolved
        .base
        .declared_view()
        .resolve_pinned(&flags[which], pin)
    {
        PinnedResolution::Honoured(_) => Ok(()),
        refused => Err(refused.rejection().unwrap()),
    }
}

const URL: (&str, &str) = ("PAYMENT_API_URL", "https://payments.example");

#[test]
fn a_pin_only_worker_lacking_the_variants_settings_refuses_its_pin() {
    let on = pin(As::ServingHosted, &[URL, ("FLAG_NEW_CHECKOUT", "on")], 0);
    assert_eq!(on.variant(), Some("on"));
    // The worker owes no settings for a flag it does not own, and starts.
    assert_eq!(
        replay(As::WorkerHosted, &[], 0, &on),
        Err(PinRejection::RequiresSettingsAbsent)
    );
    assert_eq!(replay(As::WorkerHosted, &[URL], 0, &on), Ok(()));
}

#[test]
fn a_pin_of_a_development_only_flag_is_refused_in_a_hosted_process() {
    let on = pin(As::ServingDevelopment, &[("FLAG_PREVIEW", "on")], 1);
    assert_eq!(
        replay(As::ServingHosted, &[], 1, &on),
        Err(PinRejection::OutsideClass)
    );
    assert_eq!(replay(As::ServingDevelopment, &[], 1, &on), Ok(()));
    // The code default is what a hosted process serves, so its pin replays.
    let off = pin(As::ServingDevelopment, &[], 1);
    assert_eq!(replay(As::ServingHosted, &[], 1, &off), Ok(()));
}
