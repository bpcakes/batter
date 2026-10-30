//! Constraint triggers installed, changed, adopted, repaired and retired in the
//! transaction that installs the routines they bind.

use crate::verification::LockStatus;

use super::verification::wait_for_lock;
use super::{Fixture, Role, catalog_state, pid};
use crate::support::{Result, bounded, require};
use batter_sqlx::managed::{ManagedObject, ManagedObjectManifest, PublicExecute, Retirement};
use batter_sqlx::verification::{
    FindingKind, Identifier, QualifiedName, RoutineSignature, VerificationPlan, VerificationStatus,
};
use sqlx::{AssertSqlSafe, PgExecutor, PgPool};

pub(super) const DEFERRED: &str = "DEFERRABLE INITIALLY DEFERRED";
pub(super) const IMMEDIATE: &str = "DEFERRABLE INITIALLY IMMEDIATE";

/// The table every firing of `check_record` appends its body version to.
pub(super) fn check_log(schema: &str) -> String {
    format!(
        "CREATE TABLE {schema}.check_log (
             id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
             version text NOT NULL)"
    )
}

pub(super) fn check_body(schema: &str, version: &str) -> String {
    format!(
        "RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
             INSERT INTO {schema}.check_log (version) VALUES ('{version}');
             RETURN NULL;
         END $$;"
    )
}

pub(super) fn check_function(schema: &str, version: &str) -> Result<ManagedObject> {
    Ok(ManagedObject::function(
        RoutineSignature::new(schema, "check_record", [])?,
        format!(
            "CREATE OR REPLACE FUNCTION {schema}.check_record() {}",
            check_body(schema, version)
        ),
    )?)
}

pub(super) fn check_trigger(schema: &str, deferral: &str) -> Result<ManagedObject> {
    Ok(ManagedObject::constraint_trigger(
        QualifiedName::new(schema, "records")?,
        Identifier::new("records_check")?,
        format!(
            "CREATE CONSTRAINT TRIGGER records_check AFTER INSERT OR UPDATE ON {schema}.records
             {deferral} FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
        ),
    )?)
}

pub(super) fn plain_check_trigger(schema: &str) -> Result<ManagedObject> {
    Ok(ManagedObject::trigger(
        QualifiedName::new(schema, "records")?,
        Identifier::new("records_check")?,
        format!(
            "CREATE OR REPLACE TRIGGER records_check AFTER INSERT OR UPDATE ON {schema}.records
             FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();"
        ),
    )?)
}

/// Body versions in firing order.
pub(super) async fn fired<'e>(executor: impl PgExecutor<'e>, schema: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT version FROM {schema}.check_log ORDER BY id"
    )))
    .fetch_all(executor)
    .await?)
}

pub(super) async fn insert<'e>(executor: impl PgExecutor<'e>, schema: &str) -> Result {
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {schema}.records (note) VALUES ('checked')"
    )))
    .execute(executor)
    .await?;
    Ok(())
}

/// The trigger holding `records_check`: oid, whether it is a constraint
/// trigger, its enabled state and its definition.
pub(super) async fn holder(pool: &PgPool, schema: &str) -> Result<(i64, bool, String, String)> {
    Ok(sqlx::query_as(
        "SELECT t.oid::bigint, t.tgconstraint <> 0, t.tgenabled::text, pg_get_triggerdef(t.oid)
         FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = 'records' AND t.tgname = 'records_check'",
    )
    .bind(schema)
    .fetch_one(pool)
    .await?)
}

/// The ledger row of `records_check`: kind, source checksum and rendered hash.
pub(super) async fn ledger_row(
    fixture: &Fixture,
    pool: &PgPool,
) -> Result<(String, Vec<u8>, Vec<u8>)> {
    let identity = format!("\"records_check\" ON \"{}\".\"records\"", fixture.schema());
    fixture
        .object_rows(pool)
        .await?
        .into_iter()
        .find(|row| row.0 == identity)
        .map(|(_, kind, source, rendered, _)| (kind, source, rendered))
        .ok_or_else(|| std::io::Error::other("the ledger has no row for records_check").into())
}

pub(super) async fn verifies(
    fixture: &Fixture,
    reader: &PgPool,
    manifest: &ManagedObjectManifest,
) -> Result<bool> {
    let report = fixture
        .verify(reader, VerificationPlan::managed_objects(manifest))
        .await?;
    Ok(report.status() == VerificationStatus::WithinDeclaredPolicy)
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_bootstraps_with_its_new_function() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let objects = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        // Neither the function nor its constraint trigger exists beforehand.
        let applied = fixture.apply(&migrator, &objects.manifest).await?;
        require!(
            applied.created_or_replaced() == 2 && applied.unchanged() == 0,
            "one apply did not install the function and its constraint trigger",
        )?;
        let (_, constraint, enabled, definition) = holder(&migrator, &schema).await?;
        require!(
            constraint && enabled == "O" && definition.contains(DEFERRED),
            "apply did not create an enabled deferred constraint trigger",
        )?;
        require!(
            ledger_row(fixture, &migrator).await?.0 == "constraint_trigger",
            "the ledger did not record the constraint trigger kind",
        )?;
        // A write in a later transaction fires the installed body at commit.
        let mut transaction = migrator.begin().await?;
        insert(&mut *transaction, &schema).await?;
        require!(
            fired(&mut *transaction, &schema).await?.is_empty(),
            "the deferred constraint trigger fired before commit",
        )?;
        transaction.commit().await?;
        require!(
            fired(&migrator, &schema).await? == ["v1"],
            "the constraint trigger did not run the installed body",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        let sqlx = fixture.sqlx_manifest()?;
        let plan =
            VerificationPlan::sqlx_migrations(&sqlx).with_managed_objects(&objects.manifest)?;
        require!(
            fixture.verify(&reader, plan).await?.status()
                == VerificationStatus::WithinDeclaredPolicy,
            "the bootstrapped set did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_reapply_executes_no_ddl() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let objects = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        let catalog = catalog_state(&migrator, &schema).await?;
        let ledger = fixture.object_rows(&migrator).await?;
        // A writer holding the trigger's table does not delay the re-apply.
        let again = fixture
            .apply_beside_writer(&migrator, &objects.manifest, "records")
            .await?;
        require!(
            again.created_or_replaced() == 0 && again.unchanged() == 2 && again.retired() == 0,
            "a repeated apply re-executed an unchanged constraint trigger",
        )?;
        // The trigger's oid and xmin are part of the state.
        require!(
            catalog_state(&migrator, &schema).await? == catalog,
            "a repeated apply dropped and re-created the constraint trigger",
        )?;
        require!(
            fixture.object_rows(&migrator).await? == ledger,
            "a repeated apply rewrote a ledger row or its applied_at",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        require!(
            verifies(fixture, &reader, &objects.manifest).await?,
            "a repeated apply did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_source_change_recreates_it() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let deferred = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let immediate = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, IMMEDIATE)?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&migrator, &deferred.manifest).await?;
        let (oid, ..) = holder(&migrator, &schema).await?;
        let (kind, source, rendered) = ledger_row(fixture, &migrator).await?;
        let applied = fixture.apply(&migrator, &immediate.manifest).await?;
        require!(
            applied.created_or_replaced() == 1 && applied.unchanged() == 1,
            "a changed constraint trigger was not the only re-created entry",
        )?;
        let (changed_oid, constraint, _, definition) = holder(&migrator, &schema).await?;
        require!(
            changed_oid != oid
                && constraint
                && definition.contains(IMMEDIATE)
                && !definition.contains("INITIALLY DEFERRED"),
            "the changed definition did not replace the previous one",
        )?;
        let (changed_kind, changed_source, changed_rendered) =
            ledger_row(fixture, &migrator).await?;
        require!(
            changed_kind == kind && changed_source != source && changed_rendered != rendered,
            "the ledger did not record the changed source and rendering",
        )?;
        // The re-created trigger checks at the end of each statement.
        let mut transaction = migrator.begin().await?;
        insert(&mut *transaction, &schema).await?;
        require!(
            fired(&mut *transaction, &schema).await? == ["v1"],
            "the re-created constraint trigger still deferred its check",
        )?;
        transaction.rollback().await?;
        let reader = fixture.pool(Role::Reader).await?;
        require!(
            verifies(fixture, &reader, &immediate.manifest).await?,
            "the changed constraint trigger did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_from_a_migration_is_adopted() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        // The pre-managed form: a migration binds the body it can see.
        fixture.add_migration(format!(
            "CREATE FUNCTION {schema}.check_record() {}
             CREATE CONSTRAINT TRIGGER records_check AFTER INSERT OR UPDATE ON {schema}.records
             {DEFERRED} FOR EACH ROW EXECUTE FUNCTION {schema}.check_record();",
            check_body(&schema, "migration")
        ));
        let migrator = fixture.pool(Role::Migrator).await?;
        fixture.migrate_only(&migrator).await?;
        let (migrated_oid, constraint, ..) = holder(&migrator, &schema).await?;
        require!(
            constraint,
            "the migration did not create a constraint trigger",
        )?;
        insert(&migrator, &schema).await?;
        let objects = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let applied = fixture.apply(&migrator, &objects.manifest).await?;
        require!(
            applied.created_or_replaced() == 2,
            "apply did not adopt both migration-created objects",
        )?;
        let (oid, constraint, enabled, definition) = holder(&migrator, &schema).await?;
        require!(
            oid != migrated_oid && constraint && enabled == "O" && definition.contains(DEFERRED),
            "the adopted constraint trigger was not re-created from its declaration",
        )?;
        require!(
            ledger_row(fixture, &migrator).await?.0 == "constraint_trigger",
            "the adopted constraint trigger was not recorded",
        )?;
        insert(&migrator, &schema).await?;
        require!(
            fired(&migrator, &schema).await? == ["migration", "v1"],
            "the adopted constraint trigger did not run the managed body",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        require!(
            verifies(fixture, &reader, &objects.manifest).await?,
            "the adopted set did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_drift_is_reported_and_repaired() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let objects = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        let identity = format!("\"records_check\" ON \"{schema}\".\"records\"");
        let recreate = |clauses: &str| {
            format!(
                "DROP TRIGGER records_check ON {schema}.records;
                 CREATE CONSTRAINT TRIGGER records_check AFTER INSERT OR UPDATE ON {schema}.records
                 {clauses} EXECUTE FUNCTION {schema}.check_record()"
            )
        };
        for drift in [
            recreate("NOT DEFERRABLE FOR EACH ROW"),
            recreate(&format!("{DEFERRED} FOR EACH ROW WHEN (NEW.note <> '')")),
            format!("ALTER TABLE {schema}.records DISABLE TRIGGER records_check"),
        ] {
            fixture.admin(&drift).await?;
            let report = fixture
                .verify(
                    &reader,
                    VerificationPlan::managed_objects(&objects.manifest),
                )
                .await?;
            require!(
                report.status() == VerificationStatus::Violations
                    && report.findings().len() == 1
                    && report.findings()[0].kind == FindingKind::ManagedObjectDrift
                    && report.findings()[0].object.as_deref() == Some(identity.as_str()),
                "out-of-band constraint trigger drift was not reported",
            )?;
            let repaired = fixture.apply(&migrator, &objects.manifest).await?;
            require!(
                repaired.created_or_replaced() == 1 && repaired.unchanged() == 1,
                "apply did not re-create exactly the drifted constraint trigger",
            )?;
            let (_, constraint, enabled, definition) = holder(&migrator, &schema).await?;
            require!(
                constraint && enabled == "O" && definition.contains(DEFERRED),
                "apply did not restore the declared constraint trigger",
            )?;
            require!(
                verifies(fixture, &reader, &objects.manifest).await?,
                "repaired constraint trigger drift did not verify",
            )?;
        }
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_retires_before_its_routine() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let installed = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &installed.manifest).await?;
        // Dropping the routine first would fail: the trigger depends on it.
        let retired = fixture.objects_with(
            PublicExecute::Revoke,
            [],
            [
                Retirement::function(RoutineSignature::new(&schema, "check_record", [])?),
                Retirement::constraint_trigger(
                    QualifiedName::new(&schema, "records")?,
                    Identifier::new("records_check")?,
                ),
            ],
        )?;
        let applied = fixture.apply(&migrator, &retired.manifest).await?;
        require!(
            applied.retired() == 2,
            "retirement did not drop the constraint trigger and its routine",
        )?;
        require!(
            !fixture.exists(&migrator, "check_record").await?
                && !fixture.trigger_exists(&migrator, "records_check").await?,
            "retired objects still exist",
        )?;
        require!(
            verifies(fixture, &reader, &retired.manifest).await?,
            "a completed constraint trigger retirement did not verify",
        )?;
        require!(
            fixture.apply(&migrator, &retired.manifest).await?.retired() == 0,
            "an absent retired constraint trigger was dropped again",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_retirements_leave_the_other_trigger_kind_alone() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let table = QualifiedName::new(&schema, "records")?;
        let name = Identifier::new("records_check")?;
        for (declared, retirement) in [
            (
                check_trigger(&schema, DEFERRED)?,
                Retirement::trigger(table.clone(), name.clone()),
            ),
            (
                plain_check_trigger(&schema)?,
                Retirement::constraint_trigger(table.clone(), name.clone()),
            ),
        ] {
            let installed = fixture.objects_with(
                PublicExecute::Revoke,
                [check_function(&schema, "v1")?, declared],
                [],
            )?;
            let _ = fixture.apply(&migrator, &installed.manifest).await?;
            let (oid, ..) = holder(&migrator, &schema).await?;
            // The other kind under the retired identity is not the retired object.
            let retiring = fixture.objects_with(
                PublicExecute::Revoke,
                [check_function(&schema, "v1")?],
                [retirement],
            )?;
            let applied = fixture.apply(&migrator, &retiring.manifest).await?;
            require!(
                applied.retired() == 0 && holder(&migrator, &schema).await?.0 == oid,
                "a trigger was retired as the other trigger kind",
            )?;
            require!(
                verifies(fixture, &reader, &retiring.manifest).await?,
                "the other trigger kind was reported as the retired object",
            )?;
        }
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_trigger_kind_changes_converge_in_both_directions() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        let plain = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                plain_check_trigger(&schema)?,
            ],
            [],
        )?;
        let constraint = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &plain.manifest).await?;
        require!(
            !holder(&migrator, &schema).await?.1,
            "the plain trigger was created as a constraint trigger",
        )?;
        for (objects, is_constraint, kind) in [
            (&constraint, true, "constraint_trigger"),
            (&plain, false, "trigger"),
        ] {
            let applied = fixture.apply(&migrator, &objects.manifest).await?;
            require!(
                applied.created_or_replaced() == 1 && applied.unchanged() == 1,
                "a kind change did not re-create exactly the trigger",
            )?;
            let (_, constraint, enabled, _) = holder(&migrator, &schema).await?;
            require!(
                constraint == is_constraint && enabled == "O",
                "the trigger holding the name has the wrong kind",
            )?;
            require!(
                ledger_row(fixture, &migrator).await?.0 == kind,
                "the ledger did not record the changed kind",
            )?;
            require!(
                verifies(fixture, &reader, &objects.manifest).await?,
                "a converged kind change did not verify",
            )?;
            require!(
                fixture
                    .apply(&migrator, &objects.manifest)
                    .await?
                    .created_or_replaced()
                    == 0,
                "a re-apply after a kind change executed DDL",
            )?;
        }
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_constraint_trigger_and_its_new_body_commit_together() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        fixture.add_migration(check_log(&schema));
        fixture.add_migration(format!("CREATE TABLE {schema}.gate (id bigint)"));
        let original = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v1")?,
                check_trigger(&schema, DEFERRED)?,
            ],
            [],
        )?;
        // The next release changes the body and the constraint trigger, and
        // adds a SQL routine reading a table the test holds. Validating its
        // body waits for that table, which pauses apply after the constraint
        // trigger's drop, with the table's lock held, and before its commit.
        let changed = fixture.objects_with(
            PublicExecute::Revoke,
            [
                check_function(&schema, "v2")?,
                check_trigger(&schema, IMMEDIATE)?,
                ManagedObject::function(
                    RoutineSignature::new(&schema, "gate_probe", [])?,
                    format!(
                        "CREATE OR REPLACE FUNCTION {schema}.gate_probe() RETURNS bigint
                         LANGUAGE sql STABLE AS $$ SELECT count(*) FROM {schema}.gate $$;"
                    ),
                )?,
            ],
            [],
        )?;
        let applying = fixture.pool(Role::Migrator).await?;
        let writer = fixture.pool(Role::Migrator).await?;
        let blocker = fixture.pool(Role::Migrator).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&applying, &original.manifest).await?;
        insert(&writer, &schema).await?;
        let (apply_pid, writer_pid) = (pid(&applying).await?, pid(&writer).await?);
        let mut gate = blocker.begin().await?;
        bounded(
            sqlx::query(AssertSqlSafe(format!(
                "LOCK TABLE {schema}.gate IN ACCESS EXCLUSIVE MODE"
            )))
            .execute(&mut *gate),
        )
        .await??;
        let apply = async {
            Ok::<_, batter_core::BoxError>(fixture.apply(&applying, &changed.manifest).await?)
        };
        let race = async {
            wait_for_lock(&observer, apply_pid, &schema, "gate", "AccessShareLock", LockStatus::Waiting)
                .await?;
            wait_for_lock(&observer, apply_pid, &schema, "records", "AccessExclusiveLock", LockStatus::Granted)
                .await?;
            // Neither the dropped trigger nor the re-created one is visible
            // before commit, under either body: the write waits on the lock.
            let write = insert(&writer, &schema);
            tokio::pin!(write);
            tokio::select! {
                result = &mut write => {
                    result?;
                    return Err(std::io::Error::other("a write escaped the apply's table lock").into());
                }
                observed = wait_for_lock(&observer, writer_pid, &schema, "records", "RowExclusiveLock", LockStatus::Waiting) => observed?,
            }
            gate.commit().await?;
            write.await
        };
        let (applied, ()) = tokio::try_join!(apply, race)?;
        require!(
            applied.created_or_replaced() == 3,
            "apply did not install the changed body, its trigger and the gate routine",
        )?;
        require!(
            fired(&observer, &schema).await? == ["v1", "v2"],
            "the blocked write did not run the body committed with its trigger",
        )?;
        require!(
            holder(&observer, &schema).await?.3.contains(IMMEDIATE),
            "the blocked write did not see the re-created trigger",
        )
    })
    .await
}
