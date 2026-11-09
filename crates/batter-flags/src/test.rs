//! Test flags: the fixture path for handlers, jobs and components under test.
//!
//! Tests build a catalog's values directly and hand them to the surface under
//! test, without a settings source or a control plane. Every decision of a
//! test view reports `Source::Test` with the injected version and freshness,
//! derives its reason like a loaded view does, and applies each kill switch's
//! stale rule when the injected freshness is stale, so `EngageWhenStale` is
//! testable without a database. This is the fixture path; the escape hatch in
//! `low_level` is not.
//!
//! The module exists only with the `test-support` feature, or the facade's
//! `flags-test-support`, so a production root cannot serve `Source::Test`.
//!
//! Migration: `batter_flags::test::TestFlags` was always compiled; enable
//! `test-support` (facade: `flags-test-support`) in the dev-dependency that
//! uses it.

use std::collections::BTreeSet;

use batter_flags_core::{
    Catalog, Flag, FlagKey, Freshness, KillSwitch, ProcessKind, SnapshotView, SwitchState, Version,
    low_level::{
        BaseBuilder, BaseError, Candidate, CandidateRow, HandleRefusal, Snapshot, SnapshotRejection,
    },
};

use crate::{flags::Flags, reader::Reader};

/// A row not yet written. The version it is written at is the one the fixture
/// ends up with, so the order of the builder's calls changes nothing.
type Row = Box<dyn FnOnce(Version) -> CandidateRow + Send + Sync>;

/// Why test flags could not be built.
///
/// Every variant names a declared key or kind, never application data. The
/// first mistake a builder met is the one reported.
///
/// ```
/// use batter_flags::{
///     test::{FixtureError, TestFlags},
///     core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let build = || -> Result<_, Box<dyn std::error::Error>> {
///     let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
///     let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
///     let flag = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
///     Ok((catalog.build(), serving, flag))
/// };
/// let (catalog, serving, _flag) = build()?;
/// let (_other_catalog, _other_kind, foreign_flag) = build()?;
///
/// // A handle from another catalog would never be read by these flags, so the
/// // fixture refuses it when it is built, instead of serving a default.
/// let error = TestFlags::builder(catalog, serving)?
///     .set(&foreign_flag, Toggle::On)
///     .build()
///     .unwrap_err();
/// assert!(matches!(error, FixtureError::ForeignHandle { .. }));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum FixtureError {
    /// The process kind was declared by another catalog.
    #[error("process kind `{kind}` belongs to another catalog")]
    ForeignKind {
        /// The kind's declared name.
        kind: String,
    },
    /// A handle was minted by another catalog.
    #[error("`{key}` belongs to another catalog")]
    ForeignHandle {
        /// The handle's key.
        key: FlagKey,
    },
    /// The process kind does not own the declaration, so nothing injected for
    /// it would ever be read.
    #[error("`{key}` is not owned by this process kind, so an injected value would never be read")]
    NotOwned {
        /// The declaration's key.
        key: FlagKey,
    },
    /// A value, state or floor was injected twice for one declaration.
    #[error("`{key}` was injected twice")]
    Repeated {
        /// The declaration's key.
        key: FlagKey,
    },
    /// The assembled snapshot was refused. Building it from handles cannot
    /// do this; the variant exists so the builder never panics.
    #[error("the injected rows were refused: {0}")]
    Rejected(SnapshotRejection),
    /// The base refused a floor. Checked handles cannot do this; the variant
    /// exists so the builder never panics.
    #[error("the injected floor was refused: {0}")]
    Inconsistent(BaseError),
}

/// Flags with injected values, for tests.
///
/// ```
/// use batter_flags::{
///     test::TestFlags,
///     core::{CatalogBuilder, CivilDate, Expiry, Freshness, SafeState, SettingsPrefix, Source,
///            StaleRule, SwitchState, Toggle, Version},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// let halt = catalog
///     .kill_switch("checkout_halt", SafeState::Released, StaleRule::EngageWhenStale)
///     .declare()?;
/// let catalog = catalog.build();
///
/// let flags = TestFlags::builder(catalog, serving)?
///     .set(&new_checkout, Toggle::On)
///     .release(&halt)
///     .version(Version::new(7))
///     .freshness(Freshness::Fresh)
///     .build()?;
///
/// let view = flags.view();
/// assert_eq!(view.evaluate(&new_checkout).value(), Toggle::On);
/// assert_eq!(
///     view.source(),
///     Source::Test { version: Version::new(7), freshness: Freshness::Fresh },
/// );
/// assert_eq!(view.switch(&halt).value(), SwitchState::Released);
/// # Ok(())
/// # }
/// ```
pub struct TestFlags {
    flags: Flags,
}

impl TestFlags {
    /// Starts test flags for `kind` over `catalog`, with nothing injected,
    /// version 1 and fresh.
    ///
    /// The kind decides what a handle may be injected for, because ownership
    /// decides what a process evaluates: a test for a handler runs as the kind
    /// that serves it.
    ///
    /// # Errors
    ///
    /// [`FixtureError::ForeignKind`] when `kind` belongs to another catalog.
    pub fn builder(catalog: Catalog, kind: ProcessKind) -> Result<TestFlagsBuilder, FixtureError> {
        if !catalog.process_kinds().contains(&kind) {
            return Err(FixtureError::ForeignKind {
                kind: kind.name().to_owned(),
            });
        }
        let base = BaseBuilder::new(catalog, kind).map_err(FixtureError::Inconsistent)?;
        Ok(TestFlagsBuilder {
            base,
            rows: Vec::new(),
            floors: BTreeSet::new(),
            injected: BTreeSet::new(),
            version: Version::new(1),
            freshness: Freshness::Fresh,
            failure: None,
        })
    }

    /// The flags to hand to the surface under test: a gate, a job wrapper or a
    /// periodic gate captures its views from them as it would from the real
    /// flags, and they report `Source::Test`.
    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    /// A view of the injected values, for evaluating directly.
    pub fn view(&self) -> SnapshotView {
        self.flags.capture()
    }
}

impl std::fmt::Debug for TestFlags {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestFlags")
            .field("flags", &self.flags)
            .finish()
    }
}

/// Builds [`TestFlags`].
///
/// Each method injects one thing and returns the builder. A mistake, such as
/// a handle from another catalog, is remembered and reported by
/// [`build`](Self::build), so a fixture reads as one chain.
#[must_use = "a builder does nothing until `build` is called"]
pub struct TestFlagsBuilder {
    base: BaseBuilder,
    rows: Vec<Row>,
    floors: BTreeSet<FlagKey>,
    injected: BTreeSet<FlagKey>,
    version: Version,
    freshness: Freshness,
    failure: Option<FixtureError>,
}

impl TestFlagsBuilder {
    fn fail(&mut self, error: FixtureError) {
        self.failure.get_or_insert(error);
    }

    fn refused(key: &FlagKey, refusal: HandleRefusal) -> FixtureError {
        match refusal {
            HandleRefusal::ForeignCatalog => FixtureError::ForeignHandle { key: key.clone() },
            HandleRefusal::NotOwned => FixtureError::NotOwned { key: key.clone() },
        }
    }

    /// Injects `value` as the published entry of a release or mode flag.
    pub fn set<F: Flag>(mut self, flag: &F, value: F::Value) -> Self {
        match self.base.check_flag(flag) {
            Err(refusal) => self.fail(Self::refused(flag.key(), refusal)),
            Ok(()) if !self.injected.insert(flag.key().clone()) => {
                self.fail(FixtureError::Repeated {
                    key: flag.key().clone(),
                });
            }
            Ok(()) => {
                let flag = flag.clone();
                self.rows
                    .push(Box::new(move |at| CandidateRow::flag(&flag, value, at)));
            }
        }
        self
    }

    /// Injects an engaged published state for a kill switch.
    pub fn engage(self, switch: &KillSwitch) -> Self {
        self.state(switch, SwitchState::Engaged)
    }

    /// Injects a released published state for a kill switch.
    pub fn release(self, switch: &KillSwitch) -> Self {
        self.state(switch, SwitchState::Released)
    }

    fn state(mut self, switch: &KillSwitch, state: SwitchState) -> Self {
        match self.base.check_switch(switch) {
            Err(refusal) => self.fail(Self::refused(switch.key(), refusal)),
            Ok(()) if !self.injected.insert(switch.key().clone()) => {
                self.fail(FixtureError::Repeated {
                    key: switch.key().clone(),
                });
            }
            Ok(()) => {
                let switch = switch.clone();
                self.rows
                    .push(Box::new(move |at| CandidateRow::switch(&switch, state, at)));
            }
        }
        self
    }

    /// Engages a kill switch with its settings floor, as an `engaged` setting
    /// would in a control-plane process: it holds before and whatever is
    /// published.
    pub fn floor(mut self, switch: &KillSwitch) -> Self {
        match self.base.check_switch(switch) {
            Err(refusal) => self.fail(Self::refused(switch.key(), refusal)),
            Ok(()) if !self.floors.insert(switch.key().clone()) => {
                self.fail(FixtureError::Repeated {
                    key: switch.key().clone(),
                });
            }
            Ok(()) => {
                if let Err(error) = self.base.switch(switch.key(), SwitchState::Engaged) {
                    self.fail(FixtureError::Inconsistent(error));
                }
            }
        }
        self
    }

    /// Sets the version every decision reports in its `Source::Test`, and the
    /// version every injected row is written at, whenever in the chain it is
    /// called.
    pub fn version(mut self, version: Version) -> Self {
        self.version = version;
        self
    }

    /// Sets the freshness every decision reports. A stale test view applies
    /// each kill switch's stale rule.
    pub fn freshness(mut self, freshness: Freshness) -> Self {
        self.freshness = freshness;
        self
    }

    /// Builds the flags.
    ///
    /// # Errors
    ///
    /// The first mistake any method met: [`FixtureError::ForeignHandle`],
    /// [`FixtureError::NotOwned`] or [`FixtureError::Repeated`].
    pub fn build(self) -> Result<TestFlags, FixtureError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let base = self.base.finish();
        let scope = base.validation_scope();
        let version = self.version;
        let rows = self.rows.into_iter().map(|row| row(version));
        let candidate = Candidate::new(&scope, version).rows(rows);
        let snapshot = Snapshot::validate(candidate).map_err(FixtureError::Rejected)?;
        let reader = Reader::test(base, std::sync::Arc::new(snapshot), self.freshness);
        Ok(TestFlags {
            flags: Flags::new(reader),
        })
    }
}

impl std::fmt::Debug for TestFlagsBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestFlagsBuilder")
            .field("injected", &self.injected.len())
            .finish_non_exhaustive()
    }
}
