//! Application authority and definitions become visible at the same commit.

use super::{Fixture, Role, catalog_state, pid};
use crate::{
    managed_objects_fixture::context,
    support::{Result, bounded, require},
};
use batter_sqlx::{
    PgLease, PgScopedSql,
    managed::{ManagedApplyWithError, ManagedObject, ManagedObjectManifest, PublicExecute},
    verification::{RoutineSignature, VerificationPlan, VerificationStatus},
};
use sqlx::{AssertSqlSafe, PgPool};

#[path = "application_failures.rs"]
mod failures;

pub(super) async fn apply(
    fixture: &Fixture,
    pool: &PgPool,
    manifest: &ManagedObjectManifest,
    work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> std::result::Result<(), sqlx::Error>,
) -> std::result::Result<
    batter_sqlx::managed::AppliedManagedObjects,
    ManagedApplyWithError<sqlx::Error>,
> {
    let lease = PgLease::acquire(pool, &context())
        .await
        .expect("fixture lease");
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        lease.migrate_and_apply_with(&fixture.migrator_bundle(), manifest, work),
    )
    .await
    .expect("bounded apply")
}

fn function(fixture: &Fixture, name: &str, body: &str) -> Result<ManagedObject> {
    Ok(ManagedObject::function(
        RoutineSignature::new(fixture.schema(), name, [])?,
        format!(
            "CREATE OR REPLACE FUNCTION {}.{name}() RETURNS bigint
            LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
            AS $$ BEGIN {body} END $$",
            fixture.schema()
        ),
    )?)
}

async fn ledger(pool: &PgPool, schema: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT row_to_json(m)::text FROM {schema}.managed_objects m ORDER BY identity"
    )))
    .fetch_all(pool)
    .await?)
}

async fn exact_authority(
    fixture: &Fixture,
    reader: &PgPool,
    schema: &str,
    owner: &str,
    serving: &str,
) -> Result {
    let actual_owner: String = sqlx::query_scalar(
        "SELECT pg_get_userbyid(proowner)::text FROM pg_proc WHERE oid=$1::regprocedure",
    )
    .bind(format!("{schema}.private_total()"))
    .fetch_one(reader)
    .await?;
    require(actual_owner == owner, "helper ownership not published")?;
    require(
        !fixture
            .public_can_execute(reader, "private_total()")
            .await?,
        "PUBLIC gained helper",
    )?;
    let grants: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege($1,$2,'EXECUTE'),
                has_table_privilege($3,$4,'SELECT'), has_table_privilege($3,$4,'INSERT')",
    )
    .bind(serving)
    .bind(format!("{schema}.private_total()"))
    .bind(owner)
    .bind(format!("{schema}.records"))
    .fetch_one(reader)
    .await?;
    require(
        grants == (true, true, false),
        "precise owner/consumer grants differ",
    )?;
    let safe: bool = sqlx::query_scalar(
        "SELECT NOT (rolcanlogin OR rolsuper OR rolbypassrls) FROM pg_roles WHERE rolname=$1",
    )
    .bind(owner)
    .fetch_one(reader)
    .await?;
    require(safe, "helper owner is not restricted")?;
    let overload: (String, bool) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {schema}.report_total('public'::text), proacl IS NULL
         FROM pg_proc WHERE oid = '{schema}.report_total(text)'::regprocedure"
    )))
    .fetch_one(reader)
    .await?;
    require(
        overload == ("public".to_owned(), true),
        "unmanaged overload or its default PUBLIC ACL changed",
    )?;
    Ok(())
}

async fn install_original_caller(fixture: &mut Fixture, pool: &PgPool, serving: &str) -> Result {
    let schema = fixture.schema();
    let owner = fixture.delegate();
    let setup = fixture.migrator();
    let original = fixture.objects_with(
        PublicExecute::Revoke,
        [function(fixture, "report_total", "RETURN 1;")?],
        [],
    )?;
    let _ = fixture.apply(pool, &original.manifest).await?;
    fixture
        .admin(&format!(
            "GRANT {owner}, {serving} TO {setup};
         GRANT CREATE ON SCHEMA {schema} TO {serving};
         ALTER FUNCTION {schema}.report_total() OWNER TO {serving};
         REVOKE CREATE ON SCHEMA {schema} FROM {serving};
         CREATE FUNCTION {schema}.report_total(value text) RETURNS text
             LANGUAGE sql RETURN value;
         INSERT INTO {schema}.records(note,amount) VALUES ('seed',7)"
        ))
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_application_sql_publishes_owner_and_grants_atomically() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let owner = fixture.delegate().to_owned();
        let migrator = fixture.pool(Role::Migrator).await?;
        let reader = fixture.pool(Role::Reader).await?;
        let observer = fixture.pool(Role::Migrator).await?;
        let serving: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&reader)
            .await?;
        install_original_caller(fixture, &migrator, &serving).await?;
        let changed = fixture.objects_with(
            PublicExecute::Revoke,
            [
                function(
                    fixture,
                    "private_total",
                    &format!("RETURN (SELECT sum(amount) FROM {schema}.records);"),
                )?,
                function(
                    fixture,
                    "report_total",
                    &format!("RETURN {schema}.private_total();"),
                )?,
            ],
            [],
        )?;
        let before_pid = pid(&migrator).await?;
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let applying = apply(fixture, &migrator, &changed.manifest, async |sql| {
            sqlx::raw_sql(AssertSqlSafe(format!(
                "GRANT CREATE ON SCHEMA {schema} TO {owner};
                 ALTER FUNCTION {schema}.private_total() OWNER TO {owner};
                 REVOKE CREATE ON SCHEMA {schema} FROM {owner};
                 GRANT SELECT ON {schema}.records TO {owner};
                 GRANT EXECUTE ON FUNCTION {schema}.private_total() TO {serving}"
            )))
            .execute(sql.executor())
            .await?;
            let _ = entered_tx.send(());
            release_rx.await.expect("observer releases apply");
            Ok(())
        });
        let observing = async {
            entered_rx.await?;
            let result = async {
                let total: i64 = bounded(
                    sqlx::query_scalar(AssertSqlSafe(format!("SELECT {schema}.report_total()")))
                        .fetch_one(&reader),
                )
                .await??;
                require(total == 1, "new caller escaped before authority commit")?;
                require(
                    !fixture.exists(&observer, "private_total").await?,
                    "new helper visible early",
                )
            }
            .await;
            let _ = release_tx.send(());
            result
        };
        let (applied, observed) = tokio::join!(applying, observing);
        observed?;
        let applied = applied?;
        require(
            applied.digest() == changed.manifest.digest(),
            "wrong committed manifest",
        )?;
        let total: i64 =
            sqlx::query_scalar(AssertSqlSafe(format!("SELECT {schema}.report_total()")))
                .fetch_one(&reader)
                .await?;
        require(total == 7, "restricted caller could not execute new helper")?;
        exact_authority(fixture, &reader, &schema, &owner, &serving).await?;
        require(
            pid(&migrator).await? != before_pid,
            "callback session was pooled",
        )?;
        let report = fixture
            .verify(
                &reader,
                VerificationPlan::managed_objects(&changed.manifest),
            )
            .await?;
        require(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "granted set failed verification",
        )
    })
    .await
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_application_refusal_rolls_back_objects_ledger_and_grants() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let delegate = fixture.delegate().to_owned();
        let pool = fixture.pool(Role::Migrator).await?;
        let original = fixture.objects(PublicExecute::Revoke);
        // First refusal: a fresh database keeps the committed migration only.
        let error = apply(fixture, &pool, &original.manifest, async |sql| {
            sqlx::raw_sql(AssertSqlSafe(format!(
                "GRANT SELECT ON {schema}.records TO {delegate}"
            )))
            .execute(sql.executor())
            .await?;
            Err(sqlx::Error::Protocol("private-refusal-marker".into()))
        })
        .await
        .unwrap_err();
        require(
            matches!(error, ManagedApplyWithError::Application(_)),
            "refusal lost its classification",
        )?;
        require(
            !format!("{error:?} {error}").contains("private-refusal-marker"),
            "refusal leaked",
        )?;
        require(
            fixture.relation_exists(&pool, "records").await?,
            "committed migration rolled back",
        )?;
        require(
            !fixture.relation_exists(&pool, "managed_objects").await?,
            "fresh ledger escaped rollback",
        )?;
        require(
            !fixture.exists(&pool, "record_total").await?,
            "fresh definition escaped rollback",
        )?;
        let granted: bool = sqlx::query_scalar("SELECT has_table_privilege($1,$2,'SELECT')")
            .bind(&delegate)
            .bind(format!("{schema}.records"))
            .fetch_one(&pool)
            .await?;
        require(!granted, "grant escaped rollback")?;
        let _ = fixture.apply(&pool, &original.manifest).await?;
        let before = catalog_state(&pool, &schema).await?;
        let before_ledger = ledger(&pool, &schema).await?;
        let mut objects = fixture.standard_objects();
        objects.push(function(fixture, "private_total", "RETURN 7;")?);
        let changed = fixture.objects_with(PublicExecute::Revoke, objects, [])?;
        // Native statement failure leaves the callback transaction aborted; the
        // existing guard must recover it before classifying application refusal.
        let error = apply(fixture, &pool, &changed.manifest, async |sql| {
            sqlx::query("SELECT 1 / 0").execute(sql.executor()).await?;
            Ok(())
        })
        .await
        .unwrap_err();
        let ManagedApplyWithError::Application(error) = error else {
            return Err(std::io::Error::other("native callback refusal became uncertainty").into());
        };
        require(
            error.as_database_error().and_then(|e| e.code()).as_deref() == Some("22012"),
            "native cause lost",
        )?;
        require(
            catalog_state(&pool, &schema).await? == before,
            "rejection changed definitions",
        )?;
        require(
            ledger(&pool, &schema).await? == before_ledger,
            "rejection changed ledger",
        )
    })
    .await
}
