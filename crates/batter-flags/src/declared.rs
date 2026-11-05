//! Declared mode: values resolved once at startup from the application's
//! settings source.
//!
//! [`resolve`] consumes the application's catalog, reads the settings source
//! the root captured, once, and returns a [`Resolved`]. It enforces, with
//! diagnostics that never carry a setting's value:
//!
//! - strict literals: a toggle is `true`, `false`, `on` or `off` in any ASCII
//!   letter case, an enum flag is exactly one of its declared variant names and
//!   a kill switch is exactly `engaged` or `released`; blank, case variants of
//!   an enum name and non-Unicode values fail;
//! - ownership: a value for a flag the process kind does not own, or for an
//!   `only_in` flag outside its class, fails, so a deployment cannot configure
//!   what no process here evaluates;
//! - absence: a missing setting resolves to the code default (a kill switch to
//!   its safe state), unless the flag is `Absence::Required` and a value may be
//!   presented here;
//! - retired names, unknown names under the prefix and the declared
//!   `requires_settings` and invariants, so a typo never resolves silently to a
//!   default.
//!
//! A flag's value changes by changing its setting and restarting. Nothing here
//! reads the process environment: [`resolve`] reads only the source it is
//! given, so the root decides which layers, files and variables count. Secrets
//! are never flag values: a flag's value is one of a closed list of declared
//! names.
//!
//! ```
//! use batter_core::settings::SettingsSource;
//! use batter_flags::{
//!     declared,
//!     core::{CatalogBuilder, CivilDate, Expiry, SafeState, SettingsPrefix, StaleRule, Toggle},
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // The catalog is declared once, in code.
//! let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
//! let serving = catalog.process_kind("serving")?;
//! let hosted = catalog.environment_class("hosted")?;
//! let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
//! let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
//! catalog.kill_switch("checkout_halt", SafeState::Released, StaleRule::HoldLastKnown).declare()?;
//! let catalog = catalog.build()?;
//!
//! // The root captures its settings and resolves before narrowing them.
//! let source = SettingsSource::from_pairs([("FLAG_NEW_CHECKOUT".into(), "On".into())])?;
//! let today = CivilDate::new(2026, 11, 5)?;
//! let resolved = declared::resolve(catalog, &source, serving, hosted, today)?;
//!
//! // Startup code reads release and mode flags from the resolved base.
//! let decision = resolved.startup_view().evaluate(&new_checkout);
//! assert_eq!(decision.value(), Toggle::On);
//!
//! // One call ends startup: the flags are what gates and surfaces read.
//! let flags = resolved.into_flags();
//! # let _ = flags;
//! # Ok(())
//! # }
//! ```
//!
//! A secret is not a flag value, so a flag over one does not compile:
//!
//! ```compile_fail,E0277
//! use batter_core::settings::SecretString;
//! use batter_flags::core::{CatalogBuilder, SettingsPrefix};
//!
//! let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
//! let _flag = catalog.mode_flag("api_key", SecretString::new("x")).declare();
//! ```

mod applicability;
mod error;
mod plan;
mod read;

use std::fmt;

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    Catalog, CivilDate, Decision, EnvironmentClass, Flag, ProcessKind, Revision, SettingName,
    low_level::{Base, BaseBuilder},
};

pub(crate) use self::applicability::{Applicability, Process};
pub use self::error::{DeclaredError, ValueProblem};
use self::plan::Plan;
use crate::{flags::Flags, reader::Reader, report::StartupReport};

/// Resolves `catalog` for one process from `source`, once.
///
/// `kind` and `class` say what this process is: which flags it owns and which
/// `only_in` flags may carry a value. `today` is the date the root read, and
/// serves only the expiry report. The call never reads the process
/// environment or the clock.
///
/// `source` is the root's unnarrowed capture, or an overlay of layers that each
/// kept the flag names; resolve before the root narrows its own settings to
/// the names its application schema declares, or the flag names are gone and
/// every flag falls back to its default. [`StartupReport`] shows that
/// symptom as `absent` for every key a value may be presented for.
///
/// Expiry never changes a decision and never fails startup: release flags
/// whose last valid day is before `today` are reported once, in one WARN event
/// on target `batter::flags`.
///
/// # Errors
///
/// [`DeclaredError`], the first failure in a fixed order: a foreign kind or
/// class, a declared setting that is misplaced, missing or not a literal,
/// a present retired name, an unknown name under the prefix, a resolved
/// variant whose required settings are absent, and a broken invariant between
/// two flags this kind owns. An invariant whose other flag belongs to another
/// kind is not checked here, because this process cannot know what that kind
/// resolves.
///
/// ```
/// use batter_core::settings::SettingsSource;
/// use batter_flags::{declared::{self, DeclaredError}, core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle}};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// let hosted = catalog.environment_class("hosted")?;
/// catalog.release_flag("new_checkout", Toggle::Off, Expiry::on(CivilDate::new(2027, 3, 31)?)).declare()?;
/// let catalog = catalog.build()?;
///
/// // `1` is not a toggle value: the flag's key is named, the value never is.
/// let source = SettingsSource::from_pairs([("FLAG_NEW_CHECKOUT".into(), "1".into())])?;
/// let error = declared::resolve(catalog, &source, serving, hosted, CivilDate::new(2026, 11, 5)?)
///     .unwrap_err();
/// assert!(matches!(error, DeclaredError::InvalidValue { .. }));
/// assert_eq!(
///     error.to_string(),
///     "setting FLAG_NEW_CHECKOUT for flag `new_checkout` is not a toggle value (true, false, on or off)",
/// );
/// # Ok(())
/// # }
/// ```
pub fn resolve(
    catalog: Catalog,
    source: &SettingsSource,
    kind: ProcessKind,
    class: EnvironmentClass,
    today: CivilDate,
) -> Result<Resolved, DeclaredError> {
    if !catalog.environment_classes().contains(&class) {
        return Err(DeclaredError::ForeignClass {
            class: class.name().to_owned(),
        });
    }
    if !catalog.process_kinds().contains(&kind) {
        return Err(DeclaredError::ForeignKind {
            kind: kind.name().to_owned(),
        });
    }
    let mut builder = BaseBuilder::new(catalog, kind).map_err(DeclaredError::Inconsistent)?;
    let process = Process {
        kind: builder.process_kind(),
        class: &class,
    };
    let plan = Plan::read(builder.catalog(), process, source)?;
    plan.apply(&mut builder)?;
    let base = builder.finish();
    warn_expired(base.catalog(), today);
    Ok(Resolved { base, class })
}

/// Reports the release flags past their expiry, once, in one event.
fn warn_expired(catalog: &Catalog, today: CivilDate) {
    let expired: Vec<&str> = catalog
        .expired(today)
        .map(|declared| declared.key().as_str())
        .collect();
    if !expired.is_empty() {
        expired_event(&expired.join(","), expired.len(), today);
    }
}

fn expired_event(keys: &str, count: usize, today: CivilDate) {
    tracing::event!(
        name: "flags.expired",
        target: "batter::flags",
        tracing::Level::WARN,
        batter.flags.expired = keys,
        batter.flags.expired.count = count,
        batter.flags.today = %today,
        "release flags are past their expiry and should be removed",
    );
}

/// A catalog resolved against one process's settings, before any mode.
///
/// Resolution happens once, so the base this holds is the single description
/// of what the process resolved: its revisions, its report and, behind
/// [`startup_view`](Self::startup_view), the release and mode values startup
/// code may need. It is consumed by exactly one thing, [`into_flags`](Self::into_flags)
/// for declared mode, so a resolved catalog cannot be turned into two
/// diverging sets of flags.
///
/// It is neither `Clone` nor `Copy`:
///
/// ```compile_fail,E0382
/// # use batter_flags::declared::Resolved;
/// # fn resolved() -> Resolved { unimplemented!() }
/// let resolved = resolved();
/// let _flags = resolved.into_flags();
/// let _again = resolved.into_flags();
/// ```
#[must_use = "resolved flags do nothing until `into_flags` consumes them"]
pub struct Resolved {
    base: Base,
    /// The class this process runs in, which the base does not keep: it decides
    /// which keys a value may be presented for, and so what the report says.
    class: EnvironmentClass,
}

impl Resolved {
    /// The revision of every declaration's resolved variant, in key order. A
    /// declaration this process kind does not own contributes its code default
    /// (a kill switch its safe state). Two processes of one kind that resolved
    /// alike have the same revision.
    pub fn revision(&self) -> Revision {
        self.base.revision()
    }

    /// The same revision over the declarations every process kind owns, the
    /// one set every kind resolves: comparing it across kinds shows whether the
    /// fleet agrees on shared flags.
    pub fn shared_revision(&self) -> Revision {
        self.base.shared_revision()
    }

    /// The catalog this was resolved from.
    pub fn catalog(&self) -> &Catalog {
        self.base.catalog()
    }

    /// Every setting name resolution reads, in name order: each declared flag's
    /// and each retired name.
    ///
    /// A root whose own reserved settings prefixes cover the flag prefix lists
    /// these as known, so its application schema does not reject them.
    pub fn setting_names(&self) -> Vec<SettingName> {
        let catalog = self.base.catalog();
        let mut names: Vec<SettingName> = catalog
            .declarations()
            .map(|declared| declared.setting_name())
            .chain(
                catalog
                    .retired()
                    .iter()
                    .map(|retired| retired.name().clone()),
            )
            .collect();
        names.sort();
        names
    }

    /// What this process resolved, without any setting's text: each key's
    /// variant and where it came from (a setting, a setting that is absent where
    /// one may be presented, or no setting allowed here), and both revisions.
    pub fn report(&self) -> StartupReport {
        StartupReport::of(&self.base, &self.class)
    }

    /// A read of the release and mode flags over what was resolved, for startup
    /// code that needs a value before the process serves anything.
    ///
    /// It borrows this `Resolved`, so it cannot outlive
    /// [`into_flags`](Self::into_flags):
    ///
    /// ```compile_fail,E0505
    /// # use batter_flags::declared::Resolved;
    /// # fn resolved() -> Resolved { unimplemented!() }
    /// let resolved = resolved();
    /// let startup = resolved.startup_view();
    /// let _flags = resolved.into_flags();
    /// let _ = &startup;
    /// ```
    pub fn startup_view(&self) -> StartupView<'_> {
        StartupView { base: &self.base }
    }

    /// Enters declared mode: the resolved values are what the process serves
    /// until it restarts.
    pub fn into_flags(self) -> Flags {
        Flags::new(Reader::declared(self.base))
    }
}

impl fmt::Debug for Resolved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Resolved")
            .field("process_kind", &self.base.process_kind())
            .field("environment_class", &self.class)
            .field("revision", &self.base.revision())
            .finish_non_exhaustive()
    }
}

/// A startup read of release and mode flags over a [`Resolved`].
///
/// Its decisions report `Source::Declared` whichever mode follows: nothing is
/// published yet and no mode has been entered. It has no `switch` and no `pin`,
/// because kill switches act through gates and a pin records a decision a
/// running surface served:
///
/// ```compile_fail,E0599
/// # use batter_flags::{declared::Resolved, core::KillSwitch};
/// # fn resolved() -> Resolved { unimplemented!() }
/// # fn halt() -> KillSwitch { unimplemented!() }
/// let resolved = resolved();
/// let _ = resolved.startup_view().switch(&halt());
/// ```
///
/// ```compile_fail,E0599
/// # use batter_flags::{declared::Resolved, core::{ReleaseFlag, Toggle}};
/// # fn resolved() -> Resolved { unimplemented!() }
/// # fn flag() -> ReleaseFlag<Toggle> { unimplemented!() }
/// let resolved = resolved();
/// let _ = resolved.startup_view().pin(&flag());
/// ```
pub struct StartupView<'r> {
    base: &'r Base,
}

impl StartupView<'_> {
    /// Evaluates a release or mode flag over the resolved base and emits one
    /// evaluation event, like any other view.
    ///
    /// A handle from another catalog, or for a flag this process kind does not
    /// own, yields its code default with reason `error`.
    pub fn evaluate<F: Flag>(&self, flag: &F) -> Decision<F::Value> {
        self.base.declared_view().evaluate(flag)
    }
}

impl fmt::Debug for StartupView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartupView")
            .field("revision", &self.base.revision())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
