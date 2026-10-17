//! PostgreSQL 18 contracts for managed routines, triggers and constraint
//! triggers applied after migrations and verified read-only.
//!
//! Each case provisions a uniquely named schema, a migration login that owns it
//! and a serving login with only schema `USAGE` and table `SELECT`, through the
//! explicitly supplied superuser `BATTER_SQLX_ADMIN_URL`. Cases are ignored by
//! ordinary gates and selected by `scripts/test_sqlx_live.sh`.

#[allow(dead_code)]
mod support;

mod managed_objects_fixture;

#[path = "managed_objects_live/verification.rs"]
mod verification;

#[path = "managed_objects_live/constraint_triggers.rs"]
mod constraint_triggers;

#[path = "managed_objects_live/trigger_drops.rs"]
mod trigger_drops;

#[path = "managed_objects_live/trigger_lock_order.rs"]
mod trigger_lock_order;

#[path = "managed_objects_live/trigger_privileges.rs"]
mod trigger_privileges;

#[path = "managed_objects_live/trigger_lock_coverage.rs"]
mod trigger_lock_coverage;

use batter_sqlx::managed::{ManagedApplyError, ManagedObject, PublicExecute, Retirement};
use batter_sqlx::verification::{
    FindingKind, Identifier, ObjectPrivilege, QualifiedName, RoutineSignature, RoutineType,
    SupportedSurface, VerificationPlan, VerificationStatus,
};
use managed_objects_fixture::{Fixture, Role, catalog_state, pid, sqlstate};
use support::{Result, require};

const RECORD_TOTAL: &str = "record_total(bigint)";

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_apply_after_migrations_and_verify_within_policy() -> Result {
    Fixture::run(async |fixture| {
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let before = pid(&migrator).await?;
        let applied = fixture.apply(&migrator, &objects.manifest).await?;
        require(
            applied.created_or_replaced() == 4 && applied.unchanged() == 0,
            "first apply did not create every declared object",
        )?;
        require(applied.retired() == 0, "first apply retired an object")?;
        require(
            applied.digest() == objects.manifest.digest(),
            "apply did not report the manifest digest",
        )?;
        require(
            pid(&migrator).await? == before,
            "successful apply did not return its connection to the pool",
        )?;
        // The SQL routine was validated against, and reads, the migrated table.
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {}.record_total(5)",
            fixture.schema()
        )))
        .fetch_one(&migrator)
        .await?;
        require(
            total == 5,
            "managed SQL routine returned an unexpected value",
        )?;
        let rows: Vec<(String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT identity, kind FROM {}.managed_objects ORDER BY identity",
            fixture.schema()
        )))
        .fetch_all(&migrator)
        .await?;
        require(
            rows.len() == 5,
            "ledger does not hold four objects and the set row",
        )?;
        require(
            rows.iter()
                .any(|(identity, kind)| identity == "manifest" && kind == "manifest"),
            "ledger has no set row",
        )?;
        for routine in [RECORD_TOTAL, "touch_updated_at()", "archive_records()"] {
            require(
                !fixture.public_can_execute(&migrator, routine).await?,
                "PUBLIC kept EXECUTE on a managed routine",
            )?;
        }

        let reader = fixture.pool(Role::Reader).await?;
        let sqlx = fixture.sqlx_manifest()?;
        let plan =
            VerificationPlan::sqlx_migrations(&sqlx).with_managed_objects(&objects.manifest)?;
        let report = fixture.verify(&reader, plan).await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "applied managed objects did not verify",
        )?;
        require(
            report
                .supported()
                .contains(&SupportedSurface::ManagedObjects)
                && report
                    .supported()
                    .contains(&SupportedSurface::SqlxMigrationLedger),
            "verification did not cover both ledgers",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_reapply_is_idempotent_and_executes_no_ddl() -> Result {
    Fixture::run(async |fixture| {
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        let catalog = catalog_state(&migrator, fixture.schema()).await?;
        let ledger = fixture.object_rows(&migrator).await?;
        // A writer holding the trigger's table does not delay the re-apply.
        let again = fixture
            .apply_beside_writer(&migrator, &objects.manifest, "records")
            .await?;
        require(
            again.created_or_replaced() == 0 && again.unchanged() == 4 && again.retired() == 0,
            "repeated apply re-executed an unchanged object",
        )?;
        // xmin is part of the state: no routine or trigger row was rewritten.
        require(
            catalog_state(&migrator, fixture.schema()).await? == catalog,
            "repeated apply changed the catalog",
        )?;
        require(
            fixture.object_rows(&migrator).await? == ledger,
            "repeated apply rewrote unchanged ledger rows",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "repeated apply did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_retire_signatures_with_dependent_triggers() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let audit = RoutineSignature::new(&schema, "audit_record", [])?;
        let table = QualifiedName::new(&schema, "records")?;
        let audit_trigger = Identifier::new("records_audit")?;
        let initial = fixture.objects_with(
            PublicExecute::Revoke,
            [
                ManagedObject::function(
                    audit.clone(),
                    format!(
                        "CREATE OR REPLACE FUNCTION {schema}.audit_record() RETURNS trigger
                         LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;"
                    ),
                )?,
                ManagedObject::trigger(
                    table.clone(),
                    audit_trigger.clone(),
                    format!(
                        "CREATE OR REPLACE TRIGGER records_audit AFTER INSERT ON {schema}.records
                         FOR EACH ROW EXECUTE FUNCTION {schema}.audit_record();"
                    ),
                )?,
            ],
            [],
        )?;
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &initial.manifest).await?;
        require(
            fixture.exists(&migrator, "audit_record").await?,
            "initial audit routine was not created",
        )?;

        // Retiring a routine and the trigger bound to it drops both, trigger
        // first; the retirement-only manifest also unmanages nothing else.
        let retired = fixture.objects_with(
            PublicExecute::Revoke,
            [],
            [
                Retirement::function(audit.clone()),
                Retirement::trigger(table.clone(), audit_trigger.clone()),
            ],
        )?;
        let applied = fixture.apply(&migrator, &retired.manifest).await?;
        require(
            applied.retired() == 2,
            "retirement did not drop both objects",
        )?;
        require(
            !fixture.exists(&migrator, "audit_record").await?
                && !fixture.trigger_exists(&migrator, "records_audit").await?,
            "retired objects still exist",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&retired.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "a completed retirement did not verify",
        )?;
        let again = fixture.apply(&migrator, &retired.manifest).await?;
        require(
            again.retired() == 0,
            "an absent retired object was dropped again",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&initial.manifest),
            )
            .await?;
        require(
            report
                .findings()
                .iter()
                .any(|finding| finding.kind == FindingKind::ManagedObjectSetDigest),
            "a superseded manifest verified against the retired set",
        )?;

        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_retirement_blocked_by_a_declared_trigger_changes_nothing() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        // A retired routine that a declared trigger still binds fails the
        // whole apply under RESTRICT semantics.
        let current = fixture.objects(PublicExecute::Revoke);
        let _ = fixture.apply(&migrator, &current.manifest).await?;
        let touch = RoutineSignature::new(&schema, "touch_updated_at", [])?;
        let blocked = fixture.objects_with(
            PublicExecute::Revoke,
            current
                .manifest
                .objects()
                .iter()
                .filter(|object| object.identity() != touch.as_str())
                .cloned(),
            [Retirement::function(touch.clone())],
        )?;
        let failing_pid = pid(&migrator).await?;
        let error = fixture
            .apply(&migrator, &blocked.manifest)
            .await
            .expect_err("dropping a routine a trigger still uses must fail");
        require(
            matches!(error, ManagedApplyError::Statement(_))
                && sqlstate(&error).as_deref() == Some("2BP01"),
            "dependent trigger did not block the retirement",
        )?;
        require(
            pid(&migrator).await? != failing_pid,
            "failed apply returned its session to the pool",
        )?;
        require(
            fixture.exists(&migrator, "touch_updated_at").await?,
            "failed apply dropped the routine",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&current.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "failed apply changed the installed set",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_public_execute_policy_is_applied_and_verified() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        let grant = format!("GRANT EXECUTE ON FUNCTION {schema}.{RECORD_TOTAL} TO PUBLIC");
        fixture.admin(&grant).await?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        let identity = RoutineSignature::new(
            &schema,
            "record_total",
            [RoutineType::new("pg_catalog", "int8")?],
        )?;
        require(
            report.status() == VerificationStatus::Violations
                && report.findings().len() == 1
                && report.findings()[0].kind == FindingKind::ManagedObjectPublicExecute
                && report.findings()[0].object.as_deref() == Some(identity.as_str())
                && report.findings()[0].subject.as_deref() == Some("public")
                && report.findings()[0].privilege == Some(ObjectPrivilege::Execute),
            "PUBLIC EXECUTE on a revoked routine was not reported",
        )?;
        let again = fixture.apply(&migrator, &objects.manifest).await?;
        require(
            again.created_or_replaced() == 0,
            "an ACL change re-executed a definition",
        )?;
        require(
            !fixture.public_can_execute(&migrator, RECORD_TOTAL).await?,
            "apply did not revoke PUBLIC EXECUTE again",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "revoked PUBLIC EXECUTE did not verify",
        )?;

        // An unmanaged policy neither revokes nor checks PUBLIC EXECUTE.
        fixture.admin(&grant).await?;
        let unmanaged = fixture.objects(PublicExecute::Unmanaged);
        let _ = fixture.apply(&migrator, &unmanaged.manifest).await?;
        require(
            fixture.public_can_execute(&migrator, RECORD_TOTAL).await?,
            "an unmanaged policy revoked PUBLIC EXECUTE",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&unmanaged.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "an unmanaged PUBLIC policy was checked",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_out_of_band_drift_is_reported_and_repaired() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        fixture
            .admin(&format!(
                "CREATE OR REPLACE FUNCTION {schema}.record_total(extra bigint) RETURNS bigint
                 LANGUAGE sql STABLE AS $$ SELECT extra + 1 $$"
            ))
            .await?;
        fixture
            .admin(&format!("DROP TRIGGER records_touch ON {schema}.records"))
            .await?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        let mut drifted = report
            .findings()
            .iter()
            .filter(|finding| finding.kind == FindingKind::ManagedObjectDrift)
            .filter_map(|finding| finding.object.clone())
            .collect::<Vec<_>>();
        drifted.sort();
        let mut expected = vec![
            RoutineSignature::new(
                &schema,
                "record_total",
                [RoutineType::new("pg_catalog", "int8")?],
            )?
            .as_str()
            .to_owned(),
            format!("\"records_touch\" ON \"{schema}\".\"records\""),
        ];
        expected.sort();
        require(
            report.status() == VerificationStatus::Violations
                && report.findings().len() == 2
                && drifted == expected,
            "out-of-band definition changes were not reported as drift",
        )?;
        let repaired = fixture.apply(&migrator, &objects.manifest).await?;
        require(
            repaired.created_or_replaced() == 2 && repaired.unchanged() == 2,
            "apply did not replace exactly the drifted objects",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "repaired drift did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_missing_ledger_is_a_finding() -> Result {
    Fixture::run(async |fixture| {
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        fixture.migrate_only(&migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let sqlx = fixture.sqlx_manifest()?;
        let plan =
            VerificationPlan::sqlx_migrations(&sqlx).with_managed_objects(&objects.manifest)?;
        let report = fixture.verify(&reader, plan).await?;
        let ledger = format!("\"{}\".\"managed_objects\"", fixture.schema());
        require(
            report.status() == VerificationStatus::Violations
                && report.findings().len() == 1
                && report.findings()[0].kind == FindingKind::MissingManagedObjectLedger
                && report.findings()[0].object.as_deref() == Some(ledger.as_str()),
            "a migrated database without managed objects was not one finding",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_reject_undeclared_identities_and_foreign_ledgers() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let migrator = fixture.pool(Role::Migrator).await?;
        let total = RoutineSignature::new(
            &schema,
            "record_total",
            [RoutineType::new("pg_catalog", "int8")?],
        )?;
        // The header names the declared routine, but its arguments create
        // another overload than the declared signature.
        let mismatched = fixture.objects_with(
            PublicExecute::Revoke,
            [ManagedObject::function(
                total.clone(),
                format!(
                    "CREATE OR REPLACE FUNCTION {schema}.record_total(amount integer)
                     RETURNS bigint LANGUAGE sql STABLE AS $$ SELECT amount::bigint $$;"
                ),
            )?],
            [],
        )?;
        let failing_pid = pid(&migrator).await?;
        let error = fixture
            .apply(&migrator, &mismatched.manifest)
            .await
            .expect_err("another overload than declared must fail");
        require(
            matches!(&error, ManagedApplyError::IdentityMismatch { identity }
                if identity == total.as_str()),
            "an undeclared overload was accepted",
        )?;
        require(
            pid(&migrator).await? != failing_pid,
            "failed apply returned its session to the pool",
        )?;
        require(
            !fixture.exists(&migrator, "record_total").await?
                && !fixture
                    .relation_exists(&migrator, "managed_objects")
                    .await?,
            "a failed apply left objects or a ledger behind",
        )?;
        require(
            fixture.relation_exists(&migrator, "records").await?,
            "migrations committed before the failed apply were lost",
        )?;

        // A SQL body that references a column the migrations never created.
        let invalid = fixture.objects_with(
            PublicExecute::Revoke,
            [ManagedObject::function(
                total,
                format!(
                    "CREATE OR REPLACE FUNCTION {schema}.record_total(bigint) RETURNS bigint
                     LANGUAGE sql STABLE AS $$ SELECT sum(missing_column) FROM {schema}.records $$;"
                ),
            )?],
            [],
        )?;
        let error = fixture
            .apply(&migrator, &invalid.manifest)
            .await
            .expect_err("a body invalid against the migrated catalog must fail");
        require(
            sqlstate(&error).as_deref() == Some("42703"),
            "SQL body was not validated against the migrated catalog",
        )?;

        // A ledger name already used by another relation is refused by shape.
        let foreign = fixture.objects_in_ledger(PublicExecute::Revoke, "schema_history")?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE {schema}.schema_history (version bigint PRIMARY KEY)"
        )))
        .execute(&migrator)
        .await?;
        let error = fixture
            .apply(&migrator, &foreign.manifest)
            .await
            .expect_err("a foreign ledger relation must be refused");
        require(
            matches!(error, ManagedApplyError::LedgerShape),
            "a foreign ledger relation was not refused by shape",
        )?;
        require(
            !fixture.exists(&migrator, "touch_updated_at").await?,
            "a refused ledger still applied objects",
        )?;

        // With the declared overload already installed, a changed body that
        // creates another overload must not pass because the declared one exists.
        let installed = fixture.objects(PublicExecute::Revoke);
        let _ = fixture.apply(&migrator, &installed.manifest).await?;
        let catalog = catalog_state(&migrator, &schema).await?;
        let error = fixture
            .apply(&migrator, &mismatched.manifest)
            .await
            .expect_err("an untouched pre-existing overload must not satisfy its entry");
        require(
            matches!(error, ManagedApplyError::IdentityMismatch { .. }),
            "a pre-existing overload satisfied a statement that created another",
        )?;
        require(
            catalog_state(&migrator, &schema).await? == catalog,
            "a rejected apply changed the catalog",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_disabled_trigger_is_drift_and_reenabled() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        // pg_get_triggerdef does not render whether a trigger fires.
        fixture
            .admin(&format!(
                "ALTER TABLE {schema}.records DISABLE TRIGGER records_touch"
            ))
            .await?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        let trigger = format!("\"records_touch\" ON \"{schema}\".\"records\"");
        require(
            report.status() == VerificationStatus::Violations
                && report.findings().len() == 1
                && report.findings()[0].kind == FindingKind::ManagedObjectDrift
                && report.findings()[0].object.as_deref() == Some(trigger.as_str()),
            "a disabled managed trigger was not reported as drift",
        )?;
        let repaired = fixture.apply(&migrator, &objects.manifest).await?;
        require(
            repaired.created_or_replaced() == 1,
            "apply did not replace exactly the disabled trigger",
        )?;
        let enabled: String = sqlx::query_scalar(
            "SELECT t.tgenabled::text FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND t.tgname = 'records_touch'",
        )
        .bind(&schema)
        .fetch_one(&migrator)
        .await?;
        require(enabled == "O", "apply did not re-enable the trigger")?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "a re-enabled trigger did not verify",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_public_execute_from_another_grantor_fails_apply() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let delegate = fixture.delegate().to_owned();
        let objects = fixture.objects(PublicExecute::Revoke);
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let _ = fixture.apply(&migrator, &objects.manifest).await?;
        // The owner's revoke cannot remove a grant another grantor made.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "GRANT EXECUTE ON FUNCTION {schema}.{RECORD_TOTAL} TO {delegate} WITH GRANT OPTION"
        )))
        .execute(&migrator)
        .await?;
        fixture
            .admin(&format!(
                "SET ROLE {delegate};
                 GRANT EXECUTE ON FUNCTION {schema}.{RECORD_TOTAL} TO PUBLIC;
                 RESET ROLE"
            ))
            .await?;
        let failing_pid = pid(&migrator).await?;
        let error = fixture
            .apply(&migrator, &objects.manifest)
            .await
            .expect_err("a retained PUBLIC grant must fail a revoking apply");
        let identity = RoutineSignature::new(
            &schema,
            "record_total",
            [RoutineType::new("pg_catalog", "int8")?],
        )?;
        require(
            matches!(&error, ManagedApplyError::PublicExecuteRetained { identity: retained }
                if retained == identity.as_str()),
            "apply committed while PUBLIC kept EXECUTE",
        )?;
        require(
            pid(&migrator).await? != failing_pid,
            "failed apply returned its session to the pool",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        require(
            report
                .findings()
                .iter()
                .all(|finding| finding.kind == FindingKind::ManagedObjectPublicExecute)
                && report.findings().len() == 1,
            "verification did not report exactly the retained PUBLIC grant",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_objects_retirement_of_another_kind_is_left_alone() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        fixture.migrate_only(&migrator).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE PROCEDURE {schema}.legacy() LANGUAGE sql BEGIN ATOMIC SELECT 1; END"
        )))
        .execute(&migrator)
        .await?;
        // The retired function does not exist; the procedure of the same
        // identity is a different object and must survive.
        let objects = fixture.objects_with(
            PublicExecute::Revoke,
            fixture.standard_objects(),
            [Retirement::function(RoutineSignature::new(
                &schema,
                "legacy",
                [],
            )?)],
        )?;
        let applied = fixture.apply(&migrator, &objects.manifest).await?;
        require(
            applied.retired() == 0,
            "a procedure was retired as a function",
        )?;
        require(
            fixture.exists(&migrator, "legacy").await?,
            "the procedure was dropped",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&objects.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "a procedure was reported as the retired function",
        )
    })
    .await
}
