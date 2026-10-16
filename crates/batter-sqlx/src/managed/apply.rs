//! Library-ordered migrate-then-apply on one leased connection.

use super::catalog::{
    self, CatalogError, LedgerShape, LiveObject, RenderSettings, Touched, create_ledger_sql,
};
use super::{
    Identity, ManagedObject, ManagedObjectDigest, ManagedObjectKind, ManagedObjectManifest,
    PublicExecute, SET_ROW,
};
use crate::verification::{Identifier, QualifiedName};
use crate::{PgLease, SqlxFailure};
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
    /// The ledger or a catalog read exceeded its bounded capacity.
    Capacity,
    /// A statement did not act on exactly its declared identity and kind: a
    /// source statement did not create or replace it, for example because its
    /// argument list declares other types than the entry's signature, or a
    /// trigger drop reached another relation than the one apply read, because
    /// another session replaced the trigger's view or foreign table by a
    /// relation of the same name. The drop is rolled back with the rest of
    /// the transaction.
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
    let live = catalog::resolve_live(connection, manifest).await?;
    let hashes = catalog::render_hashes(connection, live_kinds(&live)).await?;
    let retired = retire(connection, manifest, &live).await?;
    // Dropping a partitioned table's trigger also drops its clones on the
    // partitions, and a clone can hold a declared identity, so the pending
    // set is decided from a catalog read after any trigger retirement.
    // Retirements only drop, so the surviving objects keep the oids and
    // renderings hashed above.
    let live = if retired.triggers > 0 {
        catalog::resolve_live(connection, manifest).await?
    } else {
        live
    };
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

    // Every `ACCESS EXCLUSIVE` lock apply takes on a trigger's table comes
    // before the weaker table locks taken below, by a routine body's
    // validation and by `CREATE OR REPLACE TRIGGER`. Taking one after them on
    // the same table, or a partition of it, would upgrade a lock apply holds,
    // which a concurrent reader that turns writer can deadlock with. `live`
    // only proposes a drop; `drop_holder` decides it under the table's lock.
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
        retired: retired.triggers + retired.routines,
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

/// How many retired objects `retire` dropped, by phase.
struct Retired {
    triggers: usize,
    routines: usize,
}

/// Drop every retired object that still exists as its kind: triggers of both
/// kinds before the routines they may bind. `RESTRICT` semantics keep
/// undeclared dependents fatal; only a trigger's own partition clones go with
/// it. `DROP FUNCTION` and `DROP PROCEDURE` refuse the other routine kind
/// themselves, but `DROP TRIGGER` removes either trigger kind by name, so a
/// trigger is dropped only when the trigger read under its table's lock is
/// of the retired kind.
async fn retire(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
    live: &HashMap<String, LiveObject>,
) -> Result<Retired, ManagedApplyError> {
    let mut retired = Retired {
        triggers: 0,
        routines: 0,
    };
    for trigger_phase in [true, false] {
        // An object of another kind under the same identity (a procedure for
        // a retired function, an aggregate, or the other trigger kind) is not
        // the retired object.
        for retirement in manifest.retirements().iter().filter(|retirement| {
            retirement.kind().is_trigger() == trigger_phase
                && live
                    .get(retirement.identity())
                    .is_some_and(|live| live.kind == Some(retirement.kind()))
        }) {
            if let Identity::Trigger { table, name } = retirement.structural_identity() {
                let Some(holder) = locked_trigger(connection, table, name).await? else {
                    continue;
                };
                if holder.kind != Some(retirement.kind()) {
                    continue;
                }
                drop_trigger(
                    connection,
                    retirement.structural_identity(),
                    retirement.identity(),
                    holder,
                )
                .await?;
            } else {
                let sql = drop_statement(retirement.kind(), retirement.structural_identity());
                execute(connection, AssertSqlSafe(sql))
                    .await
                    .map_err(statement)?;
            }
            if trigger_phase {
                retired.triggers += 1;
            } else {
                retired.routines += 1;
            }
        }
    }
    Ok(retired)
}

/// `DROP` of `identity` as an existing object of `kind`, without `CASCADE`.
pub(super) fn drop_statement(kind: ManagedObjectKind, identity: &Identity) -> String {
    let target = match identity {
        Identity::Trigger { table, name } => format!("{} ON {}", name.quoted(), table.quoted()),
        Identity::Routine(signature) => signature.quoted().to_owned(),
    };
    format!("DROP {} {target}", kind.sql_keyword())
}

/// Whether a pending entry must first drop the live object holding its
/// identity: PostgreSQL replaces neither a constraint trigger in place nor one
/// trigger kind with the other, while `CREATE OR REPLACE` keeps routines and
/// plain triggers in place.
pub(super) const fn drops_before_create(
    declared: ManagedObjectKind,
    holder: ManagedObjectKind,
) -> bool {
    use ManagedObjectKind::{ConstraintTrigger, Function, Procedure, Trigger};
    match (declared, holder) {
        (Function | Procedure, _) | (Trigger, Trigger) => false,
        (Trigger | ConstraintTrigger, ConstraintTrigger) | (ConstraintTrigger, Trigger) => true,
        // Routine and trigger identities never collide.
        (Trigger | ConstraintTrigger, Function | Procedure) => false,
    }
}

/// The relation a trigger drop is decided on, and the kind of the trigger
/// holding the name there.
#[derive(Clone, Copy)]
struct Holder {
    relation: i64,
    kind: Option<ManagedObjectKind>,
}

/// Lock a trigger's table in `ACCESS EXCLUSIVE` mode, the lock `DROP TRIGGER`
/// needs anyway, then read the kind of the trigger that holds `name` on it now.
/// `None` means no relation holds the table's name.
///
/// `DROP TRIGGER` removes either kind by name, so a kind read before the lock
/// could describe a trigger another session has since replaced. Under the
/// lock no other session can create or drop a trigger on the table, and the
/// READ COMMITTED read sees every replacement committed before it. `ONLY`
/// keeps inheritance children and partitions out of this lock; dropping a
/// partitioned table's trigger then locks its partitions as well. Views and
/// foreign tables cannot hold constraint triggers, and `LOCK TABLE` would
/// also lock a view's base relations or refuse a foreign table, so their
/// trigger is read without the lock; [`drop_trigger`] then rejects a drop that
/// reached a relation replaced under the same name.
async fn locked_trigger(
    connection: &mut PgConnection,
    table: &QualifiedName,
    name: &Identifier,
) -> Result<Option<Holder>, ManagedApplyError> {
    let Some((relation, relkind)) = catalog::named_relation(connection, table)
        .await
        .map_err(statement)?
    else {
        return Ok(None);
    };
    if relkind == "r" || relkind == "p" {
        execute(
            connection,
            AssertSqlSafe(format!(
                "LOCK TABLE ONLY {} IN ACCESS EXCLUSIVE MODE",
                table.quoted()
            )),
        )
        .await
        .map_err(statement)?;
    }
    let current = catalog::resolve_triggers(
        connection,
        &[(table.schema(), table.name(), name.as_str())],
        Touched::Ignore,
    )
    .await?;
    Ok(Some(Holder {
        relation,
        kind: current.into_values().next().and_then(|live| live.kind),
    }))
}

/// Drop the trigger that `holder` describes under `identity` (rendered as
/// `rendered`), and require that the drop reached the relation the decision
/// was read from.
///
/// `DROP TRIGGER` resolves the table by name and keeps it locked until
/// commit, so after the drop the name resolves to the relation it acted on.
/// Another session can replace a view or foreign table, which apply reads
/// without locking, by an ordinary table of the same name that holds a
/// constraint trigger; that drop must not stand, so apply fails with
/// [`ManagedApplyError::IdentityMismatch`] and the transaction rolls back.
async fn drop_trigger(
    connection: &mut PgConnection,
    identity: &Identity,
    rendered: &str,
    holder: Holder,
) -> Result<(), ManagedApplyError> {
    let (Identity::Trigger { table, .. }, Some(kind)) = (identity, holder.kind) else {
        return Ok(());
    };
    execute(connection, AssertSqlSafe(drop_statement(kind, identity)))
        .await
        .map_err(statement)?;
    let dropped_on = catalog::named_relation(connection, table)
        .await
        .map_err(statement)?;
    if dropped_on.is_some_and(|(relation, _)| relation == holder.relation) {
        Ok(())
    } else {
        Err(ManagedApplyError::IdentityMismatch {
            identity: rendered.to_owned(),
        })
    }
}

/// Drop the trigger holding a pending entry's name when PostgreSQL cannot
/// replace it in place. `holder` is the kind that held the identity when
/// apply read the catalog; it only proposes the drop, which is decided from
/// the trigger read under the table's lock.
async fn drop_holder(
    connection: &mut PgConnection,
    object: &ManagedObject,
    holder: Option<ManagedObjectKind>,
) -> Result<(), ManagedApplyError> {
    let Identity::Trigger { table, name } = object.structural_identity() else {
        return Ok(());
    };
    if !holder.is_some_and(|holder| drops_before_create(object.kind(), holder)) {
        return Ok(());
    }
    match locked_trigger(connection, table, name).await? {
        Some(current)
            if current
                .kind
                .is_some_and(|holder| drops_before_create(object.kind(), holder)) =>
        {
            drop_trigger(
                connection,
                object.structural_identity(),
                object.identity(),
                current,
            )
            .await
        }
        _ => Ok(()),
    }
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
