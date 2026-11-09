//! Views of what a publisher serves.

use super::{Published, Record};
use crate::{
    decision::{Freshness, PrePublication},
    snapshot::Provenance,
    view::{SnapshotView, ViewError},
};

impl crate::base::Base {
    /// A view of a published record, with the source its snapshot was read from
    /// and the freshness `recent` finds in that record's own age origin.
    ///
    /// # Errors
    ///
    /// [`ViewError::ForeignSnapshot`] when the record's snapshot was validated
    /// for another base.
    pub fn published_view<C: Clone>(
        &self,
        published: &Published<C>,
        recent: impl FnOnce(&C) -> bool,
    ) -> Result<SnapshotView, ViewError> {
        let freshness = published.freshness(recent);
        match published.snapshot.provenance() {
            Provenance::ControlPlane => self.control_plane_view(&published.snapshot, freshness),
            Provenance::LastGoodFile => self.persisted_file_view(&published.snapshot, freshness),
        }
    }

    /// A view of whatever `record` serves: before the first publication, a
    /// not-loaded view under `pre_publication`, aged from registration; after
    /// it, the [published view](Self::published_view).
    ///
    /// # Errors
    ///
    /// [`ViewError::ForeignSnapshot`] when the record's snapshot was validated
    /// for another base.
    pub fn record_view<C: Clone>(
        &self,
        record: &Record<C>,
        pre_publication: PrePublication,
        recent: impl FnOnce(&C) -> bool,
    ) -> Result<SnapshotView, ViewError> {
        match record {
            Record::Unpublished(unpublished) => Ok(self.not_loaded_view(
                pre_publication,
                if recent(&unpublished.registered) {
                    Freshness::Fresh
                } else {
                    Freshness::Stale
                },
            )),
            Record::Published(published) => self.published_view(published, recent),
        }
    }
}
