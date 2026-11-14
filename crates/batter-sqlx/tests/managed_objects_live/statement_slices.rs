//! A source sliced by `statement_start` installs the definition its whole text
//! installs, for every managed kind.

use super::{Fixture, Role, catalog_state};
use crate::support::{Result, require};
use batter_sqlx::managed::{ManagedObject, PublicExecute, statement_start};
use batter_sqlx::verification::{
    Identifier, QualifiedName, RoutineSignature, RoutineType, VerificationPlan, VerificationStatus,
};
use sqlx::{AssertSqlSafe, PgPool};

const HEADER: &str = "-- Reviewed; with a semicolon.\n/* block /* nested */ comment */\n\n";
const REWORDED: &str = "/* Reworded. */\n";

/// How a source text reaches its constructor.
#[derive(Clone, Copy)]
enum Passed {
    Whole,
    Sliced,
}

/// One entry per kind, with a string body and a SQL-standard body among the
/// routines. Each source is `leading` followed by its statement.
fn objects(schema: &str, leading: &str, passed: Passed) -> Result<Vec<ManagedObject>> {
    let source = |statement: String| -> Result<String> {
        let whole = format!("{leading}{statement}");
        Ok(match passed {
            Passed::Whole => whole,
            Passed::Sliced => statement_start(&whole)?.to_owned(),
        })
    };
    let table = || QualifiedName::new(schema, "records");
    Ok(vec![
        ManagedObject::function(
            RoutineSignature::new(schema, "touch_updated_at", [])?,
            source(format!(
                "CREATE OR REPLACE FUNCTION {schema}.touch_updated_at() RETURNS trigger
                 LANGUAGE plpgsql AS $$ BEGIN -- stored with the body
                 NEW.updated_at := now(); RETURN NEW; END $$;"
            ))?,
        )?,
        ManagedObject::function(
            RoutineSignature::new(schema, "doubled", [RoutineType::new("pg_catalog", "int8")?])?,
            source(format!(
                "CREATE OR REPLACE FUNCTION {schema}.doubled(amount bigint) RETURNS bigint
                 LANGUAGE sql /* not stored */ RETURN amount * 2;"
            ))?,
        )?,
        ManagedObject::procedure(
            RoutineSignature::new(schema, "archive_records", [])?,
            source(format!(
                "CREATE OR REPLACE PROCEDURE {schema}.archive_records() LANGUAGE sql
                 BEGIN ATOMIC DELETE FROM {schema}.records WHERE amount < 0; END;"
            ))?,
        )?,
        ManagedObject::trigger(
            table()?,
            Identifier::new("records_touch")?,
            source(format!(
                "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON {schema}.records
                 FOR EACH ROW EXECUTE FUNCTION {schema}.touch_updated_at();"
            ))?,
        )?,
        ManagedObject::constraint_trigger(
            table()?,
            Identifier::new("records_check")?,
            source(format!(
                "CREATE CONSTRAINT TRIGGER records_check AFTER INSERT OR UPDATE ON {schema}.records
                 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
                 EXECUTE FUNCTION {schema}.touch_updated_at();"
            ))?,
        )?,
    ])
}

/// What PostgreSQL stored for every routine and trigger in `schema`, without
/// the row identities a re-execution changes.
async fn definitions(pool: &PgPool, schema: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT pg_get_functiondef(p.oid) || '|' || p.prosrc
         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = $1
         UNION ALL
         SELECT pg_get_triggerdef(t.oid) || '|' || t.tgenabled::text
         FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND NOT t.tgisinternal
         ORDER BY 1",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?)
}

/// Every ledger row, the set row included: identity, source and rendered checksum.
async fn checksums(pool: &PgPool, schema: &str) -> Result<Vec<(String, Vec<u8>, Vec<u8>)>> {
    Ok(sqlx::query_as(AssertSqlSafe(format!(
        "SELECT identity, source_checksum, rendered_checksum
         FROM {schema}.managed_objects ORDER BY identity"
    )))
    .fetch_all(pool)
    .await?)
}

#[tokio::test]
#[ignore = "requires external PostgreSQL; scripts/test_sqlx_live.sh"]
async fn managed_statement_slices_install_what_their_whole_sources_install() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let set = |leading, passed| -> Result<_> {
            Ok(fixture
                .objects_with(
                    PublicExecute::Revoke,
                    objects(&schema, leading, passed)?,
                    [],
                )?
                .manifest)
        };
        let whole = set(HEADER, Passed::Whole)?;
        let sliced = set(HEADER, Passed::Sliced)?;
        let reworded = set(REWORDED, Passed::Sliced)?;
        require!(
            whole.digest() != sliced.digest() && sliced.digest() == reworded.digest(),
            "leading text entered a sliced digest or left a whole one",
        )?;
        let pool = fixture.pool(Role::Migrator).await?;
        require!(
            fixture.apply(&pool, &whole).await?.created_or_replaced() == 5,
            "whole sources were not installed",
        )?;
        let installed = (
            definitions(&pool, &schema).await?,
            checksums(&pool, &schema).await?,
        );
        require!(
            installed.0.len() == 5 && installed.1.len() == 6,
            "unexpected fixture state",
        )?;

        // A slice is another source text, so starting to pass slices executes
        // every entry again. What PostgreSQL stores does not change.
        let applied = fixture.apply(&pool, &sliced).await?;
        require!(
            applied.created_or_replaced() == 5 && applied.unchanged() == 0,
            "adopting slices did not re-execute every entry",
        )?;
        require!(
            definitions(&pool, &schema).await? == installed.0,
            "a sliced source installed a different definition",
        )?;
        let after = checksums(&pool, &schema).await?;
        require!(
            after.len() == installed.1.len()
                && after.iter().zip(&installed.1).all(|(sliced, whole)| {
                    sliced.0 == whole.0 && sliced.1 != whole.1 && sliced.2 == whole.2
                }),
            "a slice changed a rendered checksum or kept a source checksum",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        let report = fixture
            .verify(&reader, VerificationPlan::managed_objects(&sliced))
            .await?;
        require!(
            report.status() == VerificationStatus::WithinDeclaredPolicy,
            "sliced sources did not verify",
        )?;

        // Rewording the leading text of sliced sources is then no set change.
        let catalog = catalog_state(&pool, &schema).await?;
        let applied = fixture.apply(&pool, &reworded).await?;
        require!(
            applied.created_or_replaced() == 0 && applied.unchanged() == 5,
            "a reworded header re-executed a sliced entry",
        )?;
        require!(
            catalog_state(&pool, &schema).await? == catalog
                && checksums(&pool, &schema).await?[1..] == after[1..],
            "a reworded header changed the catalog or an object row",
        )
    })
    .await
}
