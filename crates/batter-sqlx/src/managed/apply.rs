//! Library-ordered migrate-then-apply on one leased connection.

use super::catalog::{
    self, CatalogError, LedgerShape, LiveObject, RenderSettings, Touched, create_ledger_sql,
};
use super::{
    Identity, ManagedObject, ManagedObjectDigest, ManagedObjectKind, ManagedObjectManifest,
    PublicExecute, SET_ROW,
};
use crate::{PgLease, SqlxFailure};
use sqlx::{AssertSqlSafe, Connection, PgConnection, SqlSafeStr};
use std::{collections::HashMap, fmt};

/// Evidence that a committed apply installed one manifest.
///
/// The counts describe what this apply executed. An entry is re-executed only
/// when its ledger row is absent, its source checksum changed, or its live
/// catalog definition no longer renders to the hash apply last recorded, so a
/// repeated apply of an unchanged, undrifted set executes no DDL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "an applied manifest is release evidence; log or inspect its digest"]
pub struct AppliedManagedObjects {
    digest: ManagedObjectDigest,
    created_or_replaced: usize,
    unchanged: usize,
    retired: usize,
}

impl AppliedManagedObjects {
    /// Return the set digest recorded in the ledger.
    pub const fn digest(&self) -> ManagedObjectDigest {
        self.digest
    }

    /// Return how many entries were created or replaced.
    pub const fn created_or_replaced(&self) -> usize {
        self.created_or_replaced
    }

    /// Return how many entries were already installed and not re-executed.
    pub const fn unchanged(&self) -> usize {
        self.unchanged
    }

    /// Return how many retired objects still existed and were dropped.
    pub const fn retired(&self) -> usize {
        self.retired
    }
}

/// A failed migrate-then-apply. Every variant retires the leased connection.
///
/// Native causes are retained behind fixed, redacted `Display` and `Debug`
/// output; [`std::error::Error::source`] exposes them for trusted inspection.
/// No variant authorizes replay. Apart from
/// [`Self::CommitUnconfirmed`], the managed object transaction never sent
/// `COMMIT`: its effects were never visible and are discarded when the
/// retired session ends. Migrations that the migrator committed remain.
pub enum ManagedApplyError {
    /// The session lock or the SQLx migrator failed before any managed-object
    /// statement ran. A migrator error is retained as `sqlx::Error::Migrate`.
    Migration(SqlxFailure),
    /// A statement in the managed-object transaction failed: ledger creation
    /// or locking, a retirement, a source statement, the `PUBLIC` revoke or a
    /// ledger write. A dependent object blocking a retirement surfaces here.
    Statement(SqlxFailure),
    /// A relation with the ledger's name exists but is not the exact managed
    /// ledger: another kind or shape (such as a SQLx migration ledger), row-level
    /// security, or inheritance. Nothing was written.
    LedgerShape,
    /// The ledger or a catalog read exceeded its bounded capacity.
    Capacity,
    /// A source statement did not create or replace exactly its declared
    /// identity and kind, for example because its argument list declares
    /// other types than the entry's signature.
    IdentityMismatch {
        /// The declared canonical identity the statement failed to produce.
        identity: String,
    },
    /// Under [`PublicExecute::Revoke`], `PUBLIC` still holds `EXECUTE` on a
    /// managed routine after its revoke. PostgreSQL revokes only grants the
    /// revoking role made (as owner), so this happens when the applying login
    /// does not own the routine or another grantor gave `PUBLIC` the privilege.
    PublicExecuteRetained {
        /// The canonical identity `PUBLIC` can still execute.
        identity: String,
    },
    /// `COMMIT` was sent but its outcome was not acknowledged. The objects may
    /// or may not be installed; verification of the ledger decides.
    CommitUnconfirmed(SqlxFailure),
}

impl fmt::Display for ManagedApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Migration(_) => formatter.write_str("migration before managed objects failed"),
            Self::Statement(_) => formatter.write_str("managed object statement failed"),
            Self::LedgerShape => {
                formatter.write_str("managed object ledger relation has an unsupported shape")
            }
            Self::Capacity => formatter.write_str("managed object catalog capacity exceeded"),
            Self::IdentityMismatch { identity } => write!(
                formatter,
                "managed object source did not create its declared identity: {identity}"
            ),
            Self::PublicExecuteRetained { identity } => write!(
                formatter,
                "PUBLIC still holds EXECUTE on a managed routine: {identity}"
            ),
            Self::CommitUnconfirmed(_) => {
                formatter.write_str("managed object commit was not acknowledged")
            }
        }
    }
}

impl fmt::Debug for ManagedApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Migration(_) => "ManagedApplyError::Migration",
            Self::Statement(_) => "ManagedApplyError::Statement",
            Self::LedgerShape => "ManagedApplyError::LedgerShape",
            Self::Capacity => "ManagedApplyError::Capacity",
            Self::IdentityMismatch { .. } => "ManagedApplyError::IdentityMismatch",
            Self::PublicExecuteRetained { .. } => "ManagedApplyError::PublicExecuteRetained",
            Self::CommitUnconfirmed(_) => "ManagedApplyError::CommitUnconfirmed",
        })
    }
}

impl std::error::Error for ManagedApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Migration(error) | Self::Statement(error) | Self::CommitUnconfirmed(error) => {
                Some(error)
            }
            Self::LedgerShape
            | Self::Capacity
            | Self::IdentityMismatch { .. }
            | Self::PublicExecuteRetained { .. } => None,
        }
    }
}

impl ManagedApplyError {
    /// Return the retained native failure, when the variant has one.
    pub fn native(&self) -> Option<&SqlxFailure> {
        match self {
            Self::Migration(error) | Self::Statement(error) | Self::CommitUnconfirmed(error) => {
                Some(error)
            }
            Self::LedgerShape
            | Self::Capacity
            | Self::IdentityMismatch { .. }
            | Self::PublicExecuteRetained { .. } => None,
        }
    }
}

fn statement(error: sqlx::Error) -> ManagedApplyError {
    ManagedApplyError::Statement(error.into())
}

impl From<CatalogError> for ManagedApplyError {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Native(error) => statement(error),
            CatalogError::Capacity => Self::Capacity,
        }
    }
}

pub(crate) async fn migrate_and_apply(
    mut lease: PgLease,
    migrator: &sqlx::migrate::Migrator,
    manifest: &ManagedObjectManifest,
) -> Result<AppliedManagedObjects, ManagedApplyError> {
    // One session lock spans both phases, so a concurrent apply of the same
    // ledger cannot land between this connection's migrations and objects.
    // Every error below returns while it is held; dropping the lease retires
    // the session, which releases the lock with it.
    let key = manifest.lock_key();
    let connection = lease.connection_mut();
    sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
        .bind(key)
        .execute(&mut *connection)
        .await
        .map_err(|error| ManagedApplyError::Migration(error.into()))?;
    migrator
        .run_direct(None, &mut *connection, false)
        .await
        .map_err(|error| ManagedApplyError::Migration(sqlx::Error::from(error).into()))?;
    let mut transaction = connection.begin().await.map_err(statement)?;
    let applied = apply(&mut transaction, manifest).await?;
    transaction
        .commit()
        .await
        .map_err(|error| ManagedApplyError::CommitUnconfirmed(error.into()))?;
    // The objects are committed: a failed unlock retires the session rather
    // than turning a committed apply into an error.
    let unlocked = sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(key)
        .fetch_one(lease.connection_mut())
        .await;
    lease.finish_query(matches!(unlocked, Ok(true))).await;
    Ok(applied)
}

async fn execute(connection: &mut PgConnection, sql: impl SqlSafeStr) -> Result<(), sqlx::Error> {
    sqlx::query(sql)
        .persistent(false)
        .execute(connection)
        .await
        .map(|_| ())
}

async fn apply(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
) -> Result<AppliedManagedObjects, ManagedApplyError> {
    // Validate SQL-language bodies against the migrated catalog regardless of
    // the session default.
    execute(connection, "SET LOCAL check_function_bodies = on")
        .await
        .map_err(statement)?;
    // Library statements and rendering run under the canonical settings;
    // `create` restores the session's own values around each source.
    let settings = RenderSettings::apply(connection).await.map_err(statement)?;
    let rows = lock_ledger(connection, manifest).await?;
    let recorded = rows
        .iter()
        .map(|row| (row.identity.as_str(), row))
        .collect::<HashMap<_, _>>();
    let live = catalog::resolve_live(connection, manifest).await?;
    let hashes = catalog::render_hashes(connection, live_kinds(&live)).await?;
    let pending = manifest
        .objects()
        .iter()
        .filter(|object| {
            let installed = recorded.get(object.identity()).is_some_and(|row| {
                row.kind == object.kind().as_str()
                    && row.source_checksum == object.checksum()
                    && live.get(object.identity()).is_some_and(|live| {
                        live.kind == Some(object.kind())
                            && hashes
                                .get(&live.oid)
                                .is_some_and(|hash| row.rendered_checksum == hash)
                    })
            });
            !installed
        })
        .collect::<Vec<_>>();

    let retired = retire(connection, manifest, &live).await?;
    for kind_is_trigger in [false, true] {
        for object in pending
            .iter()
            .filter(|object| (object.kind() == ManagedObjectKind::Trigger) == kind_is_trigger)
        {
            create(connection, &settings, object).await?;
        }
    }

    let live = catalog::resolve_live(connection, manifest).await?;
    let hashes = catalog::render_hashes(connection, live_kinds(&live)).await?;
    let mut installed = Vec::with_capacity(manifest.objects().len());
    for object in manifest.objects() {
        let current = live
            .get(object.identity())
            .filter(|live| live.kind == Some(object.kind()))
            .and_then(|live| hashes.get(&live.oid).map(|hash| (live.oid, *hash)));
        let Some((oid, hash)) = current else {
            return Err(ManagedApplyError::IdentityMismatch {
                identity: object.identity().to_owned(),
            });
        };
        installed.push((object, oid, hash));
    }
    if manifest.public_execute() == PublicExecute::Revoke {
        revoke_public_execute(connection, &installed).await?;
    }
    record(connection, manifest, &pending, &installed).await?;
    Ok(AppliedManagedObjects {
        digest: manifest.digest(),
        created_or_replaced: pending.len(),
        unchanged: manifest.objects().len() - pending.len(),
        retired,
    })
}

fn live_kinds(
    live: &HashMap<String, LiveObject>,
) -> impl Iterator<Item = (i64, ManagedObjectKind)> + '_ {
    live.values()
        .filter_map(|object| object.kind.map(|kind| (object.oid, kind)))
}

/// Create the ledger when absent, lock it against concurrent appliers and
/// verifiers, and return its rows after checking its exact shape.
async fn lock_ledger(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
) -> Result<Vec<catalog::LedgerRow>, ManagedApplyError> {
    let ledger = manifest.ledger();
    match catalog::ledger_relation(connection, ledger)
        .await
        .map_err(statement)?
    {
        None => execute(connection, AssertSqlSafe(create_ledger_sql(ledger)))
            .await
            .map_err(statement)?,
        Some((_, kind)) if kind != "r" => return Err(ManagedApplyError::LedgerShape),
        Some(_) => {}
    }
    // ACCESS EXCLUSIVE conflicts with the verifier's pre-snapshot ACCESS SHARE,
    // so no verification snapshot can interleave with this transaction.
    execute(
        connection,
        AssertSqlSafe(format!(
            "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
            ledger.quoted()
        )),
    )
    .await
    .map_err(statement)?;
    let Some((oid, _)) = catalog::ledger_relation(connection, ledger)
        .await
        .map_err(statement)?
    else {
        return Err(ManagedApplyError::LedgerShape);
    };
    if catalog::inspect_ledger_shape(connection, oid)
        .await
        .map_err(statement)?
        != LedgerShape::Exact
    {
        return Err(ManagedApplyError::LedgerShape);
    }
    Ok(catalog::load_ledger_rows(connection, ledger).await?)
}

/// Drop every retired object that still exists: triggers before the routines
/// they may bind. `RESTRICT` semantics keep undeclared dependents fatal.
async fn retire(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
    live: &HashMap<String, LiveObject>,
) -> Result<usize, ManagedApplyError> {
    let mut retired = 0;
    for kind_is_trigger in [true, false] {
        // An object of another kind under the same identity (a procedure for
        // a retired function, or an aggregate) is not the retired object.
        for retirement in manifest.retirements().iter().filter(|retirement| {
            (retirement.kind() == ManagedObjectKind::Trigger) == kind_is_trigger
                && live
                    .get(retirement.identity())
                    .is_some_and(|live| live.kind == Some(retirement.kind()))
        }) {
            let sql = match retirement.structural_identity() {
                Identity::Trigger { table, name } => {
                    format!("DROP TRIGGER {} ON {}", name.quoted(), table.quoted())
                }
                Identity::Routine(signature) => format!(
                    "DROP {} {}",
                    routine_keyword(retirement.kind()),
                    signature.quoted()
                ),
            };
            execute(connection, AssertSqlSafe(sql))
                .await
                .map_err(statement)?;
            retired += 1;
        }
    }
    Ok(retired)
}

const fn routine_keyword(kind: ManagedObjectKind) -> &'static str {
    match kind {
        ManagedObjectKind::Procedure => "PROCEDURE",
        ManagedObjectKind::Function | ManagedObjectKind::Trigger => "FUNCTION",
    }
}

/// Execute one source statement and require that it wrote exactly its
/// declared identity, of its declared kind, in this transaction.
async fn create(
    connection: &mut PgConnection,
    settings: &RenderSettings,
    object: &ManagedObject,
) -> Result<(), ManagedApplyError> {
    settings.restore(connection).await.map_err(statement)?;
    execute(connection, AssertSqlSafe(object.source()))
        .await
        .map_err(statement)?;
    settings.reapply(connection).await.map_err(statement)?;
    let written = match object.structural_identity() {
        Identity::Routine(signature) => {
            catalog::resolve_routines(
                connection,
                &[(signature.schema(), signature.name())],
                Touched::Report,
            )
            .await?
        }
        Identity::Trigger { table, name } => {
            catalog::resolve_triggers(
                connection,
                &[(table.schema(), table.name(), name.as_str())],
                Touched::Report,
            )
            .await?
        }
    };
    let exact = written
        .get(object.identity())
        .is_some_and(|live| live.touched && live.kind == Some(object.kind()));
    if exact {
        Ok(())
    } else {
        Err(ManagedApplyError::IdentityMismatch {
            identity: object.identity().to_owned(),
        })
    }
}

/// Revoke `PUBLIC` `EXECUTE` where `PUBLIC` holds it, then require that it no
/// longer does: a revoke by a role that did not grant the privilege succeeds
/// with only a warning.
async fn revoke_public_execute(
    connection: &mut PgConnection,
    installed: &[(&ManagedObject, i64, [u8; 32])],
) -> Result<(), ManagedApplyError> {
    let routines = installed
        .iter()
        .filter(|(object, _, _)| object.kind() != ManagedObjectKind::Trigger)
        .map(|(_, oid, _)| *oid)
        .collect::<Vec<_>>();
    let holders = catalog::public_execute_holders(connection, &routines)
        .await
        .map_err(statement)?;
    for (object, _, _) in installed.iter().filter(|(_, oid, _)| holders.contains(oid)) {
        let Identity::Routine(signature) = object.structural_identity() else {
            continue;
        };
        let sql = format!(
            "REVOKE EXECUTE ON {} {} FROM PUBLIC",
            routine_keyword(object.kind()),
            signature.quoted()
        );
        execute(connection, AssertSqlSafe(sql))
            .await
            .map_err(statement)?;
    }
    let retained = catalog::public_execute_holders(connection, &routines)
        .await
        .map_err(statement)?;
    match installed.iter().find(|(_, oid, _)| retained.contains(oid)) {
        Some((object, _, _)) => Err(ManagedApplyError::PublicExecuteRetained {
            identity: object.identity().to_owned(),
        }),
        None => Ok(()),
    }
}

/// Replace ledger rows: drop undeclared rows, upsert re-executed entries and
/// record the set row. Rows of unchanged entries keep their original time.
async fn record(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
    pending: &[&ManagedObject],
    installed: &[(&ManagedObject, i64, [u8; 32])],
) -> Result<(), ManagedApplyError> {
    let ledger = manifest.ledger().quoted();
    let declared = manifest
        .objects()
        .iter()
        .map(|object| object.identity())
        .chain([SET_ROW])
        .collect::<Vec<_>>();
    sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM ONLY {ledger} WHERE identity <> ALL($1::text[])"
    )))
    .persistent(false)
    .bind(declared)
    .execute(&mut *connection)
    .await
    .map_err(statement)?;

    let rendered = installed
        .iter()
        .map(|(object, _, hash)| (object.identity(), hash))
        .collect::<HashMap<_, _>>();
    let mut identities = Vec::with_capacity(pending.len() + 1);
    let mut kinds = Vec::with_capacity(pending.len() + 1);
    let mut sources = Vec::with_capacity(pending.len() + 1);
    let mut hashes = Vec::with_capacity(pending.len() + 1);
    for object in pending {
        identities.push(object.identity());
        kinds.push(object.kind().as_str());
        sources.push(object.checksum().to_vec());
        hashes.push(rendered[object.identity()].to_vec());
    }
    identities.push(SET_ROW);
    kinds.push(SET_ROW);
    sources.push(manifest.digest().as_bytes().to_vec());
    hashes.push(
        catalog::rendered_set_digest(
            installed
                .iter()
                .map(|(object, _, hash)| (object.identity(), hash.as_slice())),
        )
        .to_vec(),
    );
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {ledger} AS ledger
             (identity, kind, source_checksum, rendered_checksum, applied_at)
         SELECT row.identity, row.kind, row.source_checksum, row.rendered_checksum,
                pg_catalog.now()
         FROM unnest($1::text[], $2::text[], $3::bytea[], $4::bytea[])
              AS row(identity, kind, source_checksum, rendered_checksum)
         ON CONFLICT (identity) DO UPDATE SET
             kind = EXCLUDED.kind,
             source_checksum = EXCLUDED.source_checksum,
             rendered_checksum = EXCLUDED.rendered_checksum,
             applied_at = EXCLUDED.applied_at"
    )))
    .persistent(false)
    .bind(identities)
    .bind(kinds)
    .bind(sources)
    .bind(hashes)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(statement)
}
