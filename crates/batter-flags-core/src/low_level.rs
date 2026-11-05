//! The disclosed publication path.
//!
//! Rust has no friend crates, so everything `batter-flags` needs to mint a
//! view is public here. That makes this a weaker path than the canonical one:
//! a caller of this module asserts the [`Source`](crate::Source) and the
//! [`Freshness`](crate::Freshness) of every view it builds, and this crate has
//! no way to tell the `batter-flags` reader from any other caller. There is no
//! "unverified" marker, because it would be on every view or on none.
//!
//! The views the protected surfaces hand to application code (an admitted HTTP
//! request, a job invocation, a periodic run) are captured by `batter-flags`
//! from its own reader, and none of them accepts a view built here. This module
//! is not a provider interface and never the way to test: injected test flags
//! are `batter-flags`'s `TestFlags`, which builds on it.
//!
//! The path, in order: a [`BaseBuilder`] consumes the catalog and records what
//! the process's settings supplied, giving a [`Base`]; a [`Candidate`] read for
//! the base's [`ValidationScope`] becomes a [`Snapshot`] through
//! [`Snapshot::validate`]; the base then builds a
//! [`SnapshotView`](crate::SnapshotView) over it. A loader keeps what it
//! publishes as one [`Published`] record, advanced only through
//! [`Published::next`] and read through [`Base::published_view`], so a view's
//! freshness is always judged from the evidence of the snapshot it serves.
//!
//! # Caller obligations
//!
//! Nothing on this path can tell a complete record from an incomplete one, so
//! each of these is the caller's to meet:
//!
//! - Record every literal the process's settings supplied. An unrecorded
//!   literal serves the declared default.
//! - Record every enabling variant whose required settings were absent or
//!   empty, with [`BaseBuilder::settings_absent`]. An unrecorded absence
//!   counts the settings as present, so a row selecting that variant is served
//!   and not held.
//! - Carry the snapshot a candidate replaces, with [`Candidate::carrying`], on
//!   every load after the first. A candidate that does not carry releases a
//!   held, engaged kill switch to its safe state.
//! - Assert each view's freshness truthfully. Its source follows from the
//!   snapshot's [`Provenance`]: a view refuses a snapshot read from the other
//!   source.
//!
//! ```
//! use std::sync::Arc;
//!
//! use batter_flags_core::{
//!     CatalogBuilder, CivilDate, Expiry, Flag, Freshness, Reason, SettingsPrefix, Toggle, Version,
//!     low_level::{BaseBuilder, Candidate, CandidateRow, Snapshot},
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
//! let serving = catalog.process_kind("serving")?;
//! catalog.environment_class("hosted")?;
//! let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
//! let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
//! let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
//!
//! let scope = base.validation_scope();
//! let candidate = Candidate::new(&scope)
//!     .at(Version::new(7))
//!     .row(CandidateRow::flag(&new_checkout, Toggle::On, Version::new(7)));
//! let snapshot = Arc::new(Snapshot::validate(&scope, candidate)?);
//!
//! let view = base.control_plane_view(&snapshot, Freshness::Fresh)?;
//! let decision = view.evaluate(&new_checkout);
//! assert_eq!(decision.value(), Toggle::On);
//! assert_eq!(decision.reason(), Reason::Static);
//! # Ok(())
//! # }
//! ```

pub use crate::{
    authority::{Authority, Refusal, Standing, Transition},
    base::{Base, BaseBuilder, BaseError, Resolution, ResolutionSource},
    candidate::{Candidate, CandidateRow},
    check::HandleRefusal,
    published::{Disposition, Published},
    snapshot::{
        CarriedSwitch, ForeignSnapshot, HoldCause, Provenance, Snapshot, SnapshotRejection,
        ValidationScope,
    },
    view::{GateReading, ViewError, gate_switch},
};
