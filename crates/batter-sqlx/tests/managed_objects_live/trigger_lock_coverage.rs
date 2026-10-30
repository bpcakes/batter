//! Work that the catalog read after a retirement adds beyond apply's lock
//! set. Retiring a trigger makes apply read the catalog again, because a
//! partitioned table's retirement drops its partition clones. Another session
//! can replace a managed trigger before that read, which then adds a drop on a
//! table apply did not lock, or on one it holds only in `SHARE ROW EXCLUSIVE`
//! mode. Requesting that lock while holding the set could deadlock, so apply
//! refuses the work with `IdentityMismatch` and changes nothing.

use crate::verification::LockStatus;

use super::constraint_triggers::{check_function, check_log};
use super::trigger_drops::{RacePools, wait_for_any_lock_wait, with_sqlstate};
use super::trigger_lock_order::concurrent_write;
use super::verification::wait_for_lock;
use super::{Fixture, Role, pid};
use crate::support::{Result, bounded, require};
use batter_core::BoxError;
use batter_sqlx::managed::{
    AppliedManagedObjects, ManagedApplyError, ManagedObject, ManagedObjectManifest, PublicExecute,
    Retirement,
};
use batter_sqlx::verification::{Identifier, QualifiedName};
use sqlx::AssertSqlSafe;
use tokio::sync::Notify;

/// The plain trigger `beta_audit` on `beta`, firing on `beta_events`, and the
/// plain trigger `alpha_audit` on `alpha`, declared or retired.
fn coverage_objects(
    fixture: &Fixture,
    retire_alpha: AlphaDisposition,
    beta_events: &str,
) -> Result<ManagedObjectManifest> {
    let retire_alpha = retire_alpha == AlphaDisposition::Retire;
    let schema = fixture.schema();
    let plain = |table: &str, name: &str, events: &str| {
        ManagedObject::trigger(
            QualifiedName::new(schema, table)?,
            Identifier::new(name)?,
            format!(
                "CREATE OR REPLACE TRIGGER {name} AFTER {events} ON {schema}.{table}
                 FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
            ),
        )
        .map_err(BoxError::from)
    };
    let mut objects = vec![
        check_function(schema, "v1")?,
        plain("beta", "beta_audit", beta_events)?,
    ];
    let mut retirements = Vec::new();
    if retire_alpha {
        retirements.push(Retirement::trigger(
            QualifiedName::new(schema, "alpha")?,
            Identifier::new("alpha_audit")?,
        ));
    } else {
        objects.push(plain("alpha", "alpha_audit", "INSERT")?);
    }
    Ok(fixture
        .objects_with(PublicExecute::Revoke, objects, retirements)?
        .manifest)
}

/// The number of triggers named `name` in `schema`, and how many of them are
/// constraint triggers.
async fn triggers_named(observer: &sqlx::PgPool, schema: &str, name: &str) -> Result<(i64, i64)> {
    Ok(bounded(
        sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE t.tgconstraint <> 0)
             FROM pg_catalog.pg_trigger AS t
             JOIN pg_catalog.pg_class AS c ON c.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND t.tgname = $2",
        )
        .bind(schema)
        .bind(name)
        .fetch_one(observer),
    )
    .await??)
}

/// Apply `change`, which retires `alpha_audit` and declares `beta_audit` with
/// `beta_events`, while:
/// 1. a reader holds `alpha`, so apply first waits for it alone;
/// 2. meanwhile another session replaces `beta_audit` by a constraint
///    trigger, and a second reader reads `beta`;
/// 3. the first reader commits;
/// 4. once apply has finished or waits for `beta`, the second reader writes
///    `alpha` and commits.
///
/// The read after the retirement then wants `beta_audit` dropped. Apply must
/// refuse that work, leave the catalog as the other sessions left it, and
/// converge on the next apply.
async fn coverage_race(fixture: &mut Fixture, beta_events: &str) -> Result {
    let schema = fixture.schema().to_owned();
    fixture.add_migration(check_log(&schema));
    fixture.add_migration(format!(
        "CREATE TABLE {schema}.alpha (id bigint NOT NULL);
         CREATE TABLE {schema}.beta (id bigint NOT NULL);"
    ));
    let install = coverage_objects(fixture, AlphaDisposition::Retain, "INSERT")?;
    let change = coverage_objects(fixture, AlphaDisposition::Retire, beta_events)?;
    let beta = change
        .objects()
        .iter()
        .find(|object| object.identity().contains("beta_audit"))
        .map(|object| object.identity().to_owned())
        .ok_or_else(|| std::io::Error::other("the manifest declares no beta_audit"))?;
    let pools = RacePools::new(fixture, &[]).await?;
    let holding = fixture.pool(Role::Migrator).await?;
    let _ = fixture
        .apply(&pools.applying, &install)
        .await
        .map_err(with_sqlstate)?;
    let apply_pid = pid(&pools.applying).await?;
    let mut alpha_reader = pools.blocker.begin().await?;
    bounded(
        sqlx::query(AssertSqlSafe(format!(
            "SELECT count(*) FROM {schema}.alpha"
        )))
        .execute(&mut *alpha_reader),
    )
    .await??;
    let done = Notify::new();
    let apply = async {
        let outcome = fixture.apply(&pools.applying, &change).await;
        done.notify_one();
        outcome
    };
    let others = async {
        let observer = &pools.observer;
        wait_for_lock(
            observer,
            apply_pid,
            &schema,
            "alpha",
            "AccessExclusiveLock",
            LockStatus::Waiting,
        )
        .await?;
        bounded(
            sqlx::raw_sql(AssertSqlSafe(format!(
                "BEGIN;
                 DROP TRIGGER beta_audit ON {schema}.beta;
                 CREATE CONSTRAINT TRIGGER beta_audit AFTER INSERT ON {schema}.beta
                     DEFERRABLE INITIALLY DEFERRED
                     FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();
                 COMMIT;"
            )))
            .execute(observer),
        )
        .await??;
        let mut beta_reader = holding.begin().await?;
        bounded(
            sqlx::query(AssertSqlSafe(format!("SELECT count(*) FROM {schema}.beta")))
                .execute(&mut *beta_reader),
        )
        .await??;
        alpha_reader.commit().await?;
        tokio::select! {
            () = done.notified() => {}
            waited = wait_for_any_lock_wait(observer, apply_pid, &schema, "beta") => waited?,
        }
        concurrent_write(
            &mut beta_reader,
            format!("INSERT INTO {schema}.alpha (id) VALUES (1)"),
        )
        .await?;
        beta_reader.commit().await?;
        Ok::<_, BoxError>(())
    };
    let (outcome, written) = tokio::join!(apply, others);
    written?;
    require_refused(outcome, &beta)?;
    require_converges(fixture, &pools, &schema, &change).await
}

/// Fail unless `outcome` refuses the work on `beta` with `IdentityMismatch`.
fn require_refused(
    outcome: std::result::Result<AppliedManagedObjects, ManagedApplyError>,
    beta: &str,
) -> Result {
    match outcome {
        Err(ManagedApplyError::IdentityMismatch { identity }) if identity == beta => Ok(()),
        other => Err(std::io::Error::other(format!(
            "apply did not refuse the uncovered work on beta_audit: {:?}",
            other
                .map(|applied| (applied.created_or_replaced(), applied.retired()))
                .map_err(with_sqlstate)
        ))
        .into()),
    }
}

/// The refused apply left both triggers as the other sessions left them, and
/// the next apply of `change` retires `alpha_audit` and restores the plain
/// `beta_audit`.
async fn require_converges(
    fixture: &Fixture,
    pools: &RacePools,
    schema: &str,
    change: &ManagedObjectManifest,
) -> Result {
    require!(
        triggers_named(&pools.observer, schema, "alpha_audit").await? == (1, 0),
        "the refused apply did not leave alpha_audit in place",
    )?;
    require!(
        triggers_named(&pools.observer, schema, "beta_audit").await? == (1, 1),
        "the refused apply did not leave the concurrent constraint trigger in place",
    )?;
    let applied = fixture
        .apply(&pools.applying, change)
        .await
        .map_err(with_sqlstate)?;
    require!(
        (applied.created_or_replaced(), applied.retired()) == (1, 1),
        "the next apply did not retire alpha_audit and replace beta_audit",
    )?;
    require!(
        triggers_named(&pools.observer, schema, "beta_audit").await? == (1, 0),
        "the next apply did not restore the plain beta_audit",
    )
}

/// `beta_audit` is unchanged, so `beta` is not in the lock set. The read after
/// retiring `alpha_audit` finds the concurrent constraint trigger and would
/// drop it, locking `beta` while apply holds `alpha`.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_retirement_refuses_new_work_on_an_unlocked_table() -> Result {
    Fixture::run(async |fixture| coverage_race(fixture, "INSERT").await).await
}

/// `beta_audit` changes, so apply holds `beta` in `SHARE ROW EXCLUSIVE` mode.
/// The read after retiring `alpha_audit` finds the concurrent constraint
/// trigger, whose drop needs `ACCESS EXCLUSIVE` on `beta`.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_retirement_refuses_new_work_needing_a_stronger_lock() -> Result {
    Fixture::run(async |fixture| coverage_race(fixture, "INSERT OR UPDATE").await).await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AlphaDisposition {
    Retire,
    Retain,
}
