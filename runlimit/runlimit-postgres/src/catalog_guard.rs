//! Every production statement resolves catalog objects by their `pg_catalog`
//! name, so a schema earlier on the caller's `search_path` cannot replace a
//! function or type the limiter relies on (a shadowed `set_config` would
//! silently drop the server timeouts; a shadowed clock would move windows).
//!
//! The rule covers function calls and casts. SQL syntax that the PostgreSQL
//! grammar maps to `pg_catalog` itself (`EXTRACT`, `COALESCE`, `GREATEST`,
//! `LEAST`, and keyword types such as `bigint`) needs no prefix. Operators
//! and the limiter's own tables still resolve through `search_path`, which
//! the application chooses. Published migrations are immutable and are not
//! checked here.
//!
//! Completeness does not depend on remembering to register a statement: every
//! SQL-looking string literal in production source must initialize a
//! registered constant, and every `.sql` file in `src/` must be registered.

use std::{collections::BTreeSet, fs, path::Path};

mod detector;
mod literals;

use detector::unqualified;
use literals::{rust_files, sql_literals};

use crate::{
    attempts::{row::StoredAttempt, sql as attempt},
    gcra, protocol,
};

/// `(source file, constant, statement)` for every statement the crate sends.
const STATEMENTS: &[(&str, &str, &str)] = &[
    (
        "protocol.rs",
        "SET_LOCAL_TIMEOUTS_SQL",
        protocol::SET_LOCAL_TIMEOUTS_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_ADVISORY_LOCK_SQL",
        protocol::BATCH_ADVISORY_LOCK_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_ROW_LOCK_SQL",
        protocol::BATCH_ROW_LOCK_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_CAPACITY_LOCK_SQL",
        protocol::BATCH_CAPACITY_LOCK_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_PREFLIGHT_SQL",
        protocol::BATCH_PREFLIGHT_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_UPSERT_SQL",
        protocol::BATCH_UPSERT_SQL,
    ),
    ("protocol.rs", "CLEANUP_SQL", protocol::CLEANUP_SQL),
    ("gcra.rs", "LOCK_SHARDS", gcra::LOCK_SHARDS),
    ("gcra.rs", "SAMPLE_TIME", gcra::SAMPLE_TIME),
    ("gcra.rs", "READ_COUNTERS", gcra::READ_COUNTERS),
    ("gcra.rs", "UPSERT", gcra::UPSERT),
    ("gcra.rs", "UPDATE_WATERMARKS", gcra::UPDATE_WATERMARKS),
    ("gcra.rs", "CLEANUP_LOCKS", gcra::CLEANUP_LOCKS),
    (
        "gcra.rs",
        "CLEANUP_ADVANCE_TIME",
        gcra::CLEANUP_ADVANCE_TIME,
    ),
    ("gcra.rs", "CLEANUP_DELETE", gcra::CLEANUP_DELETE),
    ("attempts/row.rs", "LOCK_SQL", StoredAttempt::LOCK_SQL),
    ("attempts/sql.rs", "SHARD_LOCK_SQL", attempt::SHARD_LOCK_SQL),
    ("attempts/sql.rs", "CLEANUP_SQL", attempt::CLEANUP_SQL),
    (
        "attempts/sql.rs",
        "SAMPLE_TIME_SQL",
        attempt::SAMPLE_TIME_SQL,
    ),
    ("attempts/sql.rs", "OCCUPANCY_SQL", attempt::OCCUPANCY_SQL),
    ("attempts/sql.rs", "FREE_SLOT_SQL", attempt::FREE_SLOT_SQL),
    ("attempts/sql.rs", "INSERT_SQL", attempt::INSERT_SQL),
    ("attempts/sql.rs", "LEASE_SQL", attempt::LEASE_SQL),
    ("attempts/sql.rs", "CLAIM_SQL", attempt::CLAIM_SQL),
    ("attempts/sql.rs", "COMPLETE_SQL", attempt::COMPLETE_SQL),
];

#[test]
fn every_statement_qualifies_its_catalog_functions_and_types() {
    let mut violations = Vec::new();
    for (file, name, sql) in STATEMENTS {
        violations.extend(
            unqualified(sql)
                .into_iter()
                .map(|found| format!("{file} {name}: {found}")),
        );
    }
    assert!(
        violations.is_empty(),
        "unqualified catalog objects:\n{violations:#?}"
    );
}

#[test]
fn every_production_sql_literal_and_file_is_registered() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let registered: BTreeSet<(&str, &str)> = STATEMENTS
        .iter()
        .map(|(file, name, _)| (*file, *name))
        .collect();
    let mut unregistered = Vec::new();
    for path in rust_files(&source) {
        let file = path
            .strip_prefix(&source)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        if file == "catalog_guard.rs"
            || file.starts_with("catalog_guard/")
            || file.ends_with("tests.rs")
            || file.contains("/tests/")
        {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let production = text
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or_default();
        for (owner, literal) in sql_literals(production) {
            match owner {
                Some(name) if registered.contains(&(file.as_str(), name.as_str())) => {}
                owner => unregistered.push(format!("{file}: {owner:?} {literal:.60}")),
            }
        }
    }
    for entry in fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "sql") {
            let text = fs::read_to_string(&path).unwrap();
            if !STATEMENTS.iter().any(|(_, _, sql)| *sql == text) {
                unregistered.push(format!("{} is not a registered statement", path.display()));
            }
        }
    }
    assert!(
        unregistered.is_empty(),
        "unregistered SQL:\n{unregistered:#?}"
    );
}

#[test]
fn the_detector_rejects_each_unqualified_shape_and_accepts_syntax() {
    for sql in [
        "SELECT floor(1)",
        "SELECT count (*) FROM t",
        "SELECT x::text",
        "SELECT $1::BYTEA[]",
        "SELECT set_config('a', $1, true)",
        "WITH t AS (SELECT clock_timestamp()) SELECT 1",
        "SELECT 1 FROM unnest($1::BIGINT[]) AS input(value)",
        "SELECT 1 FROM pg_catalog.unnest($1::pg_catalog.bytea[], $2::BIGINT[]) AS input(a, b)",
    ] {
        assert!(!unqualified(sql).is_empty(), "{sql} must be rejected");
    }
    for sql in [
        "SELECT pg_catalog.floor(extract(epoch FROM pg_catalog.clock_timestamp()))::bigint",
        "SELECT pg_catalog.count(*) FROM t WHERE x = ANY($1) AND y IN (1)",
        "WITH RECURSIVE acquired(position) AS (SELECT 1), b AS MATERIALIZED (SELECT 2) \
         SELECT COALESCE(GREATEST(1, 2), LEAST(3, 4))",
        "INSERT INTO t (a, b) VALUES (1, $1::pg_catalog.text) ON CONFLICT (a) DO NOTHING",
        "SELECT 1 FROM pg_catalog.unnest($1::pg_catalog.bytea[]) AS input(value)",
        "SELECT 1 FROM ROWS FROM (pg_catalog.unnest($1::pg_catalog.bytea[]), \
         pg_catalog.unnest(ARRAY[1, 2]::BIGINT[])) WITH ORDINALITY AS input(a, b, position)",
        "SELECT 'floor(1)::text' -- count(*)\n",
        "SELECT $1::integer, $2::SMALLINT, $3::double precision",
    ] {
        assert_eq!(
            unqualified(sql),
            Vec::<String>::new(),
            "{sql} must be accepted"
        );
    }
}
