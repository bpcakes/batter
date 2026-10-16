//! Trigger drops and retirements, decided under the table's lock.
//!
//! `DROP TRIGGER` removes either trigger kind by name, so every drop here is
//! decided from the trigger read again under the table's `ACCESS EXCLUSIVE`
//! lock and must reach the relation that read named. Every lock the trigger
//! statements need is taken first, one relation at a time, never waiting for
//! one while holding another.

use super::{execute, statement};
use crate::managed::catalog::{self, LiveObject, Touched};
use crate::managed::{
    Identity, ManagedApplyError, ManagedObject, ManagedObjectKind, ManagedObjectManifest,
};
use crate::verification::{Identifier, QualifiedName};
use sqlx::{AssertSqlSafe, PgConnection};
use std::collections::HashMap;

/// Rounds of taking the whole lock set before apply gives up with the last
/// `NOWAIT` refusal. Every round after the first first waits for the relation
/// that refused, so rounds repeat only while other transactions keep holding
/// a different relation of the set.
const LOCK_ROUNDS: usize = 32;

/// Take every lock apply's trigger statements need on application tables
/// before any of them runs, waiting for one relation at a time while holding
/// none of the others.
///
/// A trigger retirement or drop needs `ACCESS EXCLUSIVE`, and every other
/// pending trigger `SHARE ROW EXCLUSIVE`, on its table and, for a partitioned
/// table, on every partition of its tree, which a row trigger's statements
/// lock as well. Each relation gets its own `LOCK TABLE ONLY`: a recursive
/// `LOCK TABLE` would hold the partitioned table while it waits for a
/// partition. No fixed order is deadlock-free: a write routed through a
/// partitioned table locks it before the partition it reaches, while a
/// transaction that read a partition and then writes through its partitioned
/// table already holds the partition. Apply therefore takes the whole set
/// with `NOWAIT` inside a savepoint, partitioned tables before their
/// partitions. When a relation refuses, it rolls the savepoint back,
/// releasing everything the round took, and starts the next round by waiting
/// for that relation alone. It never waits while holding another relation of
/// the set, so it cannot complete a deadlock with such transactions; a
/// session `lock_timeout` still bounds each wait. The candidates are the
/// trigger retirements the catalog shows as present and every pending trigger
/// entry, exclusive when its holder must be dropped. Views and foreign tables
/// are not locked, as in [`locked_trigger`]. Nor are foreign partitions,
/// which `LOCK TABLE ONLY` refuses: a plain row trigger statement on their
/// partitioned table locks them later, while holding the rest of the set. A
/// constraint trigger cannot exist on a partition tree that has one.
pub(super) async fn lock_trigger_tables(
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
        .map(|retirement| (retirement.structural_identity(), true));
    let replaced = pending
        .iter()
        .filter(|object| object.kind().is_trigger())
        .map(|object| {
            let drops = holder(object.identity())
                .is_some_and(|holder| drops_before_create(object.kind(), holder));
            (object.structural_identity(), drops)
        });
    let requests = retired
        .chain(replaced)
        .filter_map(|(identity, exclusive)| match identity {
            Identity::Trigger { table, .. } => Some((table.schema(), table.name(), exclusive)),
            Identity::Routine(_) => None,
        })
        .collect::<Vec<_>>();
    let statements = catalog::lockable_tables_in_lock_order(connection, &requests)
        .await
        .map_err(statement)?
        .into_iter()
        .map(|(relation, exclusive)| {
            let mode = if exclusive {
                "ACCESS EXCLUSIVE"
            } else {
                "SHARE ROW EXCLUSIVE"
            };
            format!("LOCK TABLE ONLY {relation} IN {mode} MODE")
        })
        .collect::<Vec<_>>();
    take_all(connection, &statements).await
}

/// Run every `LOCK TABLE` in `statements` in one savepoint, with `NOWAIT`,
/// as described for [`lock_trigger_tables`].
///
/// The savepoint is always released, after a rollback to it when a relation
/// refused, so the apply's later statements run in its top-level transaction
/// and write rows whose `xmin` the identity check compares with it.
async fn take_all(
    connection: &mut PgConnection,
    statements: &[String],
) -> Result<(), ManagedApplyError> {
    if statements.is_empty() {
        return Ok(());
    }
    let mut busy: Option<usize> = None;
    let mut rounds = 0;
    loop {
        rounds += 1;
        execute(connection, "SAVEPOINT batter_managed_locks")
            .await
            .map_err(statement)?;
        if let Some(index) = busy {
            // Wait for the relation that refused, holding none of the others.
            execute(connection, AssertSqlSafe(statements[index].as_str()))
                .await
                .map_err(statement)?;
        }
        let mut refused = None;
        for (index, sql) in statements.iter().enumerate() {
            if Some(index) == busy {
                continue;
            }
            match execute(connection, AssertSqlSafe(format!("{sql} NOWAIT"))).await {
                Ok(()) => {}
                Err(error) if lock_not_available(&error) => {
                    refused = Some((index, error));
                    break;
                }
                Err(error) => return Err(statement(error)),
            }
        }
        let Some((index, error)) = refused else {
            return execute(connection, "RELEASE SAVEPOINT batter_managed_locks")
                .await
                .map_err(statement);
        };
        for sql in [
            "ROLLBACK TO SAVEPOINT batter_managed_locks",
            "RELEASE SAVEPOINT batter_managed_locks",
        ] {
            execute(connection, sql).await.map_err(statement)?;
        }
        if rounds == LOCK_ROUNDS {
            return Err(statement(error));
        }
        busy = Some(index);
    }
}

/// `LOCK TABLE ... NOWAIT` refused because another transaction holds a
/// conflicting lock (SQLSTATE `55P03`).
fn lock_not_available(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "55P03")
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
/// partitioned table's trigger then locks its partitions as well. Both are
/// normally held already, taken by [`lock_trigger_tables`]. Views and
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
