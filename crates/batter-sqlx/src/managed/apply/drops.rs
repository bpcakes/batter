//! Trigger drops and retirements, decided under the table's lock.
//!
//! `DROP TRIGGER` removes either trigger kind by name, so every drop here is
//! decided from the trigger read again under the table's `ACCESS EXCLUSIVE`
//! lock, must reach the relation that read named, and takes its lock before
//! any weaker lock apply takes, partitioned tables before their partitions.

use super::{execute, statement};
use crate::managed::catalog::{self, LiveObject, Touched};
use crate::managed::{
    Identity, ManagedApplyError, ManagedObject, ManagedObjectKind, ManagedObjectManifest,
    Retirement,
};
use crate::verification::{Identifier, QualifiedName};
use sqlx::{AssertSqlSafe, PgConnection};
use std::collections::{BTreeMap, HashMap};

/// Take every `ACCESS EXCLUSIVE` lock a trigger drop in this apply may need,
/// partitioned tables before their partitions, before any drop runs.
///
/// A write routed through a partitioned table locks it before the partition
/// it reaches, and so does `DROP TRIGGER` on a partitioned table. Locking a
/// partition first, for a drop whose identity sorts before one on its
/// partitioned table, could deadlock with such a write. The candidates are
/// the trigger retirements the catalog shows as present and the pending
/// entries whose holder must be dropped. Each takes the lock its drop takes
/// anyway: a table `ONLY`, and a partitioned table with its partitions, as
/// `DROP TRIGGER` on it locks them. With every such lock held first, no later
/// statement of the apply upgrades a lock it holds. Views and foreign tables
/// are not locked, as in [`locked_trigger`].
pub(super) async fn lock_drop_tables(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
    live: &HashMap<String, LiveObject>,
    pending: &[&ManagedObject],
) -> Result<(), ManagedApplyError> {
    let holder = |identity: &str| live.get(identity).and_then(|live| live.kind);
    let retired = manifest
        .retirements()
        .iter()
        .filter(|retirement| {
            retirement.kind().is_trigger()
                && holder(retirement.identity()) == Some(retirement.kind())
        })
        .map(Retirement::structural_identity);
    let replaced = pending
        .iter()
        .filter(|object| {
            holder(object.identity())
                .is_some_and(|holder| drops_before_create(object.kind(), holder))
        })
        .map(|object| object.structural_identity());
    let mut tables = BTreeMap::new();
    for identity in retired.chain(replaced) {
        if let Identity::Trigger { table, .. } = identity {
            tables.insert((table.schema(), table.name()), table);
        }
    }
    let keys = tables.keys().copied().collect::<Vec<_>>();
    let ordered = catalog::lockable_tables_in_lock_order(connection, &keys)
        .await
        .map_err(statement)?;
    let list = ordered
        .iter()
        .filter_map(|(schema, name, relkind)| {
            let table = tables.get(&(schema.as_str(), name.as_str()))?;
            let only = if relkind == "p" { "" } else { "ONLY " };
            Some(format!("{only}{}", table.quoted()))
        })
        .collect::<Vec<_>>();
    if list.is_empty() {
        return Ok(());
    }
    execute(
        connection,
        AssertSqlSafe(format!(
            "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
            list.join(", ")
        )),
    )
    .await
    .map_err(statement)
}

/// How many retired objects `retire` dropped, by phase.
pub(super) struct Retired {
    pub(super) triggers: usize,
    pub(super) routines: usize,
}

/// Drop every retired object that still exists as its kind: triggers of both
/// kinds before the routines they may bind. `RESTRICT` semantics keep
/// undeclared dependents fatal; only a trigger's own partition clones go with
/// it. `DROP FUNCTION` and `DROP PROCEDURE` refuse the other routine kind
/// themselves, but `DROP TRIGGER` removes either trigger kind by name, so a
/// trigger is dropped only when the trigger read under its table's lock is
/// of the retired kind.
pub(super) async fn retire(
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
pub(in crate::managed) fn drop_statement(kind: ManagedObjectKind, identity: &Identity) -> String {
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
pub(in crate::managed) const fn drops_before_create(
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
/// partitioned table's trigger then locks its partitions as well. The lock is
/// normally held already, taken in partition order by [`lock_drop_tables`].
/// Views and
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
pub(super) async fn drop_holder(
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
