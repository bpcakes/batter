use super::{
    InspectionFragment, PgTransaction, VerificationError,
    migration::{self, LedgerLock, ProtectedRows},
    policy::MAX_MIGRATION_LEDGER_ROWS,
    report::{Finding, FindingKind, SupportedSurface, UnsupportedSurface},
    request::{SqlxLedgerMode, SqlxMigrationManifest},
};
use std::collections::{HashMap, HashSet};

type LedgerRow = (i64, Vec<u8>, bool);
type ColumnShape = HashMap<String, (String, String, bool)>;

pub(super) async fn inspect(
    transaction: &mut PgTransaction<'_>,
    manifest: &SqlxMigrationManifest,
    lock: LedgerLock,
) -> Result<InspectionFragment, VerificationError> {
    let ledger = manifest.ledger();
    let mut fragment = InspectionFragment {
        supported: vec![SupportedSurface::SqlxMigrationLedger],
        evaluated_items: manifest.expected().len(),
        ..InspectionFragment::default()
    };
    let protected = match lock {
        LedgerLock::Ready(protected) => protected,
        LedgerLock::Missing => {
            if ledger_exists(transaction, manifest).await? {
                fragment
                    .unsupported
                    .push(UnsupportedSurface::UnprotectedMigrationLedger);
            } else {
                inspect_missing_ledger(manifest, &mut fragment.findings);
            }
            fragment.evaluated_items = fragment.evaluated_items.saturating_add(1);
            return Ok(fragment);
        }
        LedgerLock::WrongKind => {
            fragment.findings.push(Finding::new(
                FindingKind::SqlxLedgerShape,
                Some(ledger.quoted()),
                None::<String>,
                None,
            ));
            return Ok(fragment);
        }
        LedgerLock::Unprotected => {
            fragment
                .unsupported
                .push(UnsupportedSurface::UnprotectedMigrationLedger);
            return Ok(fragment);
        }
    };
    let metadata = sqlx::query_as::<_, (String, bool)>(
        "SELECT c.relkind::text, c.relrowsecurity
         FROM pg_catalog.pg_class AS c
         WHERE c.oid = $1::bigint::oid",
    )
    .bind(protected.oid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    fragment.evaluated_items = fragment.evaluated_items.saturating_add(1);

    let Some((kind, row_security)) = metadata else {
        fragment
            .unsupported
            .push(UnsupportedSurface::UnprotectedMigrationLedger);
        return Ok(fragment);
    };
    if kind != "r" || row_security {
        fragment.findings.push(Finding::new(
            if row_security {
                FindingKind::LedgerRowSecurity
            } else {
                FindingKind::SqlxLedgerShape
            },
            Some(ledger.quoted()),
            None::<String>,
            None,
        ));
        return Ok(fragment);
    }
    if !shape_is_exact(
        transaction,
        manifest,
        protected.oid(),
        &mut fragment.findings,
        &mut fragment.evaluated_items,
    )
    .await?
    {
        return Ok(fragment);
    }
    let rows = match migration::load_protected_rows(transaction, protected).await? {
        ProtectedRows::Rows(rows) => rows,
        ProtectedRows::Unprotected => {
            fragment
                .unsupported
                .push(UnsupportedSurface::UnprotectedMigrationLedger);
            return Ok(fragment);
        }
    };
    fragment.evaluated_items = fragment.evaluated_items.saturating_add(rows.len());
    if rows.len() > MAX_MIGRATION_LEDGER_ROWS {
        fragment.findings.push(Finding::new(
            FindingKind::MigrationLedgerLimit,
            Some(ledger.quoted()),
            None::<String>,
            None,
        ));
        return Ok(fragment);
    }
    compare_rows(manifest, &rows, &mut fragment.findings);
    Ok(fragment)
}

async fn ledger_exists(
    transaction: &mut PgTransaction<'_>,
    manifest: &SqlxMigrationManifest,
) -> Result<bool, VerificationError> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pg_catalog.pg_class AS c
            JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = $2)",
    )
    .bind(manifest.ledger().schema())
    .bind(manifest.ledger().name())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
}

fn inspect_missing_ledger(manifest: &SqlxMigrationManifest, findings: &mut Vec<Finding>) {
    if manifest.mode() == SqlxLedgerMode::Exact {
        findings.push(Finding::new(
            FindingKind::MissingMigration,
            Some(manifest.ledger().quoted()),
            None::<String>,
            None,
        ));
    }
}

async fn shape_is_exact(
    transaction: &mut PgTransaction<'_>,
    manifest: &SqlxMigrationManifest,
    oid: i64,
    findings: &mut Vec<Finding>,
    evaluated_items: &mut usize,
) -> Result<bool, VerificationError> {
    let columns = sqlx::query_as::<_, (String, String, String, bool)>(
        "SELECT a.attname::text, n.nspname::text, t.typname::text, a.attnotnull
         FROM pg_catalog.pg_attribute AS a
         JOIN pg_catalog.pg_type AS t ON t.oid = a.atttypid
         JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
         WHERE a.attrelid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped
         LIMIT 10001",
    )
    .bind(oid)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    if columns.len() > 10_000 {
        return Err(VerificationError::CatalogCapacity);
    }
    *evaluated_items = evaluated_items.saturating_add(columns.len());
    let actual = columns
        .into_iter()
        .map(|(name, schema, ty, not_null)| (name, (schema, ty, not_null)))
        .collect::<HashMap<_, _>>();
    let primary_keys = sqlx::query_scalar::<_, Vec<String>>(
        "SELECT ARRAY(
             SELECT a.attname::text
             FROM unnest(c.conkey) WITH ORDINALITY AS key(attnum, position)
             JOIN pg_catalog.pg_attribute AS a
               ON a.attrelid = c.conrelid AND a.attnum = key.attnum
             ORDER BY key.position)
         FROM pg_catalog.pg_constraint AS c
         WHERE c.conrelid = $1::oid AND c.contype = 'p'
         LIMIT 2",
    )
    .bind(oid)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    *evaluated_items = evaluated_items.saturating_add(primary_keys.len());
    if shape_matches(&actual, &primary_keys) {
        return Ok(true);
    }
    findings.push(Finding::new(
        FindingKind::SqlxLedgerShape,
        Some(manifest.ledger().quoted()),
        None::<String>,
        None,
    ));
    Ok(false)
}

fn shape_matches(actual: &ColumnShape, primary_keys: &[Vec<String>]) -> bool {
    let expected = [
        ("version", "int8"),
        ("description", "text"),
        ("installed_on", "timestamptz"),
        ("success", "bool"),
        ("checksum", "bytea"),
        ("execution_time", "int8"),
    ];
    actual.len() == expected.len()
        && expected.iter().all(|(name, ty)| {
            actual.get(*name).is_some_and(|(schema, actual, not_null)| {
                schema == "pg_catalog" && actual == ty && *not_null
            })
        })
        && primary_keys == [vec!["version".to_owned()]]
}

fn compare_rows(manifest: &SqlxMigrationManifest, rows: &[LedgerRow], findings: &mut Vec<Finding>) {
    let expected = manifest
        .expected()
        .iter()
        .map(|migration| (migration.version, migration))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    for (version, checksum, success) in rows {
        if !seen.insert(*version) {
            findings.push(row_finding(
                manifest,
                FindingKind::DuplicateMigrationRow,
                *version,
            ));
        }
        match expected.get(version) {
            None => findings.push(row_finding(
                manifest,
                FindingKind::UnexpectedMigration,
                *version,
            )),
            Some(expected) => {
                if !success {
                    findings.push(row_finding(
                        manifest,
                        FindingKind::UnsuccessfulMigration,
                        *version,
                    ));
                }
                if expected.checksum != *checksum {
                    findings.push(row_finding(
                        manifest,
                        FindingKind::MigrationChecksumMismatch,
                        *version,
                    ));
                }
            }
        }
    }
    if manifest.mode() == SqlxLedgerMode::Exact {
        for version in expected.keys().filter(|version| !seen.contains(version)) {
            findings.push(row_finding(
                manifest,
                FindingKind::MissingMigration,
                *version,
            ));
        }
    }
}

fn row_finding(manifest: &SqlxMigrationManifest, kind: FindingKind, version: i64) -> Finding {
    Finding::new(
        kind,
        Some(manifest.ledger().quoted()),
        Some(version.to_string()),
        None,
    )
}

#[cfg(test)]
fn column_name(manifest: &SqlxMigrationManifest, column: &str) -> String {
    format!(
        "{}.{}",
        manifest.ledger().quoted(),
        super::policy::quote_identifier(column)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::{MigrationExpectation, QualifiedName};

    fn manifest(mode: SqlxLedgerMode) -> SqlxMigrationManifest {
        SqlxMigrationManifest::new(
            QualifiedName::new("public", "_sqlx_migrations").unwrap(),
            mode,
            [
                MigrationExpectation::new(1, [1]),
                MigrationExpectation::new(3, [3]),
            ],
        )
        .unwrap()
    }

    #[test]
    fn installed_subset_is_set_membership_not_a_prefix() {
        let manifest = manifest(SqlxLedgerMode::InstalledSubset);
        let mut findings = Vec::new();
        compare_rows(&manifest, &[(3, vec![3], true)], &mut findings);
        assert!(findings.is_empty());
    }

    #[test]
    fn installed_subset_rejects_versions_outside_the_expected_set() {
        let manifest = manifest(SqlxLedgerMode::InstalledSubset);
        let mut findings = Vec::new();
        compare_rows(&manifest, &[(2, vec![2], true)], &mut findings);
        assert!(findings.iter().any(|finding| {
            finding.kind == FindingKind::UnexpectedMigration
                && finding.subject.as_deref() == Some("2")
        }));
    }

    #[test]
    fn absent_ledger_is_missing_only_in_exact_mode() {
        let mut exact_findings = Vec::new();
        inspect_missing_ledger(&manifest(SqlxLedgerMode::Exact), &mut exact_findings);
        assert_eq!(exact_findings.len(), 1);
        assert_eq!(exact_findings[0].kind, FindingKind::MissingMigration);

        let mut subset_findings = Vec::new();
        inspect_missing_ledger(
            &manifest(SqlxLedgerMode::InstalledSubset),
            &mut subset_findings,
        );
        assert!(subset_findings.is_empty());
    }

    #[test]
    fn exact_history_requires_all_rows_and_rejects_unknown_rows() {
        let manifest = manifest(SqlxLedgerMode::Exact);
        let mut findings = Vec::new();
        compare_rows(
            &manifest,
            &[(1, vec![1], true), (2, vec![2], true)],
            &mut findings,
        );
        assert!(findings.iter().any(|finding| {
            finding.kind == FindingKind::MissingMigration && finding.subject.as_deref() == Some("3")
        }));
        assert!(findings.iter().any(|finding| {
            finding.kind == FindingKind::UnexpectedMigration
                && finding.subject.as_deref() == Some("2")
        }));
    }

    #[test]
    fn installed_subset_still_rejects_failed_or_mismatched_rows() {
        let manifest = manifest(SqlxLedgerMode::InstalledSubset);
        let mut findings = Vec::new();
        compare_rows(
            &manifest,
            &[(1, vec![9], false), (3, vec![3], true)],
            &mut findings,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::UnsuccessfulMigration)
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::MigrationChecksumMismatch)
        );
    }

    #[test]
    fn sqlx_column_diagnostics_quote_every_component() {
        assert_eq!(
            column_name(&manifest(SqlxLedgerMode::Exact), "version"),
            "\"public\".\"_sqlx_migrations\".\"version\""
        );
    }

    #[test]
    fn sqlx_shape_requires_exact_columns_types_nullability_and_primary_key() {
        let mut columns = [
            ("version", "int8"),
            ("description", "text"),
            ("installed_on", "timestamptz"),
            ("success", "bool"),
            ("checksum", "bytea"),
            ("execution_time", "int8"),
        ]
        .into_iter()
        .map(|(name, ty)| {
            (
                name.to_owned(),
                ("pg_catalog".to_owned(), ty.to_owned(), true),
            )
        })
        .collect::<ColumnShape>();
        let primary = vec![vec!["version".to_owned()]];
        assert!(shape_matches(&columns, &primary));
        columns.insert(
            "extra".to_owned(),
            ("pg_catalog".to_owned(), "int4".to_owned(), true),
        );
        assert!(!shape_matches(&columns, &primary));
        columns.remove("extra");
        columns.get_mut("version").unwrap().0 = "service".to_owned();
        assert!(!shape_matches(&columns, &primary));
        columns.get_mut("version").unwrap().0 = "pg_catalog".to_owned();
        assert!(!shape_matches(
            &columns,
            &[vec!["version".to_owned(), "description".to_owned()]],
        ));
    }
}
