use super::policy::{
    AdditionalMigrations, MAX_MIGRATION_CHECKSUM_BYTES, MAX_MIGRATION_LEDGER_ROWS,
    MigrationExpectation, MigrationPolicy, quote_identifier,
};
use super::report::{Finding, FindingKind};
use super::{PgTransaction, VerificationError};
use std::collections::{HashMap, HashSet};

type MigrationRow<'a> = &'a (i64, Vec<u8>, bool);

/// Inheritance membership is read from the transaction snapshot, unlike the
/// planner's descendant expansion. Only ordinary standalone ledgers are covered.
pub(super) async fn has_inheritance(
    transaction: &mut PgTransaction<'_>,
    policy: &MigrationPolicy,
) -> Result<bool, VerificationError> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pg_catalog.pg_class AS c
            JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
            JOIN pg_catalog.pg_inherits AS i
              ON i.inhparent = c.oid OR i.inhrelid = c.oid
            WHERE n.nspname = $1 AND c.relname = $2)",
    )
    .bind(policy.ledger.schema())
    .bind(policy.ledger.name())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
}

/// Result of protecting the ledger before the transaction's first snapshot.
pub(super) enum LedgerLock {
    Ready,
    Missing,
    WrongKind,
}

pub(super) async fn lock_before_snapshot(
    transaction: &mut PgTransaction<'_>,
    policy: &MigrationPolicy,
) -> Result<LedgerLock, VerificationError> {
    use sqlx::Connection;

    // Protect the ledger and conservatively retain locks on existing descendants.
    // Inherited ledgers are rejected after snapshot capture; ACCESS SHARE cannot
    // prevent later attachments, so the supported ledger read also uses ONLY.
    // SAVEPOINT and LOCK do not establish the repeatable-read snapshot. A
    // savepoint lets an absent/unsupported ledger remain a structured finding
    // while preserving inspection of the other declared surfaces.
    let mut guard = transaction
        .begin()
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    let locked = sqlx::query(sqlx::AssertSqlSafe(format!(
        "LOCK TABLE {} IN ACCESS SHARE MODE",
        policy.ledger.quoted()
    )))
    .execute(&mut *guard)
    .await;
    match locked {
        Ok(_) => {
            guard
                .commit()
                .await
                .map_err(|error| VerificationError::Native(error.into()))?;
            Ok(LedgerLock::Ready)
        }
        Err(error) => {
            let code = error.as_database_error().and_then(|error| error.code());
            let outcome = match code.as_deref() {
                Some("42P01" | "3F000") => Some(LedgerLock::Missing),
                Some("42809") => Some(LedgerLock::WrongKind),
                _ => None,
            };
            if let Err(rollback) = guard.rollback().await {
                return Err(VerificationError::Rollback {
                    primary: Some(Box::new(VerificationError::Native(error.into()))),
                    source: rollback.into(),
                });
            }
            outcome.ok_or_else(|| VerificationError::Native(error.into()))
        }
    }
}

pub(crate) async fn inspect(
    transaction: &mut PgTransaction<'_>,
    policy: &MigrationPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let ledger = policy.ledger.quoted();
    let metadata = sqlx::query_as::<_, (i64, String, bool)>(
        "SELECT c.oid::bigint, c.relkind::text, c.relrowsecurity
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE (n.nspname = $1
                OR ($1 = 'pg_temp' AND n.oid = pg_catalog.pg_my_temp_schema()))
           AND c.relname = $2",
    )
    .bind(policy.ledger.schema())
    .bind(policy.ledger.name())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;

    let Some((ledger_oid, relkind, row_security)) = metadata else {
        findings.push(Finding::new(
            FindingKind::MissingMigration,
            Some(ledger),
            None::<String>,
            None,
        ));
        return Ok(());
    };

    if relkind != "r" {
        findings.push(Finding::new(
            FindingKind::LedgerColumnShape,
            Some(policy.ledger.quoted()),
            None::<String>,
            None,
        ));
        return Ok(());
    }

    if row_security {
        findings.push(Finding::new(
            FindingKind::LedgerRowSecurity,
            Some(policy.ledger.quoted()),
            None::<String>,
            None,
        ));
        return Ok(());
    }

    if !ledger_shape_is_valid(transaction, policy, ledger_oid, findings).await? {
        return Ok(());
    }

    // `ledger` is made only from individually validated identifiers and is
    // quoted as a complete qualified name above; SQLx 0.9 requires this
    // explicit assertion for the one unavoidable identifier interpolation.
    let rows = sqlx::query_as::<_, (i64, Vec<u8>, i64, bool)>(sqlx::AssertSqlSafe(format!(
        "SELECT version,
                substring(checksum FROM 1 FOR $1),
                octet_length(checksum)::bigint,
                success
         FROM ONLY {ledger}
         ORDER BY version
         LIMIT $2"
    )))
    .bind(i32::try_from(MAX_MIGRATION_CHECKSUM_BYTES).expect("checksum limit fits i32"))
    .bind(i64::try_from(MAX_MIGRATION_LEDGER_ROWS + 1).expect("ledger limit fits i64"))
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;

    if rows.len() > MAX_MIGRATION_LEDGER_ROWS {
        findings.push(Finding::new(
            FindingKind::MigrationLedgerLimit,
            Some(ledger),
            None::<String>,
            None,
        ));
        return Ok(());
    }

    let rows = rows
        .into_iter()
        .map(|(version, mut checksum, checksum_length, success)| {
            if checksum_length
                > i64::try_from(MAX_MIGRATION_CHECKSUM_BYTES).expect("checksum limit fits i64")
            {
                // Policy checksums cannot exceed the cap, so one extra byte
                // preserves a definite mismatch without materializing the
                // remainder of an oversized database value.
                checksum.push(0);
            }
            (version, checksum, success)
        })
        .collect::<Vec<_>>();

    compare_rows(policy, &rows, findings);
    Ok(())
}

async fn ledger_shape_is_valid(
    transaction: &mut PgTransaction<'_>,
    policy: &MigrationPolicy,
    ledger_oid: i64,
    findings: &mut Vec<Finding>,
) -> Result<bool, VerificationError> {
    let columns = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT a.attname::text,
                t.typname::text,
                a.attnotnull
         FROM pg_catalog.pg_attribute AS a
         JOIN pg_catalog.pg_type AS t ON t.oid = a.atttypid
         WHERE a.attrelid = $1::oid
           AND a.attnum > 0
           AND NOT a.attisdropped",
    )
    .bind(ledger_oid)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    let by_name = columns
        .into_iter()
        .map(|(name, ty, not_null)| (name, (ty, not_null)))
        .collect::<HashMap<_, _>>();

    // Compare stable pg_type names rather than `format_type`, whose internal
    // catalog lookup is not part of a repeatable-read snapshot guarantee.
    let required_columns = [
        ("version", "int8"),
        ("checksum", "bytea"),
        ("success", "bool"),
    ];
    let mut shape_ok = true;
    for (name, expected_type) in required_columns {
        let kind = match by_name.get(name) {
            Some((actual_type, not_null)) if actual_type == expected_type && *not_null => continue,
            Some(_) => FindingKind::LedgerColumnShape,
            None => FindingKind::MissingLedgerColumn,
        };
        shape_ok = false;
        findings.push(Finding::new(
            kind,
            Some(ledger_column_name(policy, name)),
            None::<String>,
            None,
        ));
    }
    Ok(shape_ok)
}

fn ledger_column_name(policy: &MigrationPolicy, name: &str) -> String {
    format!("{}.{}", policy.ledger.quoted(), quote_identifier(name))
}

fn compare_rows(
    policy: &MigrationPolicy,
    rows: &[(i64, Vec<u8>, bool)],
    findings: &mut Vec<Finding>,
) {
    let mut by_version: HashMap<i64, Vec<MigrationRow<'_>>> = HashMap::new();
    for row in rows {
        by_version.entry(row.0).or_default().push(row);
    }

    for (version, entries) in &by_version {
        if entries.len() > 1 {
            findings.push(Finding::new(
                FindingKind::DuplicateMigrationRow,
                Some(policy.ledger.quoted()),
                Some(version.to_string()),
                None,
            ));
        }
    }

    let required: HashMap<i64, &MigrationExpectation> = policy
        .required
        .iter()
        .map(|migration| (migration.version, migration))
        .collect();
    let required_max = policy
        .required
        .iter()
        .map(|migration| migration.version)
        .max();

    for migration in &policy.required {
        let Some(entries) = by_version.get(&migration.version) else {
            findings.push(Finding::new(
                FindingKind::MissingMigration,
                Some(policy.ledger.quoted()),
                Some(migration.version.to_string()),
                None,
            ));
            continue;
        };
        for (_, checksum, success) in entries {
            if !success {
                findings.push(Finding::new(
                    FindingKind::UnsuccessfulMigration,
                    Some(policy.ledger.quoted()),
                    Some(migration.version.to_string()),
                    None,
                ));
            }
            if checksum != &migration.checksum {
                findings.push(Finding::new(
                    FindingKind::MigrationChecksumMismatch,
                    Some(policy.ledger.quoted()),
                    Some(migration.version.to_string()),
                    None,
                ));
            }
        }
    }

    let listed: HashMap<i64, &MigrationExpectation> = match &policy.additional {
        AdditionalMigrations::AllowListed(rows) => rows
            .iter()
            .map(|migration| (migration.version, migration))
            .collect(),
        AdditionalMigrations::Reject | AdditionalMigrations::AllowSuccessful => HashMap::new(),
    };

    let mut seen_extra = HashSet::new();
    for (version, entries) in by_version {
        if required.contains_key(&version) || !seen_extra.insert(version) {
            continue;
        }
        for (_, checksum, success) in entries {
            let allowed = match &policy.additional {
                AdditionalMigrations::Reject => false,
                AdditionalMigrations::AllowSuccessful => {
                    *success && required_max.is_some_and(|maximum| version > maximum)
                }
                AdditionalMigrations::AllowListed(_) => {
                    listed.get(&version).is_some_and(|expected| {
                        expected.checksum == *checksum
                            && *success
                            && required_max.is_some_and(|maximum| version > maximum)
                    })
                }
            };
            if !allowed {
                findings.push(Finding::new(
                    FindingKind::UnexpectedMigration,
                    Some(policy.ledger.quoted()),
                    Some(version.to_string()),
                    None,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::policy::QualifiedName;
    use crate::verification::report::FindingKind;

    fn policy(additional: AdditionalMigrations) -> MigrationPolicy {
        MigrationPolicy {
            ledger: QualifiedName::new("public", "ledger").unwrap(),
            required: vec![MigrationExpectation::new(1, vec![1, 2, 3])],
            additional,
        }
    }

    #[test]
    fn required_rows_check_checksum_success_and_later_compatibility() {
        let mut findings = Vec::new();
        compare_rows(
            &policy(AdditionalMigrations::AllowSuccessful),
            &[(1, vec![1, 2, 3], true), (2, vec![4], true)],
            &mut findings,
        );
        assert!(findings.is_empty());

        let mut findings = Vec::new();
        compare_rows(
            &policy(AdditionalMigrations::Reject),
            &[(1, vec![1, 2, 3], true), (2, vec![4], true)],
            &mut findings,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::UnexpectedMigration)
        );

        let mut findings = Vec::new();
        compare_rows(
            &policy(AdditionalMigrations::AllowSuccessful),
            &[(1, vec![9], false)],
            &mut findings,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::MigrationChecksumMismatch)
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::UnsuccessfulMigration)
        );
    }

    #[test]
    fn required_absence_and_duplicate_rows_are_findings() {
        let mut findings = Vec::new();
        compare_rows(
            &policy(AdditionalMigrations::Reject),
            &[(2, vec![4], true), (2, vec![4], true)],
            &mut findings,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::MissingMigration)
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::DuplicateMigrationRow)
        );
    }

    #[test]
    fn ledger_column_findings_quote_every_identifier_component() {
        assert_eq!(
            ledger_column_name(&policy(AdditionalMigrations::Reject), "version"),
            "\"public\".\"ledger\".\"version\""
        );
    }

    #[test]
    fn allow_successful_requires_a_nonempty_later_boundary() {
        let policy = MigrationPolicy {
            ledger: QualifiedName::new("public", "ledger").unwrap(),
            required: Vec::new(),
            additional: AdditionalMigrations::AllowSuccessful,
        };
        let mut findings = Vec::new();
        compare_rows(&policy, &[(1, vec![1], true)], &mut findings);
        assert_eq!(
            findings
                .iter()
                .filter(|finding| finding.kind == FindingKind::UnexpectedMigration)
                .count(),
            1
        );
    }

    #[test]
    fn allow_listed_requires_a_successful_later_checksum() {
        let policy = policy(AdditionalMigrations::AllowListed(vec![
            MigrationExpectation::new(2, vec![2]),
        ]));
        let mut findings = Vec::new();
        compare_rows(
            &policy,
            &[(1, vec![1, 2, 3], true), (2, vec![2], true)],
            &mut findings,
        );
        assert!(findings.is_empty());

        let mut findings = Vec::new();
        compare_rows(
            &policy,
            &[(1, vec![1, 2, 3], true), (2, vec![9], true)],
            &mut findings,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::UnexpectedMigration)
        );

        let mut findings = Vec::new();
        compare_rows(
            &policy,
            &[(1, vec![1, 2, 3], true), (0, vec![2], true)],
            &mut findings,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.kind == FindingKind::UnexpectedMigration)
        );
    }
}
