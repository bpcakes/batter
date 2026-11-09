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
//! the process's settings supplied, giving a [`Base`]. A loader keeps one
//! [`Publisher`] over the base's [`ValidationScope`], started
//! [`new`](Publisher::new) or [restored](Publisher::restore) from the
//! last-good file. Each [`Candidate`] it reads goes through
//! [`Publisher::judge`] and [`Judged::publish`], and readers build a
//! [`SnapshotView`](crate::SnapshotView) from the [`Record`] it serves with
//! [`Base::record_view`], so a view's freshness is always judged from the
//! evidence of the snapshot it serves. The publisher carries held kill-switch
//! states into every read, refuses regressions and drift, and counts a file's
//! age from registration: none of those is left to the loader.
//!
//! [`Snapshot::validate`] and the `*_view` constructors on [`Base`] also build
//! standalone snapshots and views, for evaluation over a snapshot no
//! publisher judged. Such a view asserts its own freshness, and a standalone
//! candidate carries only what [`Candidate::carrying`] gives it.
//!
//! Migration: `Published::first`, `Published::next` and `Published::rejected`
//! are no longer public, and a loader no longer calls [`Candidate::carrying`].
//! Keep a [`Publisher`], advance it with `publisher.judge(candidate).publish(at)`
//! and read it with [`Base::record_view`].
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
//! - Assert the freshness of every standalone view truthfully. Its source
//!   follows from the snapshot's [`Provenance`]: a view refuses a snapshot
//!   read from the other source.
//!
//! ```
//! use batter_flags_core::{
//!     CatalogBuilder, CivilDate, Expiry, Flag, PrePublication, Reason, SettingsPrefix, Toggle,
//!     Version,
//!     low_level::{BaseBuilder, Candidate, CandidateRow, Disposition, Publisher},
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
//! let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
//! let new_checkout = catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
//! let base = BaseBuilder::new(catalog.build(), serving)?.finish();
//!
//! // Ticks stand in for a monotonic instant.
//! let mut publisher = Publisher::new(&base.validation_scope(), 0_u64);
//! let candidate = Candidate::new(&base.validation_scope(), Version::new(7))
//!     .row(CandidateRow::flag(&new_checkout, Toggle::On, Version::new(7)));
//! assert_eq!(publisher.judge(candidate).publish(10), Disposition::Applied);
//!
//! let view = base.record_view(publisher.record(), PrePublication::Unready, |at| 20 - at <= 60)?;
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
    published::{Disposition, Judged, Published, Publisher, Record, Restored, Unpublished},
    snapshot::{
        CarriedSwitch, ForeignSnapshot, HoldCause, Provenance, Snapshot, SnapshotRejection,
        ValidationScope,
    },
    view::{GateReading, ViewError, gate_switch},
};
