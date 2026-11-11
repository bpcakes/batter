//! The startup report: what a process resolved, with no setting values.

use std::fmt;

use batter_flags_core::{
    EnvironmentClass, FlagKey, Revision,
    low_level::{Base, ResolutionSource},
};

use crate::declared::{Applicability, Process};

/// Where a resolved variant came from.
///
/// `Absent` is meaningful only because the other two are separate: it says a
/// value may be presented for the key in this process and none was, so the key
/// served its code default (a kill switch its safe state). When an application
/// expected a setting to be present, `Absent` on the keys it should have set is
/// the visible symptom of a source that was narrowed without the flag names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ValueSource {
    /// A setting supplied the variant.
    Literal,
    /// A value may be presented for the key here, and no setting was present.
    Absent,
    /// No value may be presented for the key here: the process kind does not
    /// own the declaration, or it is restricted to another environment class.
    /// The variant is its code default, and a setting for it would have failed
    /// startup.
    NotApplicable,
}

impl ValueSource {
    /// The stable word the report prints: `literal`, `absent` or
    /// `not_applicable`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Literal => "literal",
            Self::Absent => "absent",
            Self::NotApplicable => "not_applicable",
        }
    }

    /// What a resolution's source means for a declaration the process may or
    /// may not present a value for.
    fn of(applicability: &Applicability, source: ResolutionSource) -> Self {
        match (applicability.may_present(), source) {
            (false, ResolutionSource::Literal | ResolutionSource::Absent) => Self::NotApplicable,
            (true, ResolutionSource::Literal) => Self::Literal,
            (true, ResolutionSource::Absent) => Self::Absent,
        }
    }
}

/// One declaration as a process resolved it.
///
/// It names the key, the variant it resolved to (a declared variant name, or
/// `engaged` or `released` for a kill switch) and its [`ValueSource`]. The
/// setting's own text is never kept, so it cannot be reported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportEntry {
    key: FlagKey,
    variant: &'static str,
    source: ValueSource,
}

impl ReportEntry {
    /// The declaration's key.
    pub fn key(&self) -> &FlagKey {
        &self.key
    }

    /// The resolved variant.
    pub fn variant(&self) -> &'static str {
        self.variant
    }

    /// Whether a setting or the default supplied it.
    pub fn source(&self) -> ValueSource {
        self.source
    }
}

impl fmt::Display for ReportEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}={} ({})",
            self.key,
            self.variant,
            self.source.as_str()
        )
    }
}

/// What a process resolved at startup: every declaration's variant and source,
/// and the two revisions a fleet compares.
///
/// The report holds declared names and digests only; formatting it with `{}`,
/// `{:?}` or `{:#?}` can never show a setting's text. A root logs it once at
/// startup so an operator can see which flags came from settings and which
/// fell back to their defaults.
///
/// ```
/// use batter_flags::{ValueSource, core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle}, declared};
/// use batter_core::settings::SettingsSource;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, hosted) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// catalog.release_flag("fast_path", Toggle::Off, expiry).declare()?;
/// let catalog = catalog.build();
///
/// let source = SettingsSource::from_pairs([("FLAG_NEW_CHECKOUT".into(), "on".into())])?;
/// let resolved = declared::resolve(catalog, &source, serving, hosted, CivilDate::new(2026, 11, 5)?)?;
///
/// let report = resolved.report();
/// let lines: Vec<String> = report.entries().iter().map(ToString::to_string).collect();
/// assert_eq!(lines, ["fast_path=off (absent)", "new_checkout=on (literal)"]);
/// assert_eq!(report.entries()[1].source(), ValueSource::Literal);
/// assert_eq!(report.revision(), resolved.revision());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupReport {
    entries: Vec<ReportEntry>,
    revision: Revision,
    shared_revision: Revision,
}

impl StartupReport {
    pub(crate) fn of(base: &Base, class: &EnvironmentClass) -> Self {
        let process = Process {
            kind: base.process_kind(),
            class,
        };
        Self {
            entries: base
                .resolved()
                .map(|resolution| ReportEntry {
                    key: resolution.key().clone(),
                    variant: resolution.variant(),
                    source: ValueSource::of(
                        &Applicability::of(&resolution.declared(), process),
                        resolution.source(),
                    ),
                })
                .collect(),
            revision: base.revision(),
            shared_revision: base.shared_revision(),
        }
    }

    /// Every declaration, in key order.
    pub fn entries(&self) -> &[ReportEntry] {
        &self.entries
    }

    /// The revision over every declaration this process resolved.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The revision over the declarations every process kind owns.
    pub fn shared_revision(&self) -> Revision {
        self.shared_revision
    }
}

impl fmt::Display for StartupReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "flags revision={} shared_revision={}",
            self.revision.to_hex(),
            self.shared_revision.to_hex()
        )?;
        for entry in &self.entries {
            write!(formatter, "\n  {entry}")?;
        }
        Ok(())
    }
}
