//! Privileges apply's table locks need beyond the trigger statements'. Every
//! `LOCK TABLE` needs `MAINTAIN`, `UPDATE`, `DELETE` or `TRUNCATE` on its
//! relation, while `CREATE TRIGGER` needs only `TRIGGER` and `DROP TRIGGER`
//! only ownership of the named table. Without such a privilege, apply fails
//! before any trigger statement runs and changes nothing. Batter grants
//! nothing: here the superuser stands in for the table owner's own grant.

use super::constraint_triggers::{DEFERRED, check_function, check_log};
use super::trigger_drops::with_sqlstate;
use super::trigger_lock_order::{parted_check_objects, partitions};
use super::{Fixture, Role, catalog_state, sqlstate};
use crate::support::{Result, require};
use batter_sqlx::managed::{
    AppliedManagedObjects, ManagedApplyError, ManagedObject, ManagedObjectManifest, PublicExecute,
};
use batter_sqlx::verification::{Identifier, QualifiedName};

/// Whether `outcome` is the lock step's refusal for a missing privilege.
fn lock_denied(outcome: &std::result::Result<AppliedManagedObjects, ManagedApplyError>) -> bool {
    matches!(outcome, Err(error @ ManagedApplyError::Statement(_))
        if sqlstate(error).as_deref() == Some("42501"))
}

/// `check_record` and, when `trigger` is set, a plain trigger on `delegated`.
fn delegated_objects(fixture: &Fixture, trigger: ObjectKind) -> Result<ManagedObjectManifest> {
    let trigger = trigger == ObjectKind::Trigger;
    let schema = fixture.schema();
    let mut objects = vec![check_function(schema, "v1")?];
    if trigger {
        objects.push(ManagedObject::trigger(
            QualifiedName::new(schema, "delegated")?,
            Identifier::new("delegated_audit")?,
            format!(
                "CREATE OR REPLACE TRIGGER delegated_audit AFTER INSERT ON {schema}.delegated
                 FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
            ),
        )?);
    }
    Ok(fixture
        .objects_with(PublicExecute::Revoke, objects, [])?
        .manifest)
}

/// The applying login may create triggers on a table another role owns
/// (`TRIGGER`), which `CREATE OR REPLACE TRIGGER` alone allows. Apply's
/// `SHARE ROW EXCLUSIVE` lock on that table is refused until the owner also
/// grants `MAINTAIN`.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_locks_need_a_lock_privilege_on_a_delegated_table() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let (migrator, delegate) = (fixture.migrator().to_owned(), fixture.delegate().to_owned());
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(format!(
            "CREATE TABLE {schema}.delegated (id bigint NOT NULL)"
        ));
        let applying = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let (base, declared) = (
            delegated_objects(fixture, ObjectKind::Routine)?,
            delegated_objects(fixture, ObjectKind::Trigger)?,
        );
        let _ = fixture
            .apply(&applying, &base)
            .await
            .map_err(with_sqlstate)?;
        fixture
            .admin(&format!(
                "ALTER TABLE {schema}.delegated OWNER TO {delegate};
                 GRANT TRIGGER ON {schema}.delegated TO {migrator};"
            ))
            .await?;
        let catalog = catalog_state(&observer, &schema).await?;
        require!(
            lock_denied(&fixture.apply(&applying, &declared).await),
            "apply without a lock privilege on the trigger's table did not fail with 42501",
        )?;
        require!(
            catalog_state(&observer, &schema).await? == catalog,
            "an apply refused its lock changed the catalog",
        )?;
        fixture
            .admin(&format!(
                "GRANT MAINTAIN ON {schema}.delegated TO {migrator}"
            ))
            .await?;
        let applied = fixture
            .apply(&applying, &declared)
            .await
            .map_err(with_sqlstate)?;
        require!(
            applied.created_or_replaced() == 1,
            "apply did not create the trigger once the owner granted MAINTAIN",
        )
    })
    .await
}

/// The applying login owns `parted`, so `DROP TRIGGER` on it may drop the
/// clone on `parted_east`, which another role owns. Apply's
/// `ACCESS EXCLUSIVE` lock on that partition is refused until its owner
/// grants `MAINTAIN`.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_locks_need_a_lock_privilege_on_a_separately_owned_partition() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let (migrator, delegate) = (fixture.migrator().to_owned(), fixture.delegate().to_owned());
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(partitions(&schema));
        let applying = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let (install, retire) = (
            parted_check_objects(fixture, Some(DEFERRED))?,
            parted_check_objects(fixture, None)?,
        );
        let _ = fixture
            .apply(&applying, &install)
            .await
            .map_err(with_sqlstate)?;
        fixture
            .admin(&format!(
                "ALTER TABLE {schema}.parted_east OWNER TO {delegate}"
            ))
            .await?;
        let catalog = catalog_state(&observer, &schema).await?;
        require!(
            lock_denied(&fixture.apply(&applying, &retire).await),
            "apply without a lock privilege on a partition did not fail with 42501",
        )?;
        require!(
            catalog_state(&observer, &schema).await? == catalog,
            "an apply refused its lock changed the catalog",
        )?;
        fixture
            .admin(&format!(
                "GRANT MAINTAIN ON {schema}.parted_east TO {migrator}"
            ))
            .await?;
        let applied = fixture
            .apply(&applying, &retire)
            .await
            .map_err(with_sqlstate)?;
        require!(
            applied.retired() == 1,
            "apply did not retire the trigger once the partition's owner granted MAINTAIN",
        )
    })
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObjectKind {
    Trigger,
    Routine,
}
