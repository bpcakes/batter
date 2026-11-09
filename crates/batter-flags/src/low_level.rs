//! Disclosed, non-canonical access to the live snapshot.
//!
//! The canonical paths capture one view per request, invocation or run from
//! the registered flags, and hand it to application code through a surface
//! that gated it. The escape hatch here is deliberately weaker: a caller that
//! captures a view itself takes on the obligations the rustdoc of
//! [`capture_now`] discloses, and no protected surface accepts what it
//! produces.

use batter_flags_core::SnapshotView;

use crate::flags::Flags;

/// Captures a view of `flags` as of now.
///
/// This is not the canonical path. A view is frozen when it is captured, and
/// whatever the caller does with it afterwards is the caller's:
///
/// - A view captured here is not the view any gate, job wrapper or periodic
///   run used. Evaluating it in the middle of work that was gated by another
///   capture can show a different state than the one the work was admitted
///   under.
/// - No protected surface accepts it: an HTTP route group, a job wrapper and a
///   periodic gate read only views they captured themselves, so passing this
///   view to one is not possible.
/// - It is not a fixture. Tests build views from
///   `test::TestFlags`, behind the `test-support` feature, which reports
///   `Source::Test`.
///
/// It takes `&Flags` and never fails: it does no I/O, never panics and emits
/// no event until the view evaluates something.
///
/// ```
/// use batter_core::settings::SettingsSource;
/// use batter_flags::{
///     declared, low_level,
///     core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Source, Toggle},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, hosted) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// let catalog = catalog.build();
///
/// let source = SettingsSource::from_pairs([("FLAG_NEW_CHECKOUT".into(), "on".into())])?;
/// let flags = declared::resolve(catalog, &source, serving, hosted, CivilDate::new(2026, 11, 5)?)?
///     .into_flags();
///
/// let view = low_level::capture_now(&flags);
/// let decision = view.evaluate(&new_checkout);
/// assert_eq!(decision.value(), Toggle::On);
/// assert!(matches!(decision.source(), Source::Declared { .. }));
/// # Ok(())
/// # }
/// ```
pub fn capture_now(flags: &Flags) -> SnapshotView {
    flags.capture()
}
