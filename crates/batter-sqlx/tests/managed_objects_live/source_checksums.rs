//! Switching a set to `SourceChecksum::FromStatementStart` changes only its
//! recorded source checksums, for every managed kind; then its leading text
//! is free to edit.

use super::constraint_triggers::verifies;
use super::{Fixture, Role, catalog_state};
use crate::support::{Result, require};
use batter_sqlx::managed::{ManagedObject, PublicExecute, SourceChecksum};
use batter_sqlx::verification::{Identifier, QualifiedName, RoutineSignature, RoutineType};
use sqlx::{AssertSqlSafe, PgPool};

const HEADER: &str = "-- Reviewed; with a semicolon.\n/* block /* nested */ comment */\n\n";
const REWORDED: &str = "/* Reworded. */\n";

/// One entry per kind, with a string body and a SQL-standard body among the
/// routines. Each source is `leading` followed by its statement.
fn objects(schema: &str, leading: &str) -> Result<Vec<ManagedObject>> {
    let source = |statement: String| -> Result<String> { Ok(format!("{leading}{statement}")) };
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

/// The deparsed definition, `prosrc` and `tgenabled` of every routine and
/// trigger in `schema`, without the row identities a re-execution changes. A
/// deparse omits parse locations, so equal results do not show that stored
/// node trees are byte-equal.
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
async fn managed_statement_start_checksums_change_no_installed_definition() -> Result {
    Fixture::run(async |fixture| {
        let schema = fixture.schema().to_owned();
        let set = |leading, rule| -> Result<_> {
            Ok(fixture
                .objects_under(PublicExecute::Revoke, rule, objects(&schema, leading)?, [])?
                .manifest)
        };
        let whole = set(HEADER, SourceChecksum::WholeSource)?;
        let sliced = set(HEADER, SourceChecksum::FromStatementStart)?;
        let reworded = set(REWORDED, SourceChecksum::FromStatementStart)?;
        require!(
            whole.digest() != sliced.digest() && sliced.digest() == reworded.digest(),
            "leading text entered a statement-start digest or left a whole one",
        )?;
        let pool = fixture.pool(Role::Migrator).await?;
        require!(
            fixture.apply(&pool, &whole).await?.created_or_replaced() == 5,
            "whole sources were not installed",
        )?;
        let whole_definitions = definitions(&pool, &schema).await?;
        let whole_rows = checksums(&pool, &schema).await?;
        require!(
            whole_definitions.len() == 5 && whole_rows.len() == 6,
            "unexpected fixture state",
        )?;

        // Every source has leading text, so switching the rule changes every
        // checksum and executes every entry again, still from its whole
        // source. What PostgreSQL stores does not change.
        let applied = fixture.apply(&pool, &sliced).await?;
        require!(
            applied.created_or_replaced() == 5 && applied.unchanged() == 0,
            "switching the rule did not re-execute every entry",
        )?;
        require!(
            definitions(&pool, &schema).await? == whole_definitions,
            "switching the rule installed a different definition",
        )?;
        let sliced_rows = checksums(&pool, &schema).await?;
        require!(
            sliced_rows.len() == whole_rows.len()
                && sliced_rows.iter().zip(&whole_rows).all(
                    |((identity, source, rendered), (was_identity, was_source, was_rendered))| {
                        identity == was_identity && source != was_source && rendered == was_rendered
                    }
                ),
            "the rule changed a rendered checksum or kept a source checksum",
        )?;
        let reader = fixture.pool(Role::Reader).await?;
        require!(
            verifies(fixture, &reader, &sliced).await?,
            "statement-start checksums did not verify",
        )?;

        // Rewording the leading text is then no set change:
        // every ledger row keeps its checksums, and no object row is rewritten.
        let catalog = catalog_state(&pool, &schema).await?;
        let object_rows = fixture.object_rows(&pool).await?;
        let applied = fixture.apply(&pool, &reworded).await?;
        require!(
            applied.created_or_replaced() == 0 && applied.unchanged() == 5,
            "a reworded header re-executed an entry",
        )?;
        require!(
            catalog_state(&pool, &schema).await? == catalog
                && checksums(&pool, &schema).await? == sliced_rows
                && fixture.object_rows(&pool).await? == object_rows,
            "a reworded header changed the catalog or a ledger row",
        )
    })
    .await
}
