//! Every operational statement resolves catalog objects by their
//! `pg_catalog` name, so a schema earlier on the caller's `search_path` cannot
//! replace a function or type the limiter relies on (a shadowed `set_config`
//! would silently drop the server timeouts; a shadowed clock would move
//! windows).
//!
//! The rule covers function calls, casts and composite-field notation: `x.f`
//! calls `f(x)` when `x` has no column `f`, so every qualified name must be a
//! column of its own receiver, which `fields` resolves. SQL syntax that the
//! PostgreSQL grammar maps to `pg_catalog` itself (`EXTRACT`, `COALESCE`,
//! `GREATEST`, `LEAST`, and keyword types such as `bigint`) needs no prefix.
//! Operators and the limiter's own tables still resolve through
//! `search_path`, which the application chooses. Published migrations are
//! immutable, are not qualified and are not checked here; they run under the
//! caller's `search_path`, which must be trusted while migrating.
//!
//! Operational SQL keeps to a dialect this guard reads completely: no dollar
//! quoting, no quoted identifiers, casts only as `::` (never `CAST(...)` or a
//! typed literal such as `text 'value'`), and string constants only as
//! function arguments or list elements. The guard rejects anything outside it
//! rather than trying to parse it.
//!
//! The set of statements is closed by construction: `crate::statements`
//! generates [`Statement`](crate::statements::Statement) from each family's
//! `statements!` block, whose text is a string literal or an included `.sql`
//! file in `src`, so `statements::all()` lists every statement the crate can
//! send and the first test below checks each one. No other SQL text can
//! reach PostgreSQL: the crate's `clippy.toml` disallows every sqlx entry
//! point that accepts SQL text, and `gate` checks that inventory, probes each
//! entry under Clippy and checks that only `Statement::query` and the three
//! bundled migrators suppress it.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

mod detector;
mod fields;
mod gate;
mod live;

use detector::{catalog_calls, unqualified};

use crate::statements;

/// The catalog functions the limiters call. Any other is a decision to make
/// here, not a qualified call to accept: `pg_catalog.query_to_xml`, for one,
/// runs its text argument as SQL that `statements::all()` never lists.
const CATALOG_FUNCTIONS: &[&str] = &[
    "cardinality",
    "clock_timestamp",
    "count",
    "floor",
    "gen_random_uuid",
    "generate_series",
    "min",
    "pg_advisory_xact_lock",
    "pg_current_xact_id",
    "set_config",
    "unnest",
];

/// A statement the limiters send is a query or a data change, never a
/// command such as `EXECUTE` that runs SQL this guard cannot see.
fn command_form(sql: &str) -> Option<String> {
    let code = detector::strip_comments_and_quotes(sql);
    // One command only: anything after a semicolon is a second one.
    if code
        .split_once(';')
        .is_some_and(|(_, rest)| !rest.trim().is_empty())
    {
        return Some("more than one command".to_owned());
    }
    let first = code
        .trim_start()
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    (!["select", "with", "insert", "update", "delete"].contains(&first.as_str()))
        .then(|| format!("a {first:?} command, not a query or data change"))
}

/// Each family's migrated tables and their columns. The guard cannot read
/// what a migration's functions and triggers leave behind, so
/// `tests/migrate.rs` compares this list with each freshly migrated history.
const MIGRATED_COLUMNS: &str = include_str!("catalog_guard/migrated_columns.txt");

/// The tables `statement`'s own family installs, with their columns. Each
/// history installs alone, so a statement reads only its family's tables;
/// the session family reads none.
fn family_tables(statement: statements::Statement) -> BTreeMap<String, BTreeSet<String>> {
    let family = match statement {
        statements::Statement::FixedWindow(_) => "fixed_window",
        statements::Statement::Gcra(_) => "gcra",
        statements::Statement::Attempts(_) => "attempts",
        statements::Statement::Session(_) => return BTreeMap::new(),
    };
    MIGRATED_COLUMNS
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some(family)).then(|| {
                let table = words.next().expect("each listed table has a name");
                (table.to_owned(), words.map(str::to_owned).collect())
            })
        })
        .collect()
}

/// An inventory of the named tables and their columns.
fn inventory(tables: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    tables
        .iter()
        .map(|(table, columns)| {
            (
                (*table).to_owned(),
                columns.iter().map(|c| (*c).to_owned()).collect(),
            )
        })
        .collect()
}

#[test]
fn each_family_reads_only_the_tables_its_history_installs() {
    let fixed_window = family_tables(statements::Statement::FixedWindow(
        statements::fixed_window::Id::ALL[0],
    ));
    assert!(fixed_window.contains_key("runlimit_fixed_windows"));
    assert!(!fixed_window.contains_key("runlimit_gcra"));
    assert!(!fixed_window.contains_key("runlimit_attempts"));
    assert!(
        family_tables(statements::Statement::Session(
            statements::session::Id::ALL[0]
        ))
        .is_empty()
    );
}

#[test]
fn every_statement_qualifies_its_catalog_functions_and_types() {
    let mut violations = Vec::new();
    for (name, statement) in statements::all() {
        let tables = family_tables(statement);
        violations.extend(
            unqualified(statement.sql())
                .into_iter()
                .chain(command_form(statement.sql()))
                .chain(fields::unresolved(statement.sql(), &tables))
                .chain(
                    catalog_calls(statement.sql())
                        .into_iter()
                        .filter(|call| !CATALOG_FUNCTIONS.contains(&call.as_str()))
                        .map(|call| {
                            format!(
                                "pg_catalog.{call}(...) is not a catalog function the limiters call"
                            )
                        }),
                )
                .map(|found| format!("{name}: {found}")),
        );
    }
    assert!(
        violations.is_empty(),
        "unqualified catalog objects:\n{violations:#?}"
    );
}

// A statement may include only a `.sql` file directly in `src`, so a published
// migration, which lives in its migrator's directory, cannot become one; and
// every such file is a statement, so none is left unchecked.
#[test]
fn included_statements_are_exactly_the_sql_files_in_src() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let on_disk: BTreeSet<String> = fs::read_dir(&source)
        .expect("src is readable")
        .map(|entry| entry.expect("src entry is readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let mut included = BTreeSet::new();
    let mut problems = Vec::new();
    for (name, statement) in statements::all() {
        let Some(file) = statement.file() else {
            continue;
        };
        // Relative to src/statements/<family>.rs.
        match file.strip_prefix("../") {
            Some(sql)
                if !sql.contains('/')
                    && Path::new(sql)
                        .extension()
                        .is_some_and(|extension| extension == "sql") =>
            {
                if !included.insert(sql.to_owned()) {
                    problems.push(format!(
                        "{name} includes {sql}, which another statement includes"
                    ));
                }
            }
            _ => problems.push(format!("{name} includes {file}, not a .sql file in src")),
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(
        included, on_disk,
        "every .sql file in src is exactly one statement"
    );
}

#[test]
fn only_the_listed_catalog_functions_may_be_called() {
    let calls = catalog_calls(
        "SELECT pg_catalog.query_to_xml($1, true, false, ''), pg_catalog . floor(1), \
         pg_catalog.text 'x', $2::pg_catalog.text, $3::pg_catalog.numeric(39, 0), \
         $4 :: pg_catalog . varchar(8)",
    );
    assert_eq!(
        calls,
        ["floor", "query_to_xml"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
    assert!(!CATALOG_FUNCTIONS.contains(&"query_to_xml"));
    for indirect in [
        "EXECUTE cached_check",
        "CALL run_check()",
        "PREPARE p AS SELECT 1",
    ] {
        assert!(command_form(indirect).is_some(), "{indirect}");
    }
    assert_eq!(command_form("-- note\nWITH x AS (SELECT 1) SELECT 1"), None);
    assert!(command_form("/* SELECT */ EXECUTE cached_check").is_some());
    assert_eq!(
        command_form("SELECT 1; SELECT 2"),
        Some("more than one command".to_owned())
    );
    assert_eq!(command_form("SELECT 1; -- done\n"), None);
}

// A statement fails where its family's history alone is installed if it
// reads or writes another family's table, so every relation, aliased or
// not and wherever it appears, is checked against the family's tables.
#[test]
fn every_relation_is_a_table_of_the_family_or_a_cte() {
    let tables = inventory(&[("t", &["id", "shard", "used"]), ("u", &["id", "label"])]);
    for sql in [
        // `distinct` alone does not make the next FROM a comparison.
        "SELECT 1 AS distinct FROM elsewhere",
        "WITH s AS (SELECT 1 AS distinct FROM t) SELECT s.distinct, s.other FROM s",
        // A parenthesized DELETE source is a FROM item, not a column list.
        "DELETE FROM t USING (u JOIN elsewhere ON TRUE) WHERE t.id = 1",
        // A TABLE query reads a relation outside any FROM list.
        "SELECT EXISTS (TABLE elsewhere)",
        "SELECT EXISTS (TABLE t)",
        // A DELETE source subquery declares its alias like any FROM item.
        "DELETE FROM t USING (SELECT u.id FROM u) pg_catalog WHERE pg_catalog.row_to_json IS NULL",
        "DELETE FROM t USING (SELECT u.id FROM u) AS expired WHERE expired.label IS NULL",
        // Every relation is checked, aliased or not, and so is an INSERT target.
        "SELECT pg_catalog.count(*) FROM elsewhere AS e",
        "SELECT a.id FROM t AS a JOIN elsewhere AS e ON TRUE",
        "INSERT INTO elsewhere AS e (id) VALUES (1)",
        "INSERT INTO elsewhere (id) VALUES (1)",
        "SELECT e.id FROM other.t AS e",
    ] {
        assert!(
            !fields::unresolved(sql, &tables).is_empty(),
            "{sql} must fail"
        );
    }
}

// Unreserved keywords may name columns and aliases, so a keyword after AS,
// or a command word anywhere but where a command begins, is a name.
#[test]
fn a_keyword_used_as_a_name_is_a_name() {
    let tables = inventory(&[("t", &["id", "shard", "used"]), ("u", &["id", "label"])]);
    for sql in [
        // RETURNING after a label starts no RETURNING list.
        "WITH w AS (INSERT INTO t (id, used) SELECT 1 AS returning, 2 AS n RETURNING id) SELECT w.n FROM w",
        // EXCLUDED is the DO UPDATE action's alone, not RETURNING's.
        "INSERT INTO t (id) VALUES (1) ON CONFLICT (id) DO NOTHING RETURNING EXCLUDED.used",
        "INSERT INTO t (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET used = 1 RETURNING EXCLUDED.used",
        // A keyword used as a name is a name: it neither ends the FROM list
        // nor opens a command, and a bare one that would hide the rest fails.
        "SELECT 1 FROM t AS set, elsewhere",
        "SELECT 1 FROM t set, elsewhere",
        "SELECT 1 AS delete FROM t, elsewhere",
        "SELECT 1 delete FROM t, elsewhere",
        "SELECT (1) delete FROM t, elsewhere",
        "SELECT 1 AS update FROM t, elsewhere",
        // A label spelled as leaves the AS keyword before it a keyword.
        "SELECT 1 AS as FROM elsewhere",
        "SELECT 1 AS as FROM t, elsewhere",
        // A column named rows before FROM opens no ROWS FROM item.
        "SELECT 1, rows FROM (SELECT 2 AS rows) AS s, elsewhere",
        // Stacked join qualifications all end before the next item.
        "SELECT a.id FROM t AS a JOIN u AS b JOIN u AS c USING (id) ON TRUE, elsewhere",
        "SELECT a.id FROM t AS a JOIN u AS b JOIN u AS c ON TRUE ON TRUE JOIN elsewhere AS e ON TRUE",
        // A field named for a clause does not end the join condition.
        "WITH k AS (SELECT 1 AS limit) SELECT a.id FROM t AS a JOIN k ON k.limit = a.id, elsewhere",
        "WITH k AS (SELECT 1 AS where) SELECT a.id FROM t AS a JOIN k ON k.where = a.id JOIN elsewhere AS e ON TRUE",
        // A join alias named conflict does not open ON CONFLICT.
        "SELECT a.id FROM t AS a JOIN u AS conflict ON conflict.id = a.id JOIN elsewhere AS e ON TRUE",
        "SELECT a.id FROM t AS a JOIN u AS conflict ON conflict.id = a.id, elsewhere",
        // A recursive CTE's SEARCH or CYCLE clause is outside the dialect.
        "WITH RECURSIVE r(id) AS (SELECT u.id FROM u) SEARCH DEPTH FIRST BY id SET ordering \
         UPDATE elsewhere SET used = 1",
        "WITH RECURSIVE r(id) AS (SELECT u.id FROM u) CYCLE id SET looped USING path \
         SELECT r.id FROM r",
        // SELECT INTO creates a table, whatever word precedes INTO.
        "SELECT 1 AS n INTO sampled",
        "WITH s AS (SELECT 1 AS n) SELECT n INTO sampled FROM s",
        "SELECT 1 AS insert INTO sampled",
        "SELECT 1 insert INTO sampled",
    ] {
        assert!(
            !fields::unresolved(sql, &tables).is_empty(),
            "{sql} must fail"
        );
    }
    for sql in [
        "WITH c AS (SELECT 1 AS id OFFSET 0) SELECT c.id FROM c",
        "SELECT a.id FROM t AS a JOIN u AS conflict ON conflict.id = a.id",
        "SELECT a.id FROM t AS a JOIN u AS b JOIN u AS c USING (id) ON TRUE, u AS d",
        "SELECT s.rows FROM (SELECT 2 AS rows) AS s, t",
        "SELECT 1 AS as FROM t",
        "INSERT INTO t (id) VALUES (1) RETURNING t.id",
        "UPDATE t SET used = t.used + 1 RETURNING t.used",
        "WITH k AS (SELECT 1 AS limit) SELECT a.id FROM t AS a JOIN k ON k.limit = a.id",
        "WITH k AS (SELECT 1 AS from, 2 AS n) SELECT k.from, k.n FROM k",
        "WITH k AS (SELECT 1 AS limit), c AS (SELECT k.limit, 2 AS n FROM k) SELECT c.limit, c.n FROM c",
        "SELECT search.id FROM (SELECT u.id FROM u) search",
        "WITH c AS (SELECT 1 AS limit, 2 AS n FROM t) SELECT c.limit, c.n FROM c",
        "WITH c AS (SELECT pg_catalog.percentile_cont(0.5) WITHIN GROUP (ORDER BY t.used) AS p FROM t) \
         SELECT c.p FROM c",
        "SELECT s.id FROM t AS set JOIN u AS s ON s.id = set.id",
        "WITH w AS (INSERT INTO u (id) VALUES (1) RETURNING id) SELECT w.id FROM w",
        "WITH w AS (SELECT 1 AS n) INSERT INTO t (id) SELECT w.n FROM w",
        "WITH w AS (SELECT 1 AS n) UPDATE t SET used = w.n FROM w WHERE t.id = w.n",
        "WITH w AS (SELECT 1 AS n) DELETE FROM t USING w WHERE t.id = w.n",
        "WITH w AS MATERIALIZED (DELETE FROM u RETURNING id) SELECT w.id FROM w",
    ] {
        assert_eq!(
            fields::unresolved(sql, &tables),
            Vec::<String>::new(),
            "{sql}"
        );
    }
}

// PostgreSQL truncates an identifier to 63 bytes, so two aliases that differ
// only after it name one relation, though the guard would read them apart.
#[test]
fn an_identifier_past_the_truncation_length_fails() {
    let tables = inventory(&[("t", &["id"])]);
    let kept = "a".repeat(63);
    let truncated = "a".repeat(64);
    assert_eq!(
        fields::unresolved(&format!("SELECT {kept}.id FROM t AS {kept}"), &tables),
        Vec::<String>::new()
    );
    assert_eq!(
        fields::unresolved(
            &format!("SELECT {truncated}.id FROM t AS {truncated}"),
            &tables
        ),
        vec!["an identifier longer than 63 bytes, which PostgreSQL truncates".to_owned(); 2]
    );
}

// PostgreSQL resolves a qualified name in its own query block and then
// outward, so an alias inside a subquery or another set-operation arm is no
// receiver for it.
#[test]
fn an_alias_is_visible_in_its_block_and_nested_ones() {
    let tables = inventory(&[("t", &["id", "shard", "used"]), ("u", &["id", "label"])]);
    for sql in [
        "SELECT w.used WHERE EXISTS (SELECT 1 FROM t AS w)",
        "SELECT a.id FROM t AS a WHERE a.id IN (SELECT 1 FROM u AS b) AND b.id = 1",
        "SELECT a.id FROM t AS a UNION SELECT a.used FROM u AS b",
        "SELECT t.id WHERE EXISTS (SELECT 1 FROM t)",
        // A FROM-item subquery reads its siblings only after LATERAL, and a
        // CTE body reads no alias of the main query.
        "SELECT s.n FROM t AS a CROSS JOIN (SELECT a.used AS n) AS s",
        "SELECT s.n FROM t AS a, (SELECT a.used AS n) AS s",
        "SELECT s.n FROM t AS a, (SELECT 1 AS n UNION SELECT a.used) AS s",
        "WITH c AS (SELECT a.id) SELECT c.id FROM c, t AS a",
        // An outer alias hides its table in nested blocks too.
        "SELECT a.id FROM t AS a WHERE EXISTS (SELECT 1 FROM u WHERE u.id = a.id AND t.id = 1)",
    ] {
        assert!(
            !fields::unresolved(sql, &tables).is_empty(),
            "{sql} must fail"
        );
    }
    for sql in [
        "SELECT a.id FROM t AS a WHERE EXISTS (SELECT 1 FROM u AS b WHERE b.id = a.id)",
        "SELECT a.id FROM t AS a WHERE EXISTS (SELECT 1 FROM u WHERE u.id = a.id)",
        "SELECT t.id FROM t WHERE EXISTS (SELECT 1 FROM u WHERE u.id = t.id)",
        "SELECT a.id FROM t AS a UNION SELECT b.id FROM u AS b",
        "SELECT s.n FROM t AS a CROSS JOIN LATERAL (SELECT a.used AS n) AS s",
        "SELECT s.n FROM t AS a, LATERAL (SELECT a.used AS n) AS s",
        // A FROM-item subquery still reads an enclosing query's aliases.
        "SELECT a.id FROM t AS a WHERE EXISTS (SELECT 1 FROM (SELECT a.used AS n) AS s WHERE s.n = 1)",
        "WITH c AS (SELECT a.id FROM t AS a) SELECT c.id FROM c WHERE EXISTS (SELECT 1 FROM c)",
    ] {
        assert_eq!(
            fields::unresolved(sql, &tables),
            Vec::<String>::new(),
            "{sql}"
        );
    }
}

// `a.row_to_json` calls row_to_json(a) through search_path when `a` has no
// such column, so a qualified name must be a column of its declared receiver.
#[test]
fn a_qualified_name_must_be_a_column_of_its_receiver() {
    let tables = inventory(&[("t", &["id", "shard", "used"]), ("u", &["id", "label"])]);
    for sql in [
        "SELECT a.row_to_json FROM t AS a",
        // An output alias of the same name is not a column of `a`.
        "SELECT a.row_to_json AS row_to_json FROM t AS a",
        "SELECT a.label FROM t AS a JOIN u AS b ON TRUE",
        "SELECT missing.id FROM t",
        // An alias hides the name it stands for.
        "SELECT t.id FROM t AS a",
        "SELECT t.used FROM t AS a, u",
        "WITH c AS (SELECT 1 AS id) SELECT c.id FROM c AS x",
        "INSERT INTO t AS x (id) VALUES (1) RETURNING t.id",
        "UPDATE t AS x SET used = 1 RETURNING t.used",
        "SELECT (a).row_to_json FROM t AS a",
        // Only a bare column reference or an `AS` item names a column.
        "WITH c AS (SELECT * FROM t) SELECT c.id FROM c",
        "WITH c AS (VALUES (1)) SELECT c.column1 FROM c",
        "WITH c AS (SELECT a.used::bigint FROM t AS a) SELECT c.used FROM c",
        "WITH c AS (SELECT a.used::bigint IS NULL FROM t AS a) SELECT c.used FROM c",
        "WITH c AS (SELECT 1 + a.used FROM t AS a) SELECT c.used FROM c",
        "WITH c AS (SELECT ARRAY[1, a.used, 2] AS samples FROM t AS a) SELECT c.used FROM c",
        "WITH c AS (SELECT pg_catalog.count(*) FROM t) SELECT c.count FROM c",
        // A name is declared once per statement, set-operation arms included.
        "SELECT a.id FROM t AS a, u AS a",
        "SELECT r.id FROM t AS r UNION ALL SELECT r.id FROM u AS r",
        // A declared name may not shadow a table or the catalog schema.
        "SELECT x.label FROM t AS x, u AS t",
        "SELECT 1 FROM u AS t WHERE EXISTS (SELECT 1 FROM t WHERE t.label IS NOT NULL)",
        "WITH t AS (SELECT 1 AS id) INSERT INTO t (id) VALUES (1) \
         ON CONFLICT (id) DO UPDATE SET used = EXCLUDED.id",
        "SELECT pg_catalog.row_to_json FROM t AS pg_catalog",
        // WITH ORDINALITY adds a column; it does not name one after the alias.
        "SELECT e.e, e.ordinality FROM pg_catalog.jsonb_each($1) WITH ORDINALITY AS e",
        // A column definition list types its columns through search_path.
        "SELECT r.value FROM pg_catalog.jsonb_to_record($1) AS r(value text)",
        "SELECT * FROM pg_catalog.json_to_record($1) AS (v text)",
        "SELECT * FROM ROWS FROM (pg_catalog.json_to_record($1) AS (v text)) AS r(v)",
        // A schema-qualified table does not qualify the field read from it.
        "SELECT pg_catalog.pg_class.row_to_json FROM pg_catalog.pg_class",
        // A parenthesized join hides its first operand's alias.
        "SELECT pg_catalog.row_to_json FROM (t AS pg_catalog CROSS JOIN u AS g)",
        // An alias after a joined item is still declared.
        "SELECT pg_catalog.row_to_json FROM t JOIN u ON TRUE, t AS pg_catalog",
        "SELECT a.label FROM t LEFT JOIN u ON u.id = t.id, t AS a",
        // One `:` is an array-slice bound, not a cast.
        "SELECT x.ids[1:r.f] FROM t AS r, u AS x",
        // A FROM function needs an alias; its implicit name could shadow.
        "SELECT 1 FROM pg_catalog.unnest($1::BIGINT[])",
        "SELECT j.id FROM t AS a JOIN u AS b USING (id) AS j",
        // An inheritance marker would hide the alias that follows.
        "SELECT pg_catalog.row_to_json FROM t * AS pg_catalog",
        // PostgreSQL 18 reads old and new in RETURNING as the target's rows.
        "WITH old AS (SELECT 1 AS gone) UPDATE t SET used = 1 RETURNING old.gone",
        "UPDATE t SET used = 1 RETURNING WITH (NEW AS pg_catalog) pg_catalog.row_to_json",
        // Without RECURSIVE, a CTE's body sees only the CTEs before it.
        "WITH a AS (SELECT b.n FROM b), b(n) AS (SELECT 1) SELECT n FROM a",
        "WITH a(n) AS (SELECT a.n FROM a) SELECT n FROM a",
        // A literal word names no column.
        "WITH c AS (SELECT TRUE, NULL) SELECT c.true, c.null FROM c",
        // An output alias named rows does not open ROWS FROM.
        "SELECT 1 AS rows FROM t AS pg_catalog WHERE pg_catalog.row_to_json IS NULL",
        // A write target is a physical table, never a CTE of that name.
        "WITH w AS (SELECT 1 AS extra) INSERT INTO w AS x (extra) VALUES (1) RETURNING x.extra",
        "WITH w AS (SELECT 1 AS extra) UPDATE w AS x SET extra = 1 RETURNING x.extra",
        // A nested CTE is rejected whatever it is called.
        "SELECT a.extra FROM u AS a WHERE EXISTS (WITH time AS (SELECT 1 AS extra) SELECT 1 FROM time)",
        "SELECT a.extra FROM u AS a WHERE EXISTS (WITH ordinality AS (SELECT 1 AS extra) SELECT 1 FROM ordinality)",
        // A nested WITH's CTE is visible only inside its query.
        "SELECT a.extra FROM u AS a WHERE EXISTS (WITH u AS (SELECT 1 AS extra) SELECT 1 FROM u)",
        // A schema-qualified source is no CTE of the same name.
        "WITH pg_class AS (SELECT 1 AS extra) SELECT c.extra FROM pg_catalog.pg_class AS c",
        // A row alias is no relation an inner FROM can read.
        "SELECT x.used FROM t AS pg_class \
         WHERE EXISTS (SELECT 1 FROM pg_class AS x WHERE x.used > 0)",
        // An unaliased untracked relation's row name could be any declaration.
        "SELECT pg_class.row_to_json FROM pg_catalog.pg_class \
         WHERE EXISTS (SELECT 1 FROM (SELECT 1 AS row_to_json) AS pg_class)",
        // EXCLUDED is the conflict pseudo-row only: no alias, and no row
        // before the conflict clause.
        "INSERT INTO t (id) SELECT excluded.id FROM u AS excluded",
        "INSERT INTO t (id) SELECT u.id FROM u WHERE EXCLUDED.used > 0 ON CONFLICT (id) DO NOTHING",
        // EXCLUDED needs exactly one INSERT, into a migrated table.
        "WITH i AS (INSERT INTO u (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET label = EXCLUDED.label \
         RETURNING id) INSERT INTO t (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET used = EXCLUDED.used",
    ] {
        assert!(
            !fields::unresolved(sql, &tables).is_empty(),
            "{sql} must be rejected"
        );
    }
    for sql in [
        "SELECT a.used, b.total, c.n, d.id FROM t AS a, (SELECT 1 AS total) AS b, \
         pg_catalog.unnest($1::BIGINT[]) AS c(n), u AS d",
        "WITH w(k) AS (SELECT 1), v AS (SELECT a.shard, a.id AS key FROM t AS a) \
         SELECT w.k, v.shard, v.key, t.used FROM w, v, t",
        "WITH k(x, n) AS (SELECT * FROM ROWS FROM (pg_catalog.unnest($1::BIGINT[])) WITH ORDINALITY AS r(x, n)) \
         SELECT k.x, k.n FROM k JOIN LATERAL (SELECT q.label FROM u AS q) AS l ON TRUE WHERE l.label = 'a'",
        "INSERT INTO t (id, used) VALUES (1, 1) ON CONFLICT (id) DO UPDATE SET used = EXCLUDED.used",
        "WITH changed AS (UPDATE t AS row SET used = 1 RETURNING row.id, pg_catalog.count(*) AS total) \
         SELECT changed.id, changed.total FROM changed",
        "SELECT pg_catalog.floor(1), $1::pg_catalog.text, s.func(1) FROM t AS s",
        "SELECT o.id FROM t AS o WHERE EXISTS (SELECT 1 FROM u AS r WHERE r.id = o.id)",
        "SELECT a.id FROM t AS a FOR NO KEY UPDATE OF a",
        "DELETE FROM t USING (SELECT u.id AS gone FROM u) AS expired WHERE t.id = expired.gone",
        "DELETE FROM t USING (SELECT u.id AS gone FROM u) expired WHERE t.id = expired.gone",
        "WITH s AS (SELECT 1 IS DISTINCT FROM 2 AS changed) SELECT s.changed FROM s",
        "WITH s AS (SELECT 1 IS NOT DISTINCT FROM 2 AS same) SELECT s.same FROM s",
        "SELECT a.id FROM t AS a JOIN u AS b USING (id, label)",
        "SELECT a.id FROM t AS a JOIN u AS b ON ARRAY[a.id, b.id] = $1, u AS c WHERE c.id = a.id",
        "WITH b(n) AS (SELECT 1), a AS (SELECT b.n FROM b) SELECT a.n FROM a",
        "WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT a.n FROM a) SELECT a.n FROM a",
        "SELECT s.v, s.ordinality FROM pg_catalog.unnest($1::BIGINT[]) WITH ORDINALITY AS s(v, ordinality) \
         WHERE $2::timestamp with time zone IS NOT NULL",
        "SELECT pg_catalog.floor(extract(epoch FROM pg_catalog.clock_timestamp())), a.id \
         FROM t AS a WHERE a.used IS DISTINCT FROM a.id",
        "WITH c AS (SELECT a.used::pg_catalog.text AS used, ARRAY[1, a.id] AS samples, a.id \
         FROM t AS a) SELECT c.used, c.samples, c.id FROM c",
    ] {
        assert_eq!(
            fields::unresolved(sql, &tables),
            Vec::<String>::new(),
            "{sql}"
        );
    }
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
        "SELECT public.clock_timestamp()",
        "SELECT \"clock_timestamp\"()",
        "SELECT /* note */ floor(1)",
        "SELECT $1::public.text",
        "SELECT \"PG_CATALOG\".floor(1)",
        "SELECT $1::\"char\"",
        "SELECT * FROM jsonb_to_record($1) AS (value integer)",
        "SELECT * FROM pg_catalog.unnest($1::BIGINT[]), jsonb_to_record($2) AS (v integer)",
        "SELECT $$it's ready$$, pg_catalog.floor(1)",
        "SELECT $tag$x$tag$",
        "SELECT text 'ready'",
        "SELECT CAST($1 AS pg_catalog.text)",
        "SELECT pg_catalog.length('can''t'), floor(1)",
        "SELECT /* outer /* inner */ -- */ 1, floor(1)",
        "SELECT pg_catalog.\"unnest\"($1::integer[], $2::integer[])",
        "SELECT $1::double",
        "SELECT $1::DOUBLE, $2::double precision",
        "SELECT * FROM json_to_record($1) AS (values integer)",
        // Unreserved keywords are function names outside their grammar.
        "SELECT materialized($1)",
        "SELECT 1 FROM t WHERE x = set(1)",
        "SELECT conflict(1), filter(2)",
        "SELECT 1 FROM t WHERE NOT materialized(1)",
        // A qualifier binds across whitespace and comments.
        "SELECT other_schema. greatest(1, 2)",
        "SELECT other_schema /* x */ . floor(1)",
        "SELECT 1 FROM pg_catalog . unnest($1::BIGINT[], $2::BIGINT[]) AS i(a, b)",
        "SELECT 1 FROM t JOIN u ON conflict(t.id)",
        "SELECT pg_catalog.count(*) FILTER (1)",
        "SELECT pg_catalog.count(*) over(1)",
        "SELECT 1 -- note\rFROM floor(1)",
        "SELECT text -- (\n'ready'",
        "SELECT $1::类型",
        "SELECT naïve(1)",
        "SELECT pg_catalog.length(1, /* , */ x 'y')",
        "SELECT 1 -- note\r$tag$x$tag$",
        "SELECT * FROM t, json_to_record($1) AS (insert integer)",
        "SELECT 1 FROM pg_catalog.unnest($1::BIGINT[]) WITH ORDINALITY AS x(v, n), floor(1)",
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
        "SELECT pg_catalog.length('floor(1)::text') -- count(*)\n",
        "SELECT $1::integer, $2::SMALLINT, $3::double precision, $4::DOUBLE  PRECISION[]",
        "SELECT $1::numeric(39, 0), $2::varchar(128), $3::timestamp(3)",
        "SELECT pg_catalog.set_config('statement_timeout', $1, true), $2::BIGINT",
        "SELECT pg_catalog.length('can''t'), pg_catalog.length('x'''), pg_catalog.floor(1)",
        "SELECT pg_catalog.length(/* units */ 'x'), pg_catalog.concat('a', -- next\n 'b')",
        "SELECT pg_catalog.length('é') -- naïve note\n",
        "WITH a(x) AS NOT MATERIALIZED (VALUES (1)), b AS ( SELECT 2) TABLE a",
        "INSERT INTO t (a) VALUES (1) ON CONFLICT (a) DO UPDATE SET (b, c) = (1, 2)",
        "UPDATE t AS r SET (b) = ROW(1) WHERE pg_catalog.count(*) FILTER (WHERE TRUE) > 0",
        "SELECT pg_catalog.count(*) OVER (ORDER BY 1)",
        "SELECT pg_catalog. count(*), pg_catalog /* x */ . floor(1)",
    ] {
        assert_eq!(
            unqualified(sql),
            Vec::<String>::new(),
            "{sql} must be accepted"
        );
    }
}
