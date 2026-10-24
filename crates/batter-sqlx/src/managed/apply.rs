//! Library-ordered migrate-then-apply on one leased connection.

mod drops;

#[cfg(test)]
pub(super) use drops::{drop_statement, drops_before_create};

use super::catalog::{
    self, CatalogError, LedgerShape, LiveObject, RenderSettings, Touched, create_ledger_sql,
};
use super::{
    Identity, ManagedObject, ManagedObjectDigest, ManagedObjectKind, ManagedObjectManifest,
    PublicExecute, SET_ROW,
};
use crate::{PgLease, SqlxFailure};
use drops::{Retired, drop_holder, lock_trigger_tables, require_covered, retire};
use sqlx::{AssertSqlSafe, Connection, PgConnection, SqlSafeStr};
use std::{collections::HashMap, fmt};

/// Evidence that a committed apply installed one manifest.
///
/// The counts describe what this apply executed. An entry is re-executed only
/// when its ledger row is absent, its kind or source checksum changed, or its
/// live catalog definition no longer renders to the hash apply last recorded,
/// so a repeated apply of an unchanged, undrifted set executes no DDL. A
/// constraint trigger dropped and re-created from its source, like a trigger
/// whose name the other trigger kind held, counts as created or replaced, not
/// as retired.
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
    /// The ledger or a catalog read exceeded its bounded capacity, or the
    /// partition trees of the trigger tables apply locks kept changing: other
    /// sessions attached, detached, created or dropped partitions in each of
    /// apply's bounded lock rounds. Nothing was changed.
    Capacity,
    /// A statement did not act on exactly its declared identity and kind: a
    /// source statement did not create or replace it, for example because its
    /// argument list declares other types than the entry's signature, or a
    /// trigger drop reached another relation than the one apply read, because
    /// another session replaced the trigger's view or foreign table by a
    /// relation of the same name. Apply also reports a trigger here when
    /// another session replaced it while apply ran, so that the catalog read
    /// after a retirement needs a lock on it that apply did not take; apply
    /// refuses that work instead of requesting the lock. Everything the
    /// transaction did is rolled back.
    IdentityMismatch {
        /// The canonical identity of the entry or retirement concerned.
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
    /// or may not be installed. Read-only verification inspects the current
    /// installed state; it does not acknowledge termination of a retired server
    /// session or authorize replay.
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
    // Every statement must see what committed before it ran, whatever the
    // session default: a trigger's kind is read again after its table is
    // locked, and under REPEATABLE READ that read would return the
    // transaction's first snapshot while `DROP TRIGGER` finds a newer trigger.
    execute(connection, "SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .await
        .map_err(statement)?;
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
    let (pending, retired) = execute_changes(connection, &settings, manifest, &recorded).await?;
    let installed = installed_objects(connection, manifest).await?;
    if manifest.public_execute() == PublicExecute::Revoke {
        revoke_public_execute(connection, &installed).await?;
    }
    record(connection, manifest, &pending, &installed).await?;
    Ok(AppliedManagedObjects {
        digest: manifest.digest(),
        created_or_replaced: pending.len(),
        unchanged: manifest.objects().len() - pending.len(),
        retired: retired.triggers + retired.routines,
    })
}

/// Retire, drop and execute what this apply changes, and return the executed
/// entries and the retired counts.
async fn execute_changes<'m>(
    connection: &mut PgConnection,
    settings: &RenderSettings,
    manifest: &'m ManagedObjectManifest,
    recorded: &HashMap<&str, &catalog::LedgerRow>,
) -> Result<(Vec<&'m ManagedObject>, Retired), ManagedApplyError> {
    let live = catalog::resolve_live(connection, manifest).await?;
    let hashes = catalog::render_hashes(connection, live_kinds(&live)).await?;
    let pending = pending_entries(manifest, recorded, &live, &hashes);
    let locks = lock_trigger_tables(connection, manifest, &live, &pending).await?;
    let retired = retire(connection, manifest, &live).await?;
    // Dropping a partitioned table's trigger also drops its clones on the
    // partitions, and a clone can hold a declared identity, so the pending
    // set is decided again from a catalog read after any trigger retirement.
    // An object another session replaced since the first read has a new oid
    // and so counts as changed; `require_covered` refuses work that needs a
    // lock the set does not hold.
    let (live, pending) = if retired.triggers > 0 {
        let live = catalog::resolve_live(connection, manifest).await?;
        let pending = pending_entries(manifest, recorded, &live, &hashes);
        require_covered(connection, &locks, &pending, &live).await?;
        (live, pending)
    } else {
        (live, pending)
    };

    // Every drop runs before the weaker table locks taken below, by a routine
    // body's validation and by `CREATE OR REPLACE TRIGGER`; taking
    // `ACCESS EXCLUSIVE` after them on the same table, or a partition of it,
    // would upgrade a lock apply holds, which a concurrent reader that turns
    // writer can deadlock with. `live` only proposes a drop; `drop_holder`
    // decides it under the table's lock, which `lock_trigger_tables` took.
    for object in &pending {
        let holder = live.get(object.identity()).and_then(|live| live.kind);
        drop_holder(connection, object, holder).await?;
    }
    // Routines first, so a trigger of either kind binds the body this
    // transaction installs.
    for trigger_phase in [false, true] {
        for object in pending
            .iter()
            .filter(|object| object.kind().is_trigger() == trigger_phase)
        {
            create(connection, settings, object).await?;
        }
    }
    Ok((pending, retired))
}

/// Every declared object as installed now, with its oid and rendered hash;
/// an object absent or of another kind fails the apply.
async fn installed_objects<'m>(
    connection: &mut PgConnection,
    manifest: &'m ManagedObjectManifest,
) -> Result<Vec<(&'m ManagedObject, i64, [u8; 32])>, ManagedApplyError> {
    let live = catalog::resolve_live(connection, manifest).await?;
    let hashes = catalog::render_hashes(connection, live_kinds(&live)).await?;
    manifest
        .objects()
        .iter()
        .map(|object| {
            live.get(object.identity())
                .filter(|live| live.kind == Some(object.kind()))
                .and_then(|live| hashes.get(&live.oid).map(|hash| (object, live.oid, *hash)))
                .ok_or_else(|| ManagedApplyError::IdentityMismatch {
                    identity: object.identity().to_owned(),
                })
        })
        .collect()
}

/// The entries whose ledger row is absent, whose kind or source checksum
/// changed, or whose live definition no longer renders to the recorded hash.
fn pending_entries<'m>(
    manifest: &'m ManagedObjectManifest,
    recorded: &HashMap<&str, &catalog::LedgerRow>,
    live: &HashMap<String, LiveObject>,
    hashes: &HashMap<i64, [u8; 32]>,
) -> Vec<&'m ManagedObject> {
    manifest
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
        .collect()
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
    match catalog::named_relation(connection, ledger)
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
    let Some((oid, _)) = catalog::named_relation(connection, ledger)
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

/// Execute one source statement and require that it wrote exactly its
/// declared identity, of its declared kind, in this transaction.
///
/// A trigger that PostgreSQL cannot replace in place was dropped by
/// [`drop_holder`] earlier in this transaction, which holds `ACCESS EXCLUSIVE`
/// on its table until commit, so no other session reads or writes the table
/// while the trigger is absent or observes the new trigger bound to an
/// earlier routine body.
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
        .filter(|(object, _, _)| object.kind().is_routine())
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
            object.kind().sql_keyword(),
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
