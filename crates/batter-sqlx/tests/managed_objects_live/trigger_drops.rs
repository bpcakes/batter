//! Trigger drops decided from the trigger read under its table's lock: a
//! retirement or a drop before create never acts on a catalog read that another
//! session's committed replacement, or an earlier retirement's cascade, has
//! made stale.

use super::constraint_triggers::{
    DEFERRED, IMMEDIATE, check_body, check_function, check_log, check_trigger, fired, holder,
    ledger_row, plain_check_trigger, verifies,
};
use super::verification::wait_for_lock;
use super::{Fixture, Role, pid, sqlstate};
use crate::support::{Result, bounded, require};
use batter_core::BoxError;
use batter_sqlx::managed::{
    AppliedManagedObjects, ManagedApplyError, ManagedObject, ManagedObjectManifest, PublicExecute,
    Retirement,
};
use batter_sqlx::verification::{Identifier, QualifiedName};
use sqlx::{AssertSqlSafe, PgPool};

/// A failed apply as a test error naming only its SQLSTATE.
fn with_sqlstate(error: ManagedApplyError) -> BoxError {
    std::io::Error::other(format!("apply failed with SQLSTATE {:?}", sqlstate(&error))).into()
}

/// One applying session, one session that races it and one observer.
struct RacePools {
    applying: PgPool,
    blocker: PgPool,
    observer: PgPool,
}

impl RacePools {
    async fn new(fixture: &mut Fixture, settings: &[(&str, &str)]) -> Result<Self> {
        Ok(Self {
            applying: fixture.pool_with(Role::Migrator, settings).await?,
            blocker: fixture.pool(Role::Migrator).await?,
            observer: fixture.pool(Role::Migrator).await?,
        })
    }
}

/// Apply `manifest` while another session's transaction, which took
/// `ACCESS EXCLUSIVE` on `records` and ran `replacement` before apply started,
/// commits only once apply has read the catalog and waits for its own
/// `ACCESS EXCLUSIVE` request on the table.
async fn apply_racing(
    fixture: &Fixture,
    pools: &RacePools,
    manifest: &ManagedObjectManifest,
    replacement: &str,
) -> Result<AppliedManagedObjects> {
    let schema = fixture.schema();
    let apply_pid = pid(&pools.applying).await?;
    let mut gate = pools.blocker.begin().await?;
    bounded(
        sqlx::query(AssertSqlSafe(format!(
            "LOCK TABLE {schema}.records IN ACCESS EXCLUSIVE MODE"
        )))
        .execute(&mut *gate),
    )
    .await??;
    bounded(sqlx::raw_sql(AssertSqlSafe(replacement.to_owned())).execute(&mut *gate)).await??;
    let apply = async {
        fixture
            .apply(&pools.applying, manifest)
            .await
            .map_err(with_sqlstate)
    };
    let release = async {
        wait_for_lock(
            &pools.observer,
            apply_pid,
            schema,
            "records",
            "AccessExclusiveLock",
            false,
        )
        .await?;
        gate.commit().await?;
        Ok::<_, BoxError>(())
    };
    let (applied, ()) = tokio::try_join!(apply, release)?;
    Ok(applied)
}

/// SQL for the racing session: drop `records_check`, then re-create it as a
/// constraint trigger (`Some(true)`) or a plain trigger (`Some(false)`), each
/// with a definition no manifest declares, or leave it removed (`None`).
fn replacement(schema: &str, constraint: Option<bool>) -> String {
    let create = match constraint {
        Some(true) => format!(
            "CREATE CONSTRAINT TRIGGER records_check AFTER INSERT ON {schema}.records
             NOT DEFERRABLE FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
        ),
        Some(false) => format!(
            "CREATE TRIGGER records_check AFTER INSERT ON {schema}.records
             FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
        ),
        None => String::new(),
    };
    format!("DROP TRIGGER records_check ON {schema}.records; {create}")
}

/// Install `records_check` as a constraint or plain trigger, then retire that
/// kind while the racing session replaces it with the other kind. The
/// retirement must leave the replacement in place.
async fn retirement_race(
    fixture: &mut Fixture,
    constraint: bool,
    settings: &[(&str, &str)],
) -> Result {
    let schema = fixture.schema().to_owned();
    fixture.add_migration(check_log(&schema));
    let (table, name) = (
        QualifiedName::new(&schema, "records")?,
        Identifier::new("records_check")?,
    );
    let (installed, retirement) = if constraint {
        (
            check_trigger(&schema, DEFERRED)?,
            Retirement::constraint_trigger(table, name),
        )
    } else {
        (
            plain_check_trigger(&schema)?,
            Retirement::trigger(table, name),
        )
    };
    let install = fixture.objects_with(
        PublicExecute::Revoke,
        [check_function(&schema, "v1")?, installed],
        [],
    )?;
    let retire = fixture.objects_with(
        PublicExecute::Revoke,
        [check_function(&schema, "v1")?],
        [retirement],
    )?;
    let pools = RacePools::new(fixture, settings).await?;
    let _ = fixture
        .apply(&pools.applying, &install.manifest)
        .await
        .map_err(with_sqlstate)?;
    let applied = apply_racing(
        fixture,
        &pools,
        &retire.manifest,
        &replacement(&schema, Some(!constraint)),
    )
    .await?;
    require(
        applied.retired() == 0,
        "a retirement dropped another kind's concurrent replacement",
    )?;
    require(
        holder(&pools.observer, &schema).await?.1 != constraint,
        "the other kind's replacement did not survive the retirement",
    )?;
    // Verification is kind-exact too: the other kind holding a retired
    // identity is not the retired object.
    let reader = fixture.pool(Role::Reader).await?;
    require(
        verifies(fixture, &reader, &retire.manifest).await?,
        "the surviving replacement was reported as the retired object",
    )
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_retirement_spares_a_concurrent_plain_replacement() -> Result {
    Fixture::run(async |fixture| retirement_race(fixture, true, &[]).await).await
}

/// The applying login defaults to REPEATABLE READ, so the case also fails if
/// apply stops pinning READ COMMITTED: the read under the lock would then see
/// the transaction's first snapshot.
#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_retirement_spares_a_concurrent_constraint_replacement() -> Result {
    Fixture::run(async |fixture| {
        retirement_race(
            fixture,
            false,
            &[("default_transaction_isolation", "repeatable read")],
        )
        .await
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_triggers_converge_over_any_concurrent_holder() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let declare = |trigger| {
            fixture.objects_with(
                PublicExecute::Revoke,
                [check_function(&schema, "v1")?, trigger],
                [],
            )
        };
        let install = declare(check_trigger(&schema, DEFERRED)?)?;
        // Both declarations must drop the installed constraint trigger; the
        // racing session replaces it with either kind or removes it first.
        let constraint = declare(check_trigger(&schema, IMMEDIATE)?)?;
        let plain = declare(plain_check_trigger(&schema)?)?;
        let pools = RacePools::new(fixture, &[]).await?;
        let reader = fixture.pool(Role::Reader).await?;
        for (declared, kind, clause) in [
            (&constraint, "constraint_trigger", IMMEDIATE),
            (&plain, "trigger", "AFTER INSERT OR UPDATE"),
        ] {
            for holder_kind in [Some(true), Some(false), None] {
                let _ = fixture
                    .apply(&pools.applying, &install.manifest)
                    .await
                    .map_err(with_sqlstate)?;
                let applied = apply_racing(
                    fixture,
                    &pools,
                    &declared.manifest,
                    &replacement(&schema, holder_kind),
                )
                .await?;
                let (_, is_constraint, enabled, definition) =
                    holder(&pools.observer, &schema).await?;
                require(
                    applied.created_or_replaced() == 1
                        && is_constraint == (kind == "constraint_trigger")
                        && enabled == "O"
                        && definition.contains(clause),
                    "a declared trigger did not converge over the concurrent holder",
                )?;
                require(
                    ledger_row(fixture, &pools.observer).await?.0 == kind,
                    "the ledger did not record the declared kind",
                )?;
                require(
                    verifies(fixture, &reader, &declared.manifest).await?,
                    "a converged declaration did not verify",
                )?;
            }
        }
        Ok(())
    })
    .await
}

/// Every table holding a `parted_check` trigger, and whether that trigger is a
/// partition clone of a parent's trigger.
async fn parted_holders(pool: &PgPool, schema: &str) -> Result<Vec<(String, bool)>> {
    Ok(sqlx::query_as(
        "SELECT c.relname::text, t.tgparentid <> 0
         FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND t.tgname = 'parted_check' AND NOT t.tgisinternal
         ORDER BY 1",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?)
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_replaces_a_retired_parent_clone_in_one_apply() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        // PostgreSQL clones a partitioned parent's trigger onto each partition.
        fixture.add_migration(format!(
            "CREATE TABLE {schema}.parted (id bigint NOT NULL, region text NOT NULL)
                 PARTITION BY LIST (region);
             CREATE TABLE {schema}.parted_east PARTITION OF {schema}.parted
                 FOR VALUES IN ('east');
             CREATE FUNCTION {schema}.check_record() {}
             CREATE CONSTRAINT TRIGGER parted_check AFTER INSERT ON {schema}.parted
             {DEFERRED} FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();",
            check_body(&schema, "migration")
        ));
        let migrator = fixture.pool(Role::Migrator).await?;
        fixture.migrate_only(&migrator).await?;
        require(
            parted_holders(&migrator, &schema).await?
                == [
                    ("parted".to_owned(), false),
                    ("parted_east".to_owned(), true),
                ],
            "the migration did not clone the parent trigger onto the partition",
        )?;
        // Retiring the parent's trigger also drops its clone, so the declared
        // partition trigger has nothing left to drop when it is created.
        let objects = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                ManagedObject::constraint_trigger(
                    QualifiedName::new(&schema, "parted_east")?,
                    Identifier::new("parted_check")?,
                    format!(
                        "CREATE CONSTRAINT TRIGGER parted_check AFTER INSERT ON {schema}.parted_east
                         {DEFERRED} FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
                    ),
                )?,
            ],
            [Retirement::constraint_trigger(
                QualifiedName::new(&schema, "parted")?,
                Identifier::new("parted_check")?,
            )],
        )?;
        let applied = fixture
            .apply(&migrator, &objects.manifest)
            .await
            .map_err(with_sqlstate)?;
        require(
            applied.retired() == 1 && applied.created_or_replaced() == 2,
            "apply did not retire the parent trigger and create the partition trigger",
        )?;
        require(
            parted_holders(&migrator, &schema).await? == [("parted_east".to_owned(), false)],
            "the partition trigger is not a standalone constraint trigger",
        )?;
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO {schema}.parted (id, region) VALUES (1, 'east')"
        )))
        .execute(&migrator)
        .await?;
        require(
            fired(&migrator, &schema).await? == ["v1"],
            "the partition write did not fire the managed body exactly once",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        require(
            verifies(fixture, &reader, &objects.manifest).await?,
            "the retired parent and declared partition trigger did not verify",
        )?;
        let again = fixture.apply(&migrator, &objects.manifest).await?;
        require(
            again.created_or_replaced() == 0 && again.retired() == 0,
            "a repeated apply executed DDL",
        )
    })
    .await
}
