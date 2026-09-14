use super::{
    MAX_MIGRATION_CHECKSUM_BYTES, MAX_MIGRATION_LEDGER_ROWS, PgTransaction, QualifiedName,
    VerificationError,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

const FIRST_NORMAL_OBJECT_ID: i64 = 16_384;
const MAX_CURSOR_NAME_ATTEMPTS: usize = 10_001;
static NEXT_CURSOR_ID: AtomicU64 = AtomicU64::new(0);

pub(super) async fn declare_history_cursor(
    transaction: &mut PgTransaction<'_>,
    ledger: &QualifiedName,
    protected_oid: i64,
) -> Result<Option<u64>, VerificationError> {
    use sqlx::Connection;

    let cursor_id = available_cursor_id(transaction, protected_oid).await?;
    let protected_select: bool =
        sqlx::query_scalar("SELECT pg_catalog.has_table_privilege($1::bigint::oid, 'SELECT')")
            .bind(protected_oid)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|error| VerificationError::Native(error.into()))?;
    let allowed_locks = protected_relation_locks(transaction, protected_oid).await?;
    let locks_before = held_access_share_locks(transaction).await?;
    let mut guard = transaction
        .begin()
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    let declared = sqlx::query(sqlx::AssertSqlSafe(history_cursor_sql(
        ledger,
        protected_oid,
        cursor_id,
    )))
    .persistent(false)
    .execute(&mut *guard)
    .await;
    match declared {
        Ok(_) => finish_declaration(guard, locks_before, allowed_locks)
            .await
            .map(|protected| protected.then_some(cursor_id)),
        Err(error) => {
            let unavailable = error
                .as_database_error()
                .and_then(|error| error.code())
                .is_some_and(|code| {
                    declaration_failure_is_unavailable(code.as_ref(), protected_select)
                });
            if let Err(rollback) = guard.rollback().await {
                return Err(VerificationError::Rollback {
                    primary: Some(Box::new(VerificationError::Native(error.into()))),
                    source: rollback.into(),
                });
            }
            if unavailable {
                Ok(None)
            } else {
                Err(VerificationError::Native(error.into()))
            }
        }
    }
}

async fn available_cursor_id(
    transaction: &mut PgTransaction<'_>,
    protected_oid: i64,
) -> Result<u64, VerificationError> {
    for _ in 0..MAX_CURSOR_NAME_ATTEMPTS {
        let cursor_id = NEXT_CURSOR_ID.fetch_add(1, Ordering::Relaxed);
        let cursor_name = history_cursor_name(protected_oid, cursor_id);
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_cursors WHERE name = $1)",
        )
        .bind(&cursor_name)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
        if !exists {
            return Ok(cursor_id);
        }
    }
    Err(VerificationError::CatalogCapacity)
}

fn declaration_failure_is_unavailable(code: &str, protected_select: bool) -> bool {
    matches!(code, "42P01" | "3F000" | "42703" | "42809" | "42883")
        || code == "42501" && protected_select
}

async fn finish_declaration(
    mut guard: sqlx::Transaction<'_, sqlx::Postgres>,
    locks_before: HashSet<i64>,
    allowed_locks: HashSet<i64>,
) -> Result<bool, VerificationError> {
    let locks_after = match held_access_share_locks_on(&mut guard).await {
        Ok(locks) => locks,
        Err(error) => {
            if let Err(rollback) = guard.rollback().await {
                return Err(VerificationError::Rollback {
                    primary: Some(Box::new(error)),
                    source: rollback.into(),
                });
            }
            return Err(error);
        }
    };
    if !cursor_locks_are_protected(&locks_before, &locks_after, &allowed_locks) {
        guard
            .rollback()
            .await
            .map_err(|error| VerificationError::Native(error.into()))?;
        return Ok(false);
    }
    guard
        .commit()
        .await
        .map(|_| true)
        .map_err(|error| VerificationError::Native(error.into()))
}

fn history_cursor_sql(ledger: &QualifiedName, protected_oid: i64, cursor_id: u64) -> String {
    let cursor_name = history_cursor_name(protected_oid, cursor_id);
    format!(
        "DECLARE {cursor_name} NO SCROLL CURSOR WITHOUT HOLD FOR
         SELECT {protected_oid}::bigint AS relation_oid,
                history.version, history.checksum, history.checksum_length,
                history.success, history.table_oid
         FROM (
             SELECT ledger.version,
                    substring(ledger.checksum FROM 1 FOR {MAX_MIGRATION_CHECKSUM_BYTES})
                        AS checksum,
                    octet_length(ledger.checksum)::bigint AS checksum_length,
                    ledger.success,
                    ledger.tableoid::bigint AS table_oid
             FROM ONLY {} AS ledger
             ORDER BY ledger.version
             LIMIT {}
         ) AS history
         RIGHT JOIN (SELECT true AS sentinel) AS identity ON true",
        ledger.quoted(),
        MAX_MIGRATION_LEDGER_ROWS + 1,
    )
}

pub(super) fn history_cursor_name(protected_oid: i64, cursor_id: u64) -> String {
    format!("batter_history_{protected_oid}_{cursor_id}")
}

async fn protected_relation_locks(
    transaction: &mut PgTransaction<'_>,
    protected_oid: i64,
) -> Result<HashSet<i64>, VerificationError> {
    let relations = sqlx::query_scalar::<_, i64>(
        "WITH protected AS (
             SELECT c.oid, c.reltoastrelid
             FROM pg_catalog.pg_class AS c
             WHERE c.oid = $1::bigint::oid
         ), related(oid) AS (
             SELECT oid FROM protected
             UNION
             SELECT indexrelid
             FROM pg_catalog.pg_index
             WHERE indrelid = $1::bigint::oid
             UNION
             SELECT reltoastrelid FROM protected WHERE reltoastrelid <> 0
             UNION
             SELECT i.indexrelid
             FROM protected AS p
             JOIN pg_catalog.pg_index AS i ON i.indrelid = p.reltoastrelid
         )
         SELECT oid::bigint FROM related",
    )
    .bind(protected_oid)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    Ok(relations.into_iter().collect())
}

async fn held_access_share_locks(
    transaction: &mut PgTransaction<'_>,
) -> Result<HashSet<i64>, VerificationError> {
    let locks = sqlx::query_scalar::<_, i64>(HELD_LOCKS_SQL)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    Ok(locks.into_iter().collect())
}

async fn held_access_share_locks_on(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<HashSet<i64>, VerificationError> {
    let locks = sqlx::query_scalar::<_, i64>(HELD_LOCKS_SQL)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    Ok(locks.into_iter().collect())
}

const HELD_LOCKS_SQL: &str = "SELECT relation::bigint
     FROM pg_catalog.pg_locks
     WHERE pid = pg_catalog.pg_backend_pid()
       AND locktype = 'relation'
       AND mode = 'AccessShareLock'
       AND granted
       AND relation IS NOT NULL";

fn cursor_locks_are_protected(
    before: &HashSet<i64>,
    after: &HashSet<i64>,
    allowed: &HashSet<i64>,
) -> bool {
    after
        .difference(before)
        .all(|oid| *oid < FIRST_NORMAL_OBJECT_ID || allowed.contains(oid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_sql_never_places_policy_names_in_string_literals() {
        let ledger = QualifiedName::new("schema\\'s", "ledger\\'s").unwrap();
        let sql = history_cursor_sql(&ledger, 42, 7);
        assert!(sql.starts_with("DECLARE batter_history_42_7 "));
        assert!(sql.contains("FROM ONLY \"schema\\'s\".\"ledger\\'s\" AS ledger"));
        assert!(!sql.contains("::pg_catalog.regclass"));
        assert!(!sql.contains("'schema"));
        assert!(sql.contains("SELECT 42::bigint AS relation_oid"));
    }

    #[test]
    fn relation_or_shape_declaration_failures_are_unavailable() {
        for code in ["42P01", "3F000", "42703", "42809", "42883"] {
            assert!(declaration_failure_is_unavailable(code, false));
        }
        assert!(declaration_failure_is_unavailable("42501", true));
        assert!(!declaration_failure_is_unavailable("42501", false));
        for code in ["42601", "08006", "53200"] {
            assert!(!declaration_failure_is_unavailable(code, true));
        }
    }

    #[test]
    fn cursor_relation_locks_must_belong_to_the_protected_heap() {
        let before = HashSet::from([42, 1_259]);
        let allowed = HashSet::from([42, 84]);
        assert!(cursor_locks_are_protected(
            &before,
            &HashSet::from([42, 84, 1_259]),
            &allowed
        ));
        assert!(!cursor_locks_are_protected(
            &before,
            &HashSet::from([42, 84, 99_999]),
            &allowed
        ));
    }
}
