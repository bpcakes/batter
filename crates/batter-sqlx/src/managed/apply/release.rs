//! A locked ledger authorizes one manifest before object mutation.

use super::{
    ManagedApplyError, ManagedObjectManifest, ManagedReleaseError, ManagedRevision, SET_ROW,
    catalog, statement,
};
use sqlx::{AssertSqlSafe, PgConnection};

pub(super) enum LedgerOrigin {
    Created,
    Existing,
}

pub(super) struct AuthorizedRelease<'a>(&'a ManagedObjectManifest);

impl<'a> AuthorizedRelease<'a> {
    pub(super) fn manifest(&self) -> &'a ManagedObjectManifest {
        self.0
    }
}

pub(super) fn authorize<'a>(
    manifest: &'a ManagedObjectManifest,
    rows: &[catalog::LedgerRow],
    origin: LedgerOrigin,
) -> Result<AuthorizedRelease<'a>, ManagedApplyError> {
    let refuse = ManagedApplyError::Release;
    if matches!(origin, LedgerOrigin::Created) && rows.is_empty() {
        return Ok(AuthorizedRelease(manifest));
    }
    let row = rows
        .iter()
        .find(|row| row.identity == SET_ROW)
        .ok_or_else(|| refuse(ManagedReleaseError::InvalidLedger))?;
    if row.kind != SET_ROW
        || row.source_checksum.len() != 32
        || row.rendered_checksum.len() != 32
        || rows
            .iter()
            .any(|row| row.identity != SET_ROW && row.release_revision != 0)
    {
        return Err(refuse(ManagedReleaseError::InvalidLedger));
    }
    let installed = u64::try_from(row.release_revision)
        .ok()
        .and_then(ManagedRevision::new)
        .ok_or_else(|| refuse(ManagedReleaseError::InvalidLedger))?;
    let requested = manifest.revision();
    if requested < installed {
        return Err(refuse(ManagedReleaseError::Downgrade {
            requested,
            installed,
        }));
    }
    if requested == installed && row.source_checksum != manifest.digest().as_bytes() {
        return Err(refuse(ManagedReleaseError::RevisionConflict {
            revision: requested,
        }));
    }
    Ok(AuthorizedRelease(manifest))
}

/// The caller holds ACCESS EXCLUSIVE and has verified the exact legacy shape.
/// A baseline may establish ordering, but may not simultaneously change content.
pub(super) async fn upgrade_legacy(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
) -> Result<(), ManagedApplyError> {
    let ledger = manifest.ledger().quoted();
    let row = sqlx::query_as::<_, (String, Vec<u8>)>(AssertSqlSafe(format!(
        "SELECT kind, source_checksum FROM ONLY {ledger} WHERE identity = 'manifest'"
    )))
    .persistent(false)
    .fetch_optional(&mut *connection)
    .await
    .map_err(statement)?;
    if !row.is_some_and(|(kind, digest)| kind == SET_ROW && digest == manifest.digest().as_bytes())
    {
        return Err(ManagedApplyError::Release(
            ManagedReleaseError::LegacyBaselineRequired,
        ));
    }
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {ledger} ADD COLUMN release_revision pg_catalog.int8 NOT NULL DEFAULT 0"
    )))
    .persistent(false)
    .execute(&mut *connection)
    .await
    .map_err(statement)?;
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE ONLY {ledger} SET release_revision = $1 WHERE identity = 'manifest'"
    )))
    .persistent(false)
    .bind(manifest.revision().get())
    .execute(connection)
    .await
    .map_err(statement)?;
    Ok(())
}
