//! Read-only comparison of a managed-object ledger and catalog with its manifest.

use super::{
    InspectionFragment, PgTransaction, QualifiedName, VerificationError,
    report::{Finding, FindingKind, SupportedSurface, UnsupportedSurface},
};
use crate::managed::{
    self, LedgerRow, LedgerShape, LiveObject, ManagedObjectManifest, PublicExecute, RenderSettings,
    SET_ROW,
};
use sqlx::{Connection, PgConnection};
use std::collections::{HashMap, HashSet};

/// Outcome of the pre-snapshot `ACCESS SHARE` attempt on the managed ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ManagedLock {
    Locked,
    Missing,
    WrongKind,
}

/// Lock the ledger before the repeatable-read snapshot exists.
///
/// Apply holds `ACCESS EXCLUSIVE` on the ledger for its whole transaction, so
/// this lock waits for an in-flight apply and blocks a new one until the
/// verifier rolls back. Catalog rendering reads PostgreSQL's catalog caches,
/// not the snapshot; the lock keeps a library apply from committing between
/// the snapshot's ledger rows and that rendering. Only `SAVEPOINT`, `LOCK` and
/// `RELEASE` run here, none of which takes the snapshot.
pub(super) async fn lock_before_snapshot(
    transaction: &mut PgTransaction<'_>,
    ledger: &QualifiedName,
) -> Result<ManagedLock, VerificationError> {
    let mut guard = transaction
        .begin()
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    let locked = sqlx::query(sqlx::AssertSqlSafe(format!(
        "LOCK TABLE {} IN ACCESS SHARE MODE",
        ledger.quoted()
    )))
    .persistent(false)
    .execute(&mut *guard)
    .await;
    match locked {
        Ok(_) => {
            guard
                .commit()
                .await
                .map_err(|error| VerificationError::Native(error.into()))?;
            Ok(ManagedLock::Locked)
        }
        Err(error) => {
            let code = error.as_database_error().and_then(|error| error.code());
            let outcome = match code.as_deref() {
                Some("42P01" | "3F000") => Some(ManagedLock::Missing),
                Some("42809") => Some(ManagedLock::WrongKind),
                _ => None,
            };
            if let Err(rollback) = guard.rollback().await {
                return Err(VerificationError::Rollback {
                    primary: Some(Box::new(VerificationError::Native(error.into()))),
                    source: rollback.into(),
                });
            }
            outcome.ok_or_else(|| VerificationError::Native(error.into()))
        }
    }
}

fn native(error: sqlx::Error) -> VerificationError {
    VerificationError::Native(error.into())
}

fn catalog(error: managed::CatalogError) -> VerificationError {
    match error {
        managed::CatalogError::Native(error) => native(error),
        managed::CatalogError::Capacity => VerificationError::CatalogCapacity,
    }
}

pub(super) async fn inspect(
    transaction: &mut PgTransaction<'_>,
    manifest: &ManagedObjectManifest,
    lock: ManagedLock,
) -> Result<InspectionFragment, VerificationError> {
    let connection: &mut PgConnection = transaction;
    let ledger = manifest.ledger();
    let mut fragment = InspectionFragment {
        supported: vec![SupportedSurface::ManagedObjects],
        evaluated_items: manifest.objects().len() + manifest.retirements().len(),
        ..InspectionFragment::default()
    };
    let ledger_finding = |kind| Finding::new(kind, Some(ledger.quoted()), None::<String>, None);
    match lock {
        ManagedLock::Locked => {}
        ManagedLock::Missing => {
            // A ledger visible now was created after the lock attempt and is
            // not protected; absence in the snapshot is a finding.
            if managed::named_relation(connection, ledger)
                .await
                .map_err(native)?
                .is_some()
            {
                fragment
                    .unsupported
                    .push(UnsupportedSurface::UnprotectedManagedObjectLedger);
            } else {
                fragment
                    .findings
                    .push(ledger_finding(FindingKind::MissingManagedObjectLedger));
            }
            return Ok(fragment);
        }
        ManagedLock::WrongKind => {
            fragment
                .findings
                .push(ledger_finding(FindingKind::ManagedObjectLedgerShape));
            return Ok(fragment);
        }
    }
    let Some(oid) = locked_ledger(connection, ledger).await? else {
        fragment
            .unsupported
            .push(UnsupportedSurface::UnprotectedManagedObjectLedger);
        return Ok(fragment);
    };
    match managed::inspect_ledger_shape(connection, oid)
        .await
        .map_err(native)?
    {
        LedgerShape::Exact => {}
        LedgerShape::RowSecurity => {
            fragment
                .findings
                .push(ledger_finding(FindingKind::LedgerRowSecurity));
            return Ok(fragment);
        }
        LedgerShape::Inherited => {
            fragment
                .unsupported
                .push(UnsupportedSurface::UnprotectedManagedObjectLedger);
            return Ok(fragment);
        }
        LedgerShape::Mismatch => {
            fragment
                .findings
                .push(ledger_finding(FindingKind::ManagedObjectLedgerShape));
            return Ok(fragment);
        }
    }
    let rows = match managed::load_ledger_rows(connection, ledger).await {
        Ok(rows) => rows,
        Err(managed::CatalogError::Capacity) => {
            fragment
                .findings
                .push(ledger_finding(FindingKind::MigrationLedgerLimit));
            return Ok(fragment);
        }
        Err(error) => return Err(catalog(error)),
    };
    fragment.evaluated_items = fragment.evaluated_items.saturating_add(rows.len());
    compare_rows(manifest, &rows, &mut fragment.findings);
    inspect_catalog(connection, manifest, &rows, &mut fragment).await?;
    Ok(fragment)
}

/// Render every live definition under the canonical settings and compare it,
/// retired identities and the `PUBLIC` policy with the recorded ledger.
async fn inspect_catalog(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
    rows: &[LedgerRow],
    fragment: &mut InspectionFragment,
) -> Result<(), VerificationError> {
    // The verifier's transaction is rolled back, so the settings need no restore.
    let _settings = RenderSettings::apply(connection).await.map_err(native)?;
    let live = managed::resolve_live(connection, manifest)
        .await
        .map_err(catalog)?;
    let hashes = managed::render_hashes(
        connection,
        live.values()
            .filter_map(|object| object.kind.map(|kind| (object.oid, kind))),
    )
    .await
    .map_err(catalog)?;
    fragment.evaluated_items = fragment.evaluated_items.saturating_add(live.len());
    compare_catalog(manifest, rows, &live, &hashes, &mut fragment.findings);
    if manifest.public_execute() != PublicExecute::Revoke {
        return Ok(());
    }
    let routines = public_execute_candidates(manifest, &live);
    let oids = routines.keys().copied().collect::<Vec<_>>();
    let holders = managed::public_execute_holders(connection, &oids)
        .await
        .map_err(native)?;
    let mut executable = holders
        .iter()
        .filter_map(|oid| routines.get(oid))
        .collect::<Vec<_>>();
    executable.sort_unstable();
    for identity in executable {
        fragment.findings.push(Finding::new(
            FindingKind::ManagedObjectPublicExecute,
            Some((*identity).to_owned()),
            Some("public".to_owned()),
            Some(super::ObjectPrivilege::Execute),
        ));
    }
    Ok(())
}

/// Live managed routines of their declared kind, keyed by oid: the only
/// objects that can hold `EXECUTE`. Triggers of either kind are excluded.
pub(super) fn public_execute_candidates<'m>(
    manifest: &'m ManagedObjectManifest,
    live: &HashMap<String, LiveObject>,
) -> HashMap<i64, &'m str> {
    manifest
        .objects()
        .iter()
        .filter(|object| object.kind().is_routine())
        .filter_map(|object| {
            live.get(object.identity())
                .filter(|live| live.kind == Some(object.kind()))
                .map(|live| (live.oid, object.identity()))
        })
        .collect()
}

/// The ledger's oid when the snapshot-visible relation is the one locked
/// before the snapshot; a replaced or dropped name is not coupled.
async fn locked_ledger(
    connection: &mut PgConnection,
    ledger: &QualifiedName,
) -> Result<Option<i64>, VerificationError> {
    sqlx::query_scalar(
        "SELECT c.oid::bigint
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         JOIN pg_catalog.pg_locks AS l ON l.relation = c.oid
         WHERE n.nspname = $1 AND c.relname = $2
           AND l.pid = pg_catalog.pg_backend_pid()
           AND l.locktype = 'relation'
           AND l.mode = 'AccessShareLock'
           AND l.granted
         LIMIT 1",
    )
    .bind(ledger.schema())
    .bind(ledger.name())
    .fetch_optional(connection)
    .await
    .map_err(native)
}

fn object_finding(kind: FindingKind, identity: &str) -> Finding {
    Finding::new(kind, Some(identity.to_owned()), None::<String>, None)
}

/// Compare ledger rows with the manifest: the set row, every declared entry
/// and every undeclared row.
pub(super) fn compare_rows(
    manifest: &ManagedObjectManifest,
    rows: &[LedgerRow],
    findings: &mut Vec<Finding>,
) {
    let recorded = rows
        .iter()
        .map(|row| (row.identity.as_str(), row))
        .collect::<HashMap<_, _>>();
    let declared = manifest
        .objects()
        .iter()
        .map(|object| object.identity())
        .collect::<HashSet<_>>();
    let rendered = managed::rendered_set_digest(
        rows.iter()
            .filter(|row| declared.contains(row.identity.as_str()))
            .map(|row| (row.identity.as_str(), row.rendered_checksum.as_slice())),
    );
    let set_recorded = recorded.get(SET_ROW).is_some_and(|row| {
        row.kind == SET_ROW
            && row.source_checksum == manifest.digest().as_bytes()
            && row.rendered_checksum == rendered
    });
    if !set_recorded {
        findings.push(Finding::new(
            FindingKind::ManagedObjectSetDigest,
            Some(manifest.ledger().quoted()),
            None::<String>,
            None,
        ));
    }
    for object in manifest.objects() {
        match recorded.get(object.identity()) {
            None => findings.push(object_finding(
                FindingKind::MissingManagedObject,
                object.identity(),
            )),
            Some(row)
                if row.kind != object.kind().as_str()
                    || row.source_checksum != object.checksum() =>
            {
                findings.push(object_finding(
                    FindingKind::ManagedObjectChecksumMismatch,
                    object.identity(),
                ));
            }
            Some(_) => {}
        }
    }
    let mut unexpected = rows
        .iter()
        .map(|row| row.identity.as_str())
        .filter(|identity| *identity != SET_ROW && !declared.contains(identity))
        .collect::<Vec<_>>();
    unexpected.sort_unstable();
    for identity in unexpected {
        findings.push(object_finding(
            FindingKind::UnexpectedManagedObject,
            identity,
        ));
    }
}

/// Compare the live catalog with what apply recorded: drift of recorded
/// entries and presence of retired objects.
pub(super) fn compare_catalog(
    manifest: &ManagedObjectManifest,
    rows: &[LedgerRow],
    live: &HashMap<String, LiveObject>,
    hashes: &HashMap<i64, [u8; 32]>,
    findings: &mut Vec<Finding>,
) {
    let recorded = rows
        .iter()
        .map(|row| (row.identity.as_str(), row))
        .collect::<HashMap<_, _>>();
    for object in manifest.objects() {
        let Some(row) = recorded.get(object.identity()) else {
            continue;
        };
        let current = live
            .get(object.identity())
            .filter(|live| live.kind == Some(object.kind()))
            .and_then(|live| hashes.get(&live.oid));
        if current.is_none_or(|hash| row.rendered_checksum != hash) {
            findings.push(object_finding(
                FindingKind::ManagedObjectDrift,
                object.identity(),
            ));
        }
    }
    for retirement in manifest.retirements() {
        // Another kind under the same identity is not the retired object.
        if live
            .get(retirement.identity())
            .is_some_and(|live| live.kind == Some(retirement.kind()))
        {
            findings.push(object_finding(
                FindingKind::RetiredManagedObjectPresent,
                retirement.identity(),
            ));
        }
    }
}

#[cfg(test)]
mod tests;
