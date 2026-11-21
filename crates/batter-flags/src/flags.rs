//! The flags a process serves: the one capability protected surfaces read.

use std::{fmt, sync::Arc};

use batter_flags_core::SnapshotView;

use crate::reader::Reader;

/// The flags of one process: what gates, job wrappers and periodic gates read
/// their views from.
///
/// In declared mode it comes from [`Resolved::into_flags`](crate::declared::Resolved::into_flags);
/// the fixture path is `test::TestFlags`, behind the `test-support` feature. Cloning is cheap
/// and every clone reads the same flags, so each gate or component that needs
/// them takes its own clone.
///
/// ```
/// use batter_core::settings::SettingsSource;
/// use batter_flags::{
///     declared,
///     core::{CatalogBuilder, CivilDate, SettingsPrefix},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, hosted) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let catalog = catalog.build();
///
/// let source = SettingsSource::from_pairs([])?;
/// let flags = declared::resolve(catalog, &source, serving, hosted, CivilDate::new(2026, 11, 5)?)?
///     .into_flags();
///
/// // Each gate that is built from the flags owns a clone of them.
/// let for_the_http_gate = flags.clone();
/// let for_the_worker = flags.clone();
/// # let _ = (for_the_http_gate, for_the_worker);
/// # Ok(())
/// # }
/// ```
///
/// `Flags` deliberately has no method that returns a view. A handler gets a
/// view only from a surface that captured it once for its unit of work, so
/// the view it evaluates is the one that was gated, and application code
/// cannot capture a fresh view in the middle of a request and see another
/// state. The one public live read is the disclosed
/// [`low_level::capture_now`](crate::low_level::capture_now), which is not the
/// canonical path:
///
/// ```compile_fail,E0624
/// # use batter_flags::Flags;
/// # fn flags() -> Flags { unimplemented!() }
/// let _view = flags().capture();
/// ```
#[derive(Clone)]
pub struct Flags {
    reader: Arc<Reader>,
}

impl Flags {
    /// Flags over a reader the control plane also holds.
    #[cfg(feature = "postgres")]
    pub(crate) fn shared(reader: Arc<Reader>) -> Self {
        Self { reader }
    }

    pub(crate) fn new(reader: Reader) -> Self {
        Self {
            reader: Arc::new(reader),
        }
    }

    /// One view, frozen now: the only way a surface reads these flags.
    pub(crate) fn capture(&self) -> SnapshotView {
        self.reader.capture()
    }

    /// See [`Reader::capture_for_work`](crate::reader::Reader::capture_for_work).
    #[cfg_attr(
        not(feature = "runledger"),
        allow(
            dead_code,
            reason = "only the job surface, behind `runledger`, defers ungated work"
        )
    )]
    pub(crate) fn capture_for_work(&self) -> crate::reader::WorkCapture {
        self.reader.capture_for_work()
    }
}

impl fmt::Debug for Flags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Flags")
            .field("process_kind", self.reader.base().process_kind())
            .finish_non_exhaustive()
    }
}

pub(crate) mod gate;

#[cfg(test)]
mod tests;
