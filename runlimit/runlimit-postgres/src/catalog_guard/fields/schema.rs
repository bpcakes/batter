//! The schema the bundled migrations leave: each table and its columns.

use std::collections::{BTreeMap, BTreeSet};

use super::{Token, closing, is, items, punct, tokenize, word};

/// Words that open a table constraint rather than a column definition.
const CONSTRAINT_WORDS: &[&str] = &[
    "check",
    "constraint",
    "exclude",
    "foreign",
    "like",
    "primary",
    "unique",
];

/// Applies each migration's `CREATE TABLE`, `DROP TABLE` and `ALTER TABLE`
/// `ADD`, `DROP` and `RENAME` in order, so the result names only what still
/// exists.
///
/// # Panics
///
/// On a `CREATE TABLE` without a column list or a `RENAME` this reader does
/// not apply, so the inventory is never stale.
pub(in crate::catalog_guard) fn tables<'a>(
    migrations: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut tables = BTreeMap::new();
    for migration in migrations {
        let tokens = tokenize(&opaque_bodies(migration));
        for statement in tokens.split(|token| *token == Token::Punct(';')) {
            apply(statement, &mut tables);
        }
    }
    tables
}

/// `migration` with each dollar-quoted body blanked: a function body runs
/// when called, not when the migration defines it, so its statements are no
/// schema change. Comments and quoted text are skipped while looking for
/// bodies.
fn opaque_bodies(migration: &str) -> String {
    let mut out = String::with_capacity(migration.len());
    let mut rest = migration;
    while let Some(character) = rest.chars().next() {
        let skipped = match character {
            '-' if rest.starts_with("--") => rest.find('\n').unwrap_or(rest.len()),
            '/' if rest.starts_with("/*") => rest.find("*/").map_or(rest.len(), |end| end + 2),
            '\'' | '"' => rest[1..].find(character).map_or(rest.len(), |end| end + 2),
            '$' => match dollar_tag(rest) {
                Some(tag) => {
                    let body = rest[tag.len()..]
                        .find(tag)
                        .map_or(rest.len(), |end| end + 2 * tag.len());
                    out.extend(std::iter::repeat_n(' ', body));
                    rest = &rest[body..];
                    continue;
                }
                None => character.len_utf8(),
            },
            _ => character.len_utf8(),
        };
        out.push_str(&rest[..skipped]);
        rest = &rest[skipped..];
    }
    out
}

/// The dollar-quote opener (`$$` or `$tag$`) at the start of `text`.
fn dollar_tag(text: &str) -> Option<&str> {
    let end = text[1..].find('$')? + 2;
    let tag = &text[1..end - 1];
    tag.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then(|| &text[..end])
        .filter(|_| !tag.starts_with(|c: char| c.is_ascii_digit()))
}

/// The index past `IF [NOT] EXISTS` and `ONLY` at `index`.
fn skip_guards(tokens: &[Token], mut index: usize) -> usize {
    while ["if", "not", "exists", "only"]
        .iter()
        .any(|w| is(tokens, index, w))
    {
        index += 1;
    }
    index
}

fn apply(tokens: &[Token], tables: &mut BTreeMap<String, BTreeSet<String>>) {
    // A DO block, procedure call or EXECUTE runs now, with schema effects
    // this reader cannot see.
    assert!(
        !["do", "call", "execute"].iter().any(|w| is(tokens, 0, w)),
        "a migration statement that runs code whose schema effects are unknown"
    );
    let table_ddl =
        ["create", "alter", "drop"].iter().any(|w| is(tokens, 0, w)) && is(tokens, 1, "table");
    // A schema-qualified target names a table this reader would misread.
    assert!(
        !(table_ddl && punct(tokens, skip_guards(tokens, 2) + 1, '.')),
        "table DDL with a schema-qualified target, which this reader does not apply"
    );
    // The tokenizer reads a quoted identifier as a placeholder, so a table
    // change it would misname fails instead.
    assert!(
        !(table_ddl && tokens.contains(&Token::Word("quoted_identifier".to_owned()))),
        "table DDL with a quoted identifier, which this reader does not apply"
    );
    if is(tokens, 0, "create") && is(tokens, 1, "table") {
        let name = skip_guards(tokens, 2);
        let table = word(tokens, name)
            .expect("CREATE TABLE names a table")
            .to_owned();
        assert!(
            punct(tokens, name + 1, '('),
            "CREATE TABLE {table} without a column list"
        );
        let columns = items(tokens, name + 2, closing(tokens, name + 1))
            .into_iter()
            .filter_map(|(start, _)| word(tokens, start))
            .filter(|column| !CONSTRAINT_WORDS.contains(column))
            .map(str::to_owned)
            .collect();
        // A repeated conditional create leaves the table as it is.
        let conditional = is(tokens, 2, "if");
        match tables.get(&table) {
            Some(_) if conditional => {}
            Some(_) => panic!("CREATE TABLE {table} creates an existing table"),
            None => {
                tables.insert(table, columns);
            }
        }
    } else if is(tokens, 0, "drop") && is(tokens, 1, "table") {
        for (start, end) in items(tokens, skip_guards(tokens, 2), tokens.len()) {
            assert!(
                end - start <= 2,
                "DROP TABLE with a qualified or unreadable target"
            );
            if let Some(table) = word(tokens, start) {
                tables.remove(table);
            }
        }
    } else if is(tokens, 0, "alter") && is(tokens, 1, "table") {
        let name = skip_guards(tokens, 2);
        let Some(mut table) = word(tokens, name).map(str::to_owned) else {
            return;
        };
        // `ALTER TABLE t *` reaches inheriting tables this reader does not track.
        assert!(
            !punct(tokens, name + 1, '*'),
            "ALTER TABLE {table} *, an inheritance marker this reader does not apply"
        );
        for (start, end) in items(tokens, name + 1, tokens.len()) {
            if let Some(renamed) = alter(&tokens[start..end], &table, tables) {
                table = renamed;
            }
        }
    }
}

/// Applies one `ALTER TABLE` action to `table`, returning its new name when
/// the action renames the table.
fn alter(
    action: &[Token],
    table: &str,
    tables: &mut BTreeMap<String, BTreeSet<String>>,
) -> Option<String> {
    let columns = tables.entry(table.to_owned()).or_default();
    let column_at = |index: usize| {
        let index = if is(action, index, "column") {
            index + 1
        } else {
            index
        };
        skip_guards(action, index)
    };
    match word(action, 0) {
        Some("add") if !CONSTRAINT_WORDS.iter().any(|w| is(action, 1, w)) => {
            if let Some(column) = word(action, column_at(1)) {
                columns.insert(column.to_owned());
            }
        }
        Some("drop") if !is(action, 1, "constraint") => {
            // CASCADE can also drop dependent columns, such as a generated
            // one, which this reader cannot see.
            assert!(
                !action.contains(&Token::Word("cascade".to_owned())),
                "ALTER TABLE {table}: a cascading DROP this reader does not apply"
            );
            if let Some(column) = word(action, column_at(1)) {
                columns.remove(column);
            }
        }
        Some("rename") if is(action, 1, "constraint") => {}
        Some("rename") if is(action, 1, "to") => {
            let renamed = word(action, 2).expect("RENAME TO names a table").to_owned();
            let moved = tables.remove(table).unwrap_or_default();
            tables.insert(renamed.clone(), moved);
            return Some(renamed);
        }
        Some("rename") => {
            let from = column_at(1);
            match (
                word(action, from),
                is(action, from + 1, "to"),
                word(action, from + 2),
            ) {
                (Some(old), true, Some(new)) => {
                    columns.remove(old);
                    columns.insert(new.to_owned());
                }
                _ => panic!("ALTER TABLE {table}: a RENAME this reader does not apply"),
            }
        }
        _ => {}
    }
    None
}

#[test]
fn migrations_apply_in_order_so_dropped_and_renamed_names_are_gone() {
    let tables = tables([
        "CREATE TABLE IF NOT EXISTS t (id BIGINT PRIMARY KEY, used BIGINT, old TEXT, \
         CONSTRAINT used_positive CHECK (used > 0));
         CREATE TABLE gone (id BIGINT);",
        "ALTER TABLE t ADD COLUMN shard SMALLINT, DROP COLUMN IF EXISTS old, \
         ADD CONSTRAINT k UNIQUE (id);
         ALTER TABLE t RENAME COLUMN used TO spent;
         ALTER TABLE t RENAME TO counters;
         DROP TABLE gone;
         CREATE TABLE IF NOT EXISTS counters (unrelated BIGINT);",
        // A function body's DDL runs when the function is called.
        "CREATE FUNCTION f() RETURNS void AS $body$ SELECT 1; ALTER TABLE counters DROP COLUMN id; $body$ \
         LANGUAGE sql; -- $$ is no body here
         CREATE FUNCTION g() RETURNS void AS $$ ALTER TABLE counters DROP COLUMN shard $$ LANGUAGE sql;",
    ]);
    assert_eq!(tables.keys().collect::<Vec<_>>(), ["counters"]);
    assert_eq!(
        tables["counters"],
        ["id", "shard", "spent"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
    for unreadable in [
        "CREATE TABLE t (id BIGINT); ALTER TABLE t RENAME id",
        "CREATE TABLE t (failures BIGINT); ALTER TABLE t DROP COLUMN \"failures\"",
        "CREATE TABLE t (failures BIGINT); DO $$ BEGIN ALTER TABLE t DROP COLUMN failures; END $$;",
        "CREATE TABLE t (failures BIGINT); CALL drop_failures();",
        "CREATE TABLE t (old BIGINT); ALTER TABLE elsewhere.t DROP COLUMN old;",
        "CREATE TABLE t (old BIGINT); DROP TABLE elsewhere.t;",
        "CREATE TABLE t (key BIGINT, shard SMALLINT GENERATED ALWAYS AS (key % 2) STORED); \
         ALTER TABLE t DROP COLUMN key CASCADE;",
        "CREATE TABLE t (old BIGINT); ALTER TABLE t * DROP COLUMN old;",
    ] {
        assert!(
            std::panic::catch_unwind(|| super::tables([unreadable])).is_err(),
            "{unreadable}"
        );
    }
}
