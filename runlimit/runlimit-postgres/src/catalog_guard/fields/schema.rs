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
    let statements: Vec<Vec<Token>> = migrations
        .into_iter()
        .flat_map(|migration| {
            tokenize(&opaque_bodies(migration))
                .split(|token| *token == Token::Punct(';'))
                .map(<[Token]>::to_vec)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut code = MigrationCode::default();
    let mut tables = BTreeMap::new();
    for statement in &statements {
        code.check(statement);
        code.apply(statement);
        apply(statement, &mut tables);
    }
    tables
}

/// The code the migrations have defined so far, in migration order, whose
/// bodies this reader does not read: their functions, and the triggers on
/// each table.
#[derive(Default)]
struct MigrationCode {
    functions: BTreeSet<String>,
    triggers: BTreeMap<String, BTreeSet<String>>,
}

/// The statement kinds a migration may run; any other, such as `SELECT`,
/// `DO` or `CALL`, could run code with schema effects this reader cannot see.
const STATEMENT_KINDS: &[&str] = &[
    "alter", "comment", "create", "delete", "drop", "grant", "insert", "lock", "revoke", "update",
];

impl MigrationCode {
    /// Records what one statement defines, drops or renames.
    fn apply(&mut self, tokens: &[Token]) {
        if let Some(function) = defined_function(tokens) {
            self.functions.insert(function);
        } else if is(tokens, 0, "create") && is(tokens, 1, "trigger") {
            if let (Some(name), Some(table)) = (word(tokens, 2), trigger_table(tokens)) {
                self.triggers
                    .entry(table)
                    .or_default()
                    .insert(name.to_owned());
            }
        } else if is(tokens, 0, "drop") && is(tokens, 1, "trigger") {
            if let (Some(name), Some(table)) =
                (word(tokens, skip_guards(tokens, 2)), trigger_table(tokens))
                && let Some(triggers) = self.triggers.get_mut(&table)
            {
                triggers.remove(name);
            }
        } else if is(tokens, 0, "drop") && is(tokens, 1, "function") {
            if let Some(name) = word(tokens, skip_guards(tokens, 2)) {
                self.functions.remove(name);
            }
        } else if is(tokens, 0, "drop") && is(tokens, 1, "table") {
            for (start, _) in items(tokens, skip_guards(tokens, 2), tokens.len()) {
                if let Some(table) = word(tokens, start) {
                    self.triggers.remove(table);
                }
            }
        } else if is(tokens, 0, "alter") && is(tokens, 1, "table") {
            let name = skip_guards(tokens, 2);
            if let (Some(table), true, true, Some(renamed)) = (
                word(tokens, name),
                is(tokens, name + 1, "rename"),
                is(tokens, name + 2, "to"),
                word(tokens, name + 3),
            ) && let Some(triggers) = self.triggers.remove(table)
            {
                self.triggers.insert(renamed.to_owned(), triggers);
            }
        }
    }

    /// Fails on a statement that runs migration-defined code now: one of
    /// another kind, a call of a defined function outside its definition
    /// or trigger (in DML, an index, a default or a check), or DML on a
    /// table a defined trigger fires on.
    fn check(&self, tokens: &[Token]) {
        if tokens.is_empty() {
            return;
        }
        // A statement opening with punctuation, such as `(SELECT ...)`, is
        // one this reader cannot classify.
        let Some(kind) = word(tokens, 0) else {
            panic!(
                "a migration statement that opens with punctuation, which this reader cannot read"
            );
        };
        assert!(
            STATEMENT_KINDS.contains(&kind),
            "a {kind:?} migration statement, which could run code with unknown schema effects"
        );
        let defines = defined_function(tokens).is_some()
            || is(tokens, 0, "create") && is(tokens, 1, "trigger");
        if !defines {
            // A name after FUNCTION, PROCEDURE or ROUTINE is a signature, as
            // in DROP FUNCTION f(), not a call.
            let calls = (0..tokens.len()).find(|&index| {
                word(tokens, index).is_some_and(|name| self.functions.contains(name))
                    && punct(tokens, index + 1, '(')
                    && !["function", "procedure", "routine"]
                        .iter()
                        .any(|w| is(tokens, index.wrapping_sub(1), w))
            });
            assert!(
                calls.is_none(),
                "a migration statement that calls a function the migrations define"
            );
        }
        let target_at = match kind {
            "insert" if is(tokens, 1, "into") => Some(2),
            "update" => Some(skip_guards(tokens, 1)),
            "delete" if is(tokens, 1, "from") => Some(skip_guards(tokens, 2)),
            _ => None,
        };
        // A qualified target's table, not its schema, is what a trigger fires on.
        assert!(
            !target_at.is_some_and(|at| punct(tokens, at + 1, '.')),
            "migration DML on a schema-qualified table, which this reader does not resolve"
        );
        let target = target_at.and_then(|at| word(tokens, at));
        let fires = target
            .and_then(|table| self.triggers.get(table))
            .is_some_and(|triggers| !triggers.is_empty());
        assert!(
            !fires,
            "migration DML on a table whose trigger runs code this reader cannot see"
        );
    }
}

/// The table a `CREATE TRIGGER` or `DROP TRIGGER` statement names after `ON`.
fn trigger_table(tokens: &[Token]) -> Option<String> {
    let on = (2..tokens.len()).find(|&index| is(tokens, index, "on"))?;
    word(tokens, on + 1).map(str::to_owned)
}

/// The function a `CREATE [OR REPLACE] FUNCTION` statement defines.
fn defined_function(tokens: &[Token]) -> Option<String> {
    let at = if is(tokens, 1, "or") && is(tokens, 2, "replace") {
        3
    } else {
        1
    };
    (is(tokens, 0, "create") && is(tokens, at, "function")).then_some(())?;
    let name = if punct(tokens, at + 2, '.') {
        at + 3
    } else {
        at + 1
    };
    word(tokens, name).map(str::to_owned)
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
    // A cascading drop, of a column, type, domain or function, also drops
    // dependent columns this reader cannot see; `ON DELETE CASCADE` and
    // `ON UPDATE CASCADE` are referential actions, not drops.
    assert!(
        !(1..tokens.len()).any(|index| is(tokens, index, "cascade")
            && !is(tokens, index - 1, "delete")
            && !is(tokens, index - 1, "update")),
        "a cascading drop, whose dependent column removals this reader does not apply"
    );
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
        // `ALTER TABLE IF EXISTS` of an absent table is skipped; any other
        // alter of one is a migration this reader cannot follow.
        if !tables.contains_key(&table) {
            assert!(
                is(tokens, 2, "if"),
                "ALTER TABLE {table} before the table exists"
            );
            return;
        }
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
    let columns = tables.get_mut(table).expect("an altered table exists");
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
        Some("set") if is(action, 1, "schema") => {
            panic!("ALTER TABLE {table} SET SCHEMA, a move this reader does not track");
        }
        Some("drop") if !is(action, 1, "constraint") => {
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
         parent BIGINT REFERENCES t (id) ON DELETE CASCADE ON UPDATE CASCADE, \
         CONSTRAINT used_positive CHECK (used > 0));
         CREATE TABLE gone (id BIGINT);",
        "ALTER TABLE t ADD COLUMN shard SMALLINT, DROP COLUMN IF EXISTS old, \
         ADD CONSTRAINT k UNIQUE (id);
         ALTER TABLE t RENAME COLUMN used TO spent;
         ALTER TABLE t RENAME TO counters;
         DROP TABLE gone;
         CREATE TABLE IF NOT EXISTS counters (unrelated BIGINT);",
        // An IF EXISTS alter of an absent table is skipped, so the later
        // conditional create defines the table.
        "ALTER TABLE IF EXISTS later ADD COLUMN ghost BIGINT; \
         CREATE TABLE IF NOT EXISTS later (real BIGINT);",
        // A seed before a trigger exists, or after it is dropped, fires
        // nothing.
        "CREATE TABLE seeded (n BIGINT); INSERT INTO seeded (n) VALUES (1); \
         CREATE FUNCTION count_seeds() RETURNS trigger AS $$ BEGIN RETURN NEW; END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER counted AFTER INSERT ON seeded FOR EACH ROW EXECUTE FUNCTION count_seeds(); \
         DROP TRIGGER counted ON seeded; INSERT INTO seeded (n) VALUES (2); \
         DROP FUNCTION count_seeds();",
        // A function body's DDL runs when the function is called.
        "CREATE FUNCTION f() RETURNS void AS $body$ SELECT 1; ALTER TABLE counters DROP COLUMN id; $body$ \
         LANGUAGE sql; -- $$ is no body here
         CREATE FUNCTION g() RETURNS void AS $$ ALTER TABLE counters DROP COLUMN shard $$ LANGUAGE sql;",
    ]);
    assert_eq!(
        tables.keys().collect::<Vec<_>>(),
        ["counters", "later", "seeded"]
    );
    assert_eq!(tables["later"], BTreeSet::from(["real".to_owned()]));
    assert_eq!(
        tables["counters"],
        ["id", "parent", "shard", "spent"]
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
        "ALTER TABLE missing ADD COLUMN extra BIGINT;",
        "CREATE TABLE t (id BIGINT); ALTER TABLE t SET SCHEMA archive;",
        // A function body runs once something calls it now.
        "CREATE TABLE t (old BIGINT); \
         CREATE FUNCTION drop_old() RETURNS void AS $$ ALTER TABLE t DROP COLUMN old $$ LANGUAGE sql; \
         SELECT drop_old();",
        "CREATE TABLE t (old BIGINT); CREATE TABLE log (n BIGINT); \
         CREATE FUNCTION f() RETURNS bigint AS $$ SELECT 1 $$ LANGUAGE sql; \
         INSERT INTO log (n) VALUES (f());",
        "CREATE TABLE t (old BIGINT); \
         CREATE FUNCTION g() RETURNS trigger AS $$ BEGIN RETURN NEW; END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER fire AFTER INSERT ON t FOR EACH ROW EXECUTE FUNCTION g(); \
         INSERT INTO t (old) VALUES (1);",
        "CREATE TABLE t (old BIGINT); \
         CREATE FUNCTION g() RETURNS trigger AS $$ BEGIN RETURN NEW; END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER fire AFTER INSERT ON t FOR EACH ROW EXECUTE FUNCTION g(); \
         ALTER TABLE t RENAME TO u; INSERT INTO u (old) VALUES (1);",
        "CREATE TABLE t (old BIGINT); \
         CREATE FUNCTION drop_old() RETURNS void AS $$ ALTER TABLE t DROP COLUMN old $$ LANGUAGE sql; \
         (SELECT drop_old());",
        "CREATE TABLE t (old BIGINT); \
         CREATE FUNCTION g() RETURNS trigger AS $$ BEGIN RETURN NEW; END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER fire AFTER INSERT ON t FOR EACH ROW EXECUTE FUNCTION g(); \
         INSERT INTO public.t (old) VALUES (1);",
        "CREATE DOMAIN d AS BIGINT; CREATE TABLE t (old d); DROP DOMAIN d CASCADE;",
        "CREATE TYPE k AS (a BIGINT); CREATE TABLE t (old k); DROP TYPE k CASCADE;",
    ] {
        assert!(
            std::panic::catch_unwind(|| super::tables([unreadable])).is_err(),
            "{unreadable}"
        );
    }
}
