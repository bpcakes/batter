//! The order of apply's table locks. Every `ACCESS EXCLUSIVE` lock a trigger
//! drop needs precedes the weaker locks of trigger replacements and routine
//! sources, and partitioned tables are locked before their partitions, so a
//! concurrent reader turning writer, or a write routed through a partitioned
//! table, cannot deadlock with an apply that drops triggers.

use super::constraint_triggers::{DEFERRED, IMMEDIATE, check_function, check_log, check_trigger};
use super::trigger_drops::{RacePools, wait_for_any_lock_wait, with_sqlstate};
use super::verification::wait_for_lock;
use super::{Fixture, Role, catalog_state, pid, sqlstate};
use crate::support::{Result, bounded, require};
use batter_core::BoxError;
use batter_sqlx::managed::{
    ManagedApplyError, ManagedObject, ManagedObjectManifest, PublicExecute, Retirement,
};
use batter_sqlx::verification::{Identifier, QualifiedName};
use sqlx::AssertSqlSafe;
use std::time::Duration;
use tokio::time::timeout;

/// A partitioned table with two partitions.
fn partitions(schema: &str) -> String {
    format!(
        "CREATE TABLE {schema}.parted (id bigint NOT NULL, region text NOT NULL)
             PARTITION BY LIST (region);
         CREATE TABLE {schema}.parted_east PARTITION OF {schema}.parted FOR VALUES IN ('east');
         CREATE TABLE {schema}.parted_west PARTITION OF {schema}.parted FOR VALUES IN ('west');"
    )
}

/// Apply `change` while another transaction, which ran `before` before apply
/// started, runs `write` once apply waits for a lock on `table`, in `mode` or
/// any mode, then commits. PostgreSQL lets that write ahead of the queued
/// request unless apply already holds a lock it conflicts with, so both
/// transactions must commit, and apply must report `expected` created or
/// replaced and retired counts.
async fn lock_order_race(
    fixture: &mut Fixture,
    install: &ManagedObjectManifest,
    change: &ManagedObjectManifest,
    (table, mode): (&str, Option<&str>),
    statements: (String, String),
    expected: (usize, usize),
) -> Result {
    let (before, write) = statements;
    let pools = RacePools::new(fixture, &[]).await?;
    let _ = fixture
        .apply(&pools.applying, install)
        .await
        .map_err(with_sqlstate)?;
    let schema = fixture.schema();
    let apply_pid = pid(&pools.applying).await?;
    let mut reader = pools.blocker.begin().await?;
    bounded(sqlx::query(AssertSqlSafe(before)).execute(&mut *reader)).await??;
    let apply = async {
        fixture
            .apply(&pools.applying, change)
            .await
            .map_err(with_sqlstate)
    };
    let writer = async {
        match mode {
            Some(mode) => {
                wait_for_lock(&pools.observer, apply_pid, schema, table, mode, false).await?;
            }
            None => wait_for_any_lock_wait(&pools.observer, apply_pid, schema, table).await?,
        }
        let written = timeout(
            Duration::from_secs(10),
            sqlx::query(AssertSqlSafe(write)).execute(&mut *reader),
        )
        .await?;
        if let Err(error) = written {
            let code = error
                .as_database_error()
                .and_then(|error| error.code())
                .map(|code| code.into_owned());
            return Err(std::io::Error::other(format!(
                "the concurrent write failed with SQLSTATE {code:?}"
            ))
            .into());
        }
        reader.commit().await?;
        Ok::<_, BoxError>(())
    };
    let (applied, ()) = tokio::try_join!(apply, writer)?;
    require(
        (applied.created_or_replaced(), applied.retired()) == expected,
        "apply did not execute exactly the changed entries and retirements",
    )
}

/// A plain trigger and a constraint trigger, both changed, on one table, or
/// on a partitioned table and one of its partitions.
fn lock_order_objects(
    fixture: &Fixture,
    plain: (&str, &str),
    constraint: (&str, &str),
    events: &str,
    deferral: &str,
) -> Result<ManagedObjectManifest> {
    let schema = fixture.schema();
    let (plain_table, plain_name) = plain;
    let (constraint_table, constraint_name) = constraint;
    Ok(fixture
        .objects_with(
            PublicExecute::Revoke,
            [
                check_function(schema, "v1")?,
                ManagedObject::trigger(
                    QualifiedName::new(schema, plain_table)?,
                    Identifier::new(plain_name)?,
                    format!(
                        "CREATE OR REPLACE TRIGGER {plain_name} AFTER {events} ON {schema}.{plain_table}
                         FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
                    ),
                )?,
                ManagedObject::constraint_trigger(
                    QualifiedName::new(schema, constraint_table)?,
                    Identifier::new(constraint_name)?,
                    format!(
                        "CREATE CONSTRAINT TRIGGER {constraint_name} AFTER INSERT
                         ON {schema}.{constraint_table}
                         {deferral} FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
                    ),
                )?,
            ],
            [],
        )?
        .manifest)
}

/// The plain trigger sorts first, so a drop taken in identity order would
/// follow its `SHARE ROW EXCLUSIVE` lock on the same table.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_drops_precede_replacements_on_one_table() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let (plain, constraint) = (("records", "records_audit"), ("records", "records_check"));
        let install = lock_order_objects(fixture, plain, constraint, "INSERT", DEFERRED)?;
        let change = lock_order_objects(fixture, plain, constraint, "INSERT OR UPDATE", IMMEDIATE)?;
        lock_order_race(
            fixture,
            &install,
            &change,
            ("records", Some("AccessExclusiveLock")),
            (
                format!("SELECT count(*) FROM {schema}.records"),
                format!("INSERT INTO {schema}.records (note) VALUES ('raced')"),
            ),
            (2, 0),
        )
        .await
    })
    .await
}

/// The plain trigger on a partitioned table sorts first and locks its
/// partitions too, so a drop taken in identity order on one of them would
/// follow that `SHARE ROW EXCLUSIVE` lock.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_drops_precede_replacements_across_a_partition() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(partitions(&schema));
        let (plain, constraint) = (("parted", "parted_audit"), ("parted_east", "parted_check"));
        let install = lock_order_objects(fixture, plain, constraint, "INSERT", DEFERRED)?;
        let change = lock_order_objects(fixture, plain, constraint, "INSERT OR UPDATE", IMMEDIATE)?;
        lock_order_race(
            fixture,
            &install,
            &change,
            ("parted_east", Some("AccessExclusiveLock")),
            (
                format!("SELECT count(*) FROM {schema}.parted_east"),
                format!("INSERT INTO {schema}.parted (id, region) VALUES (1, 'east')"),
            ),
            (2, 0),
        )
        .await
    })
    .await
}

/// A constraint trigger on `parted` and, unless retired, one on its partition
/// `parted_east` whose name sorts first, so drops in identity order would
/// lock the partition before its partitioned table.
fn partition_order_objects(
    fixture: &Fixture,
    child: Option<&str>,
    parent_deferral: &str,
) -> Result<ManagedObjectManifest> {
    let schema = fixture.schema();
    let constraint = |table: &str, name: &str, deferral: &str| {
        ManagedObject::constraint_trigger(
            QualifiedName::new(schema, table)?,
            Identifier::new(name)?,
            format!(
                "CREATE CONSTRAINT TRIGGER {name} AFTER INSERT ON {schema}.{table}
                 {deferral} FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
            ),
        )
        .map_err(BoxError::from)
    };
    let mut objects = vec![
        check_function(schema, "v1")?,
        constraint("parted", "b_parent_check", parent_deferral)?,
    ];
    let mut retirements = Vec::new();
    match child {
        Some(deferral) => objects.push(constraint("parted_east", "a_child_check", deferral)?),
        None => retirements.push(Retirement::constraint_trigger(
            QualifiedName::new(schema, "parted_east")?,
            Identifier::new("a_child_check")?,
        )),
    }
    Ok(fixture
        .objects_with(PublicExecute::Revoke, objects, retirements)?
        .manifest)
}

/// Writes routed through `parted`: one to the other partition before apply
/// starts, one to `parted_east` once apply waits for `parted`.
fn routed_writes(schema: &str) -> (String, String) {
    (
        format!("INSERT INTO {schema}.parted (id, region) VALUES (1, 'west')"),
        format!("INSERT INTO {schema}.parted (id, region) VALUES (2, 'east')"),
    )
}

/// Both constraint triggers change, so both must be dropped and re-created.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_drops_lock_partitioned_tables_before_their_partitions() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(partitions(&schema));
        let install = partition_order_objects(fixture, Some(DEFERRED), DEFERRED)?;
        let change = partition_order_objects(fixture, Some(IMMEDIATE), IMMEDIATE)?;
        lock_order_race(
            fixture,
            &install,
            &change,
            ("parted", Some("AccessExclusiveLock")),
            routed_writes(&schema),
            (2, 0),
        )
        .await
    })
    .await
}

/// The partition's constraint trigger is retired, which runs before the
/// partitioned table's changed constraint trigger is dropped.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_retirements_lock_partitioned_tables_before_their_partitions() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(partitions(&schema));
        let install = partition_order_objects(fixture, Some(DEFERRED), DEFERRED)?;
        let change = partition_order_objects(fixture, None, IMMEDIATE)?;
        lock_order_race(
            fixture,
            &install,
            &change,
            ("parted", Some("AccessExclusiveLock")),
            routed_writes(&schema),
            (1, 1),
        )
        .await
    })
    .await
}

/// A plain trigger on `parted` changes and a constraint trigger on its
/// partition `parted_east` must be dropped, while a write routed through
/// `parted` holds it before apply starts and reaches `parted_east` once apply
/// waits for `parted`. With `managed_trigger_drops_precede_replacements_across_a_partition`,
/// whose transaction holds the partition first, no fixed lock order serves
/// both, so apply never waits while holding part of its locks.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_mixed_partition_trigger_changes_let_a_routed_writer_finish() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(partitions(&schema));
        let (plain, constraint) = (("parted", "parted_audit"), ("parted_east", "parted_check"));
        let install = lock_order_objects(fixture, plain, constraint, "INSERT", DEFERRED)?;
        let change = lock_order_objects(fixture, plain, constraint, "INSERT OR UPDATE", IMMEDIATE)?;
        lock_order_race(
            fixture,
            &install,
            &change,
            ("parted", None),
            routed_writes(&schema),
            (2, 0),
        )
        .await
    })
    .await
}

/// Waiting for a refused table is an ordinary lock wait: a session
/// `lock_timeout` bounds it, and the apply then fails before `COMMIT` without
/// changing anything.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_lock_wait_honours_the_session_lock_timeout() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let declare = |deferral| {
            fixture.objects_with(
                PublicExecute::Revoke,
                [
                    check_function(&schema, "v1")?,
                    check_trigger(&schema, deferral)?,
                ],
                [],
            )
        };
        let (install, change) = (declare(DEFERRED)?, declare(IMMEDIATE)?);
        let applying = fixture
            .pool_with(Role::Migrator, &[("lock_timeout", "300ms")])
            .await?;
        let blocker = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let _ = fixture
            .apply(&applying, &install.manifest)
            .await
            .map_err(with_sqlstate)?;
        let catalog = catalog_state(&observer, &schema).await?;
        let mut reader = blocker.begin().await?;
        bounded(
            sqlx::query(AssertSqlSafe(format!(
                "LOCK TABLE {schema}.records IN ACCESS SHARE MODE"
            )))
            .execute(&mut *reader),
        )
        .await??;
        let outcome = timeout(
            Duration::from_secs(10),
            fixture.apply(&applying, &change.manifest),
        )
        .await?;
        reader.rollback().await?;
        require(
            matches!(&outcome, Err(error @ ManagedApplyError::Statement(_))
                if sqlstate(error).as_deref() == Some("55P03")),
            "the lock wait did not end at the session lock_timeout",
        )?;
        require(
            catalog_state(&observer, &schema).await? == catalog,
            "an apply that timed out changed the catalog",
        )?;
        let applied = fixture
            .apply(&applying, &change.manifest)
            .await
            .map_err(with_sqlstate)?;
        require(
            applied.created_or_replaced() == 1,
            "apply did not re-create the constraint trigger once the table was free",
        )
    })
    .await
}
