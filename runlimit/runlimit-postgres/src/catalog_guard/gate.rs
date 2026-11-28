//! The Clippy gate that keeps SQL text to `Statement::query`.
//!
//! `clippy.toml` disallows every sqlx entry point that accepts SQL text, and
//! Clippy resolves each call, import, rename, function pointer and trait
//! method to those definitions, so this crate cannot send SQL except through
//! the pinned exceptions below. These tests pin that inventory against sqlx
//! 0.9 (re-read sqlx's API for SQL-text parameters, `SqlSafeStr` and
//! `Execute` implementations on every sqlx upgrade) and check that only the
//! pinned sites suppress the lints. `probes` calls each listed entry point
//! under an expectation that the lint fires, so a Clippy run fails when an
//! entry stops resolving. `scripts/check_runlimit_workspace.py` keeps the
//! manifest from lowering the lints, which the packaged manifest cannot show
//! reliably; lint flags passed on the command line belong to
//! `scripts/verify.sh`.
//!
//! Every file is read through `rust_text::normalize`, so a comment, a doc
//! line or a string that quotes an attribute is not mistaken for one, and a
//! comment inside a lint list does not hide its lint.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// sqlx functions and methods that accept SQL text or turn text into SQL.
const METHODS: &[&str] = &[
    "sqlx::query",
    "sqlx::query_with",
    "sqlx::__query_with_result",
    "sqlx::query_as",
    "sqlx::query_as_with",
    "sqlx::query_scalar",
    "sqlx::query_scalar_with",
    "sqlx::__query_scalar_with_result",
    "sqlx::raw_sql",
    "sqlx::Executor::execute",
    "sqlx::Executor::execute_many",
    "sqlx::Executor::fetch",
    "sqlx::Executor::fetch_many",
    "sqlx::Executor::fetch_all",
    "sqlx::Executor::fetch_one",
    "sqlx::Executor::fetch_optional",
    "sqlx::Executor::prepare",
    "sqlx::Executor::prepare_with",
    "sqlx::Executor::describe",
    "sqlx::Statement::query",
    "sqlx::Statement::query_with",
    "sqlx::Statement::query_as",
    "sqlx::Statement::query_as_with",
    "sqlx::Statement::query_scalar",
    "sqlx::Statement::query_scalar_with",
    "sqlx::Connection::begin_with",
    "sqlx::Pool::begin_with",
    "sqlx::Pool::try_begin_with",
    "sqlx::postgres::PgConnection::copy_in_raw",
    "sqlx::postgres::PgConnection::copy_out_raw",
    "sqlx::postgres::PgPoolCopyExt::copy_in_raw",
    "sqlx::postgres::PgPoolCopyExt::copy_out_raw",
    "sqlx::SqlSafeStr::into_sql_str",
    "sqlx::Execute::sql",
    "sqlx::Transaction::begin",
    "sqlx::migrate::Migrator::new",
    "sqlx::migrate::Migrator::run",
    "sqlx::migrate::Migrator::run_to",
    "sqlx::migrate::Migrator::run_direct",
    "sqlx::migrate::Migrator::skip",
    "sqlx::migrate::Migrator::undo",
    "sqlx::migrate::Migrator::dangerous_set_table_name",
    "sqlx::migrate::Migrator::create_schema",
    "sqlx::migrate::MigrationSource::resolve",
    "sqlx::migrate::resolve_blocking",
    "sqlx::migrate::resolve_blocking_with_config",
    "sqlx::migrate::Migrate::create_schema_if_not_exists",
    "sqlx::migrate::Migrate::ensure_migrations_table",
    "sqlx::migrate::Migrate::dirty_version",
    "sqlx::migrate::Migrate::list_applied_migrations",
    "sqlx::migrate::Migrate::lock",
    "sqlx::migrate::Migrate::unlock",
    "sqlx::migrate::Migrate::apply",
    "sqlx::migrate::Migrate::revert",
    "sqlx::migrate::Migrate::skip",
    "sqlx::migrate::MigrateDatabase::create_database",
    "sqlx::migrate::MigrateDatabase::database_exists",
    "sqlx::migrate::MigrateDatabase::drop_database",
    "sqlx::migrate::MigrateDatabase::force_drop_database",
    "sqlx::testing::TestFn::run_test",
    "sqlx::testing::TestSupport::test_context",
    "sqlx::testing::TestSupport::cleanup_test",
    "sqlx::testing::TestSupport::cleanup_test_dbs",
    "sqlx::testing::TestSupport::snapshot",
];

/// sqlx types that carry SQL text or send sqlx's own: `SqlStr` has no public
/// tuple constructor, so disallowing the type also closes
/// `Transaction::begin(.., Some(text))` and a hand-built `Migration`;
/// `PgListener` builds `LISTEN` statements from channel names and
/// `PgAdvisoryLock` calls `pg_advisory_lock` through `search_path`.
const TYPES: &[&str] = &[
    "sqlx::AssertSqlSafe",
    "sqlx::SqlStr",
    "sqlx::QueryBuilder",
    "sqlx::RawSql",
    "sqlx::migrate::Migration",
    "sqlx::postgres::PgListener",
    "sqlx::postgres::PgAdvisoryLock",
    "sqlx::testing::TestArgs",
    "sqlx::testing::TestFixture",
    "sqlx::testing::TestContext",
];

/// sqlx's query macros.
const MACROS: &[&str] = &[
    "sqlx::query",
    "sqlx::query_unchecked",
    "sqlx::query_file",
    "sqlx::query_file_unchecked",
    "sqlx::query_as",
    "sqlx::query_as_unchecked",
    "sqlx::query_file_as",
    "sqlx::query_file_as_unchecked",
    "sqlx::query_scalar",
    "sqlx::query_scalar_unchecked",
    "sqlx::query_file_scalar",
    "sqlx::query_file_scalar_unchecked",
    "sqlx::sqlx_macros::expand_query",
    "sqlx::test",
];

/// The probe module, relative to `src`.
const PROBES: &str = "catalog_guard/gate/probes.rs";

#[cfg(runlimit_macro_probes)]
mod macro_probes;
mod probes;
mod rust_text;

use rust_text::normalize;

/// The gate's lints, each also matching its renamed singular spelling.
const GATE_LINTS: &[&str] = &[
    "clippy::disallowed_method",
    "clippy::disallowed_type",
    "clippy::disallowed_macro",
];

/// The groups that hold the gate's lints (`clippy-driver -Whelp`), rustc's
/// `warnings`, and the lint that fails an unmet probe.
const GATE_GROUPS: &[&str] = &[
    "clippy::all",
    "clippy::default",
    "clippy::style",
    "warnings",
    "unfulfilled_lint_expectations",
];

/// Whether one item of a lint list names a gate lint.
fn is_gate_lint(item: &str) -> bool {
    GATE_LINTS.iter().any(|lint| item.starts_with(lint))
}

/// Whether one item of a lint list names the gate, a group holding it, or a
/// lint a macro assembles.
fn names_the_gate(item: &str) -> bool {
    item.contains('$') || is_gate_lint(item) || GATE_GROUPS.contains(&item)
}

/// The only suppressions of the gate, each pinned to its whole item, by the
/// file under `src` that holds it, compared after `normalize`.
fn exceptions() -> Vec<(&'static str, String)> {
    // `sqlx::migrate!` names `sqlx::SqlStr` in its expansion, so each bundled
    // migrator carries the one type exception.
    let migrator = |name: &str, directory: &str| {
        format!(
            "#[expect(clippy::disallowed_types, \
             reason = \"sqlx::migrate! embeds each migration as a sqlx::SqlStr\")]
             pub static {name}: Migrator = sqlx::migrate!(\"{directory}\");"
        )
    };
    vec![
        // The live check asks PostgreSQL to prepare each closed-set
        // statement in a schema it creates and drops for each family.
        (
            "catalog_guard/live.rs",
            "#[expect(clippy::disallowed_methods, reason = \"PostgreSQL prepares each closed-set statement\")]
             async fn prepare(pool: &PgPool, statement: Statement) -> Result<(), sqlx::Error> {
                 let mut connection = pool.acquire().await?;
                 connection
                     .prepare(SqlSafeStr::into_sql_str(statement.sql()))
                     .await
                     .map(drop)
             }"
            .to_owned(),
        ),
        (
            "catalog_guard/live.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the live check administers its own schema\")]
             async fn administer(admin: &PgPool, sql: String) {
                 sqlx::query(AssertSqlSafe(sql))
                     .execute(admin)
                     .await
                     .expect(\"administer the check's schema\");
             }"
            .to_owned(),
        ),
        (
            "catalog_guard/live.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the live check verifies its search path\")]
             async fn current_schemas(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
                 sqlx::query_scalar(\"SELECT pg_catalog.current_schemas(false)\")
                     .fetch_one(pool)
                     .await
             }"
            .to_owned(),
        ),
        (
            "statements.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the one place SQL text reaches sqlx\")]
             pub(crate) fn query<'q>(self) -> Query<'q, Postgres, PgArguments> {
                 sqlx::query(self.sql())
             }"
            .to_owned(),
        ),
        // The one migrator run: it selects a bundled history itself and runs a
        // fresh migrator over it, so only published SQL runs, under the
        // default table and no schema.
        (
            "base.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the one place the bundled migrations run\")]
             pub(crate) async fn migrate(&self, history: BundledHistory) -> Result<(), MigrateError> {
                 let bundled = match history {
                     BundledHistory::FixedWindow => &crate::MIGRATOR,
                     BundledHistory::Gcra => &crate::GCRA_MIGRATOR,
                     BundledHistory::Attempts => &crate::attempts::ATTEMPTS_MIGRATOR,
                 };
                 let mut connection = ConnectionCancellationGuard::new(self.pool.acquire().await?);
                 let mut migrator = Migrator::with_migrations(bundled.iter().cloned().collect());
                 migrator.set_ignore_missing(true);
                 let result = migrator
                     .run_direct(None, &mut **connection.connection(), false)
                     .await;
                 connection.release(match result {
                     Ok(()) => ConnectionOutcome::Reusable(Ok(())),
                     Err(error) => ConnectionOutcome::MustClose(Err(error)),
                 })
             }"
            .to_owned(),
        ),
        ("lib.rs", migrator("MIGRATOR", "./migrations")),
        ("gcra.rs", migrator("GCRA_MIGRATOR", "./gcra-migrations")),
        (
            "attempts.rs",
            migrator("ATTEMPTS_MIGRATOR", "./attempts-migrations"),
        ),
    ]
}

fn manifest_directory() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `directory`, by its path relative to it.
fn rust_files(directory: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("source directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path
                    .strip_prefix(directory)
                    .expect("found under the source directory")
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                found.push((relative, path));
            }
        }
    }
    found.sort();
    found
}

/// The paths of the active entries in one `clippy.toml` list. Each line of
/// the list is blank, a comment, or exactly one
/// `{ path = "...", reason = "..." },` entry; anything else fails, so a
/// commented-out entry is never counted as one Clippy loaded.
fn listed(config: &str, key: &str) -> BTreeSet<String> {
    let opener = format!("{key} = [");
    let mut lines = config.lines().skip_while(|line| *line != opener);
    assert!(lines.next().is_some(), "clippy.toml has no {key} list");
    let mut paths = BTreeSet::new();
    for line in lines.map(str::trim) {
        if line == "]" {
            return paths;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((path, _)) = line
            .strip_prefix("{ path = \"")
            .and_then(|rest| rest.split_once('"'))
            .filter(|(_, rest)| rest.starts_with(", reason = \"") && rest.ends_with("\" },"))
        else {
            panic!("clippy.toml {key}: unreadable line {line}");
        };
        assert!(
            paths.insert(path.to_owned()),
            "clippy.toml {key} repeats {path}"
        );
    }
    panic!("clippy.toml {key} is not closed");
}

/// The level a lint list sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Allow,
    Expect,
    /// `warn` cannot silence a lint under `-D warnings`, except for
    /// `warnings` itself, which it turns back into warnings.
    Warn,
}

/// Each `allow(...)` or `expect(...)` lint list in `code`, whose whitespace
/// is removed, with whether it is an expectation and its items, skipping
/// `.expect(` and `::expect(` calls and a list's `reason`.
fn lint_lists(code: &str) -> Vec<(Level, Vec<&str>)> {
    let mut lists = Vec::new();
    for (level, opener) in [
        (Level::Allow, "allow("),
        (Level::Expect, "expect("),
        (Level::Warn, "warn("),
    ] {
        for (start, _) in code.match_indices(opener) {
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c == '.' || c == ':' || c == '_' || c.is_alphanumeric()) {
                continue;
            }
            let open = start + opener.len();
            let mut depth = 1;
            let mut items = Vec::new();
            let mut item = open;
            let mut end = code.len();
            for (offset, c) in code[open..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 1 => {
                        items.push(&code[item..open + offset]);
                        item = open + offset + 1;
                    }
                    _ => {}
                }
                if depth == 0 {
                    end = open + offset;
                    break;
                }
            }
            items.push(&code[item..end]);
            items.retain(|item| !item.is_empty() && !item.starts_with("reason="));
            lists.push((level, items));
        }
    }
    lists
}

fn is_identifier(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether normalized `code` holds `word` as a whole identifier.
fn has_token(code: &str, word: &str) -> bool {
    code.match_indices(word).any(|(at, _)| {
        !code[..at].chars().next_back().is_some_and(is_identifier)
            && !code[at + word.len()..]
                .chars()
                .next()
                .is_some_and(is_identifier)
    })
}

/// The contents of each `#[...]` and `#![...]` attribute in normalized `code`.
fn attributes(code: &str) -> impl Iterator<Item = &str> {
    code.match_indices('#').filter_map(move |(at, _)| {
        let rest = &code[at + 1..];
        let rest = rest.strip_prefix('!').unwrap_or(rest).strip_prefix('[')?;
        let mut depth = 1_usize;
        let end = rest.char_indices().find_map(|(offset, c)| {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(offset)
        })?;
        Some(&rest[..end])
    })
}

/// Whether an attribute's contents assign `key`, at any nesting.
fn sets(attribute: &str, key: &str) -> bool {
    attribute.match_indices(key).any(|(at, _)| {
        !attribute[..at]
            .chars()
            .next_back()
            .is_some_and(is_identifier)
            && attribute[at + key.len()..].starts_with('=')
    })
}

#[test]
fn the_scan_sees_every_way_to_load_code_from_elsewhere() {
    for loads in [
        "#[cfg_attr(unix, path = \"../x.rs\")] mod x;",
        "#[path = \"x.rs\"] mod x;",
        "include! { \"x.rs\" }",
        "use core::include as load;",
        "core::include![\"x.rs\"];",
    ] {
        let code = normalize(loads);
        assert!(
            has_token(&code, "include") || attributes(&code).any(|a| sets(a, "path")),
            "{loads}"
        );
    }
    for harmless in [
        "let path = 1; let _ = include_str![\"x.sql\"];",
        "#[doc = \"path = include\"] fn included() {}",
    ] {
        let code = normalize(harmless);
        assert!(
            !has_token(&code, "include") && !attributes(&code).any(|a| sets(a, "path")),
            "{harmless}"
        );
    }
}

#[test]
fn clippy_disallows_every_sqlx_entry_point_that_accepts_sql() {
    let config = fs::read_to_string(manifest_directory().join("clippy.toml"))
        .expect("runlimit-postgres has a clippy.toml");
    for (key, expected) in [
        ("disallowed-methods", METHODS),
        ("disallowed-types", TYPES),
        ("disallowed-macros", MACROS),
    ] {
        let expected: BTreeSet<String> = expected.iter().map(|path| (*path).to_owned()).collect();
        assert_eq!(listed(&config, key), expected, "clippy.toml {key}");
    }
}

#[test]
fn only_the_pinned_items_suppress_the_gate() {
    let source = manifest_directory().join("src");
    let mut problems = Vec::new();
    let mut excepted = BTreeSet::new();
    let exceptions = exceptions();
    for (file, path) in rust_files(&source) {
        let mut code = normalize(&fs::read_to_string(&path).expect("source is readable"));
        for (_, pinned) in exceptions.iter().filter(|(owner, _)| *owner == file) {
            let pinned = normalize(pinned);
            match code.matches(&pinned).count() {
                1 => {
                    code = code.replacen(&pinned, "", 1);
                    excepted.insert(file.clone());
                }
                count => problems.push(format!(
                    "{file}: its pinned exception appears {count} times"
                )),
            }
        }
        for (level, items) in lint_lists(&code) {
            // A probe expects a gate lint to fire; nothing may allow one, and
            // nothing may turn `warnings` back from the `-D warnings` of the
            // Clippy runs.
            let probe = file == PROBES && level == Level::Expect;
            for item in items {
                let suppresses = match level {
                    Level::Warn => item == "warnings" || item.contains('$'),
                    Level::Allow | Level::Expect => {
                        names_the_gate(item) && !(probe && is_gate_lint(item))
                    }
                };
                if suppresses {
                    problems.push(format!("{file}: suppresses `{item}`"));
                }
            }
        }
        // Code from outside `src` would escape this scan: no `include`
        // token in any form or renaming, and no attribute that sets `path`.
        if has_token(&code, "include") {
            problems.push(format!("{file}: uses include"));
        }
        if attributes(&code).any(|attribute| sets(attribute, "path")) {
            problems.push(format!("{file}: sets a module path"));
        }
    }
    // The probes are test code: catalog_guard compiles only under test.
    let root = normalize(&fs::read_to_string(source.join("lib.rs")).expect("lib.rs"));
    if !root.contains("#[cfg(test)]mod catalog_guard;") {
        problems.push("lib.rs: catalog_guard is not test-only".to_owned());
    }
    for (file, _) in &exceptions {
        if !excepted.contains(*file) {
            problems.push(format!(
                "{file}: its pinned exception is missing or changed"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The test-only inventory helpers the shared statement generator defines,
/// each by its item head and how often it appears. Any other conditional
/// compilation in statements.rs, or any at all in a family's file, could
/// give the production build other SQL than the tests check.
const TEST_HELPERS: [(&str, usize); 5] = [
    ("#[cfg(test)] pub(crate) const ALL: &[Self]", 1),
    (
        "#[cfg(test)] pub(crate) const fn name(self) -> &'static str",
        1,
    ),
    (
        "#[cfg(test)] pub(crate) const fn file(self) -> Option<&'static str>",
        2,
    ),
    ("#[cfg(test)] fn name(self) -> String", 1),
    (
        "#[cfg(test)] pub(crate) fn all() -> Vec<(String, Statement)>",
        1,
    ),
];

/// Whether normalized `code` compiles anything conditionally.
fn conditional(code: &str) -> bool {
    configuration_words(code).next().is_some()
}

/// Each identifier in normalized `code` that begins with `cfg`: `cfg` and
/// `cfg_attr`, and any other spelling that selects code by configuration,
/// such as `cfg_select!`, which the configuration checks reject outright.
fn configuration_words(code: &str) -> impl Iterator<Item = (usize, &str)> {
    code.match_indices("cfg").filter_map(|(at, _)| {
        if code[..at].chars().next_back().is_some_and(is_identifier) {
            return None;
        }
        let end = code[at..]
            .find(|character: char| !is_identifier(character))
            .map_or(code.len(), |length| at + length);
        Some((at, &code[at..end]))
    })
}

// The statement guard reads the test build's statements, so every build
// must compile the same ones: a family's file holds no conditional
// compilation, inside a macro or not; statements.rs holds only the pinned
// test helpers; and lib.rs declares the module unconditionally.
#[test]
fn every_build_compiles_the_same_statements() {
    let source = manifest_directory().join("src");
    let mut problems = Vec::new();
    for (file, path) in rust_files(&source) {
        let mut code = normalize(&fs::read_to_string(&path).expect("source is readable"));
        if file == "statements.rs" {
            for (head, count) in TEST_HELPERS {
                let head = normalize(head);
                if code.matches(&head).count() != count {
                    problems.push(format!(
                        "statements.rs: `{head}` is not there {count} times"
                    ));
                }
                code = code.replace(&head, "");
            }
        } else if !file.starts_with("statements/") {
            continue;
        }
        if conditional(&code) {
            problems.push(format!("{file}: compiles statements conditionally"));
        }
    }
    let root = normalize(&fs::read_to_string(source.join("lib.rs")).expect("lib.rs"));
    let declaration = root
        .find("mod statements;")
        .expect("lib.rs declares statements");
    let item_start = root[..declaration].rfind([';', '}']).map_or(0, |at| at + 1);
    if conditional(&root[item_start..declaration]) {
        problems.push("lib.rs: declares statements conditionally".to_owned());
    }
    assert!(problems.is_empty(), "{problems:#?}");
    for family in [
        "#[cfg(test)]statements!{A=\"x\";}",
        "macro_rules!pick{()=>{#[cfg(not(test))]statements!{A=\"y\";}};}pick!();",
        "#[cfg_attr(test,doc=\"x\")]mod inner{}",
        "const B:bool=cfg!(test);",
        "cfg_select!{test=>{statements!{A=\"x\";}}_=>{statements!{A=\"y\";}}}",
    ] {
        assert!(conditional(&normalize(family)), "{family}");
    }
}

/// The one item the macro probes' flag may compile.
const PROBE_MODULE: &str = "#[cfg(runlimit_macro_probes)] mod macro_probes;";

/// Where each attribute's contents start and end in normalized `code`.
fn attribute_spans(code: &str) -> Vec<(usize, usize)> {
    code.match_indices('#')
        .filter_map(|(at, _)| {
            let bang = usize::from(code[at + 1..].starts_with('!'));
            let start = at + 1 + bang + 1;
            code[at + 1 + bang..].starts_with('[').then_some(())?;
            let mut depth = 1_usize;
            let end = code[start..].char_indices().find_map(|(offset, c)| {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(start + offset)
            })?;
            Some((start, end))
        })
        .collect()
}

/// Whether the word at `at` sits in a `use` item, which could import a
/// configuration macro under another name.
fn in_use_item(code: &str, at: usize) -> bool {
    let item = code[..at]
        .rfind([';', '{', '}'])
        .map_or(0, |boundary| boundary + 1);
    has_token(&code[item..at], "use")
}

/// The configuration predicates in normalized `code`: each of a `cfg` or
/// `cfg_attr` attribute and a `cfg!` call. Any other configuration syntax (a
/// `cfg`-prefixed macro such as `cfg_select!`, a `cfg` word elsewhere in an
/// attribute, or one a `use` item imports) yields its word, which no check
/// accepts. An ordinary name such as `cfg_for_pool(...)` is none.
fn configuration_predicates(code: &str) -> Vec<String> {
    let attributes = attribute_spans(code);
    let mut found = Vec::new();
    for (at, word) in configuration_words(code) {
        let rest = &code[at + word.len()..];
        let in_attribute = attributes
            .iter()
            .any(|&(start, end)| (start..end).contains(&at));
        let macro_call = rest.starts_with('!');
        if !in_attribute && !macro_call && !in_use_item(code, at) {
            continue;
        }
        let arguments = match (word, in_attribute, macro_call) {
            ("cfg_attr", true, false) => rest.strip_prefix('(').map(|arguments| (true, arguments)),
            ("cfg", true, false) => rest.strip_prefix('(').map(|arguments| (false, arguments)),
            ("cfg", _, true) => rest.strip_prefix("!(").map(|arguments| (false, arguments)),
            _ => None,
        };
        let Some((attribute, arguments)) = arguments else {
            found.push(word.to_owned());
            continue;
        };
        let mut depth = 1_usize;
        let end = arguments
            .char_indices()
            .find(|&(_, character)| match character {
                '(' => {
                    depth += 1;
                    false
                }
                ')' => {
                    depth -= 1;
                    depth == 0
                }
                ',' => attribute && depth == 1,
                _ => false,
            })
            .map_or(arguments.len(), |(index, _)| index);
        found.push(arguments[..end].to_owned());
    }
    found
}

/// The top-level comma-separated parts of a normalized predicate list.
fn predicate_parts(list: &str) -> Vec<&str> {
    let mut depth = 0_usize;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, character) in list.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&list[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&list[start..]);
    parts
}

/// Whether the Clippy runs set normalized `predicate` both ways: `test`
/// through `--all-targets` and the crate's one optional feature through the
/// default and all-features runs, joined by `not`, `all` and `any`. A
/// release-only, platform-only or other branch would compile code no run
/// lints. The macro probes' flag is no such predicate: its one declaration
/// is pinned by [`PROBE_MODULE`].
fn linted(predicate: &str) -> bool {
    if predicate == "test" || predicate == normalize("feature = \"serde\"") {
        return true;
    }
    ["not(", "all(", "any("].iter().any(|combinator| {
        predicate
            .strip_prefix(combinator)
            .and_then(|inner| inner.strip_suffix(')'))
            .is_some_and(|inner| predicate_parts(inner).into_iter().all(linted))
    })
}

// The SQL entry-point gate is a lint, so it holds only for code a Clippy run
// compiles: the crate compiles conditionally only on what those runs set.
#[test]
fn clippy_lints_every_configuration_the_crate_compiles() {
    let source = manifest_directory().join("src");
    let mut problems = Vec::new();
    for (file, path) in rust_files(&source) {
        let mut code = normalize(&fs::read_to_string(&path).expect("source is readable"));
        // The probe run caps every lint and reads only the probes' reports,
        // so its flag compiles that one module and nothing else.
        if file == "catalog_guard/gate.rs" {
            let probe_module = normalize(PROBE_MODULE);
            if code.matches(&probe_module).count() != 1 {
                problems.push(format!("{file}: `{PROBE_MODULE}` is not declared once"));
            }
            code = code.replace(&probe_module, "");
        }
        for predicate in configuration_predicates(&code) {
            if !linted(&predicate) {
                problems.push(format!(
                    "{file}: compiles on `{predicate}`, which no Clippy run sets"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let code = normalize(
        "#[cfg(not(debug_assertions))] fn a() {} #[cfg_attr(test, expect(x))] fn b() {}
         const C: bool = cfg!(all(test, feature = \"serde\")); fn my_cfg() {}",
    );
    let predicates = configuration_predicates(&code);
    assert_eq!(
        predicates
            .iter()
            .map(|predicate| linted(predicate))
            .collect::<Vec<_>>(),
        [false, true, true],
        "{predicates:?}"
    );
    for unlinted in [
        "unix",
        "target_os = \"macos\"",
        "any(test, panic = \"abort\")",
        "runlimit_macro_probes",
        "all(runlimit_macro_probes, feature = \"serde\")",
    ] {
        assert!(!linted(&normalize(unlinted)), "{unlinted}");
    }
    let selected = normalize("cfg_select! { test => { const A: u8 = 1; } _ => {} }");
    assert_eq!(configuration_predicates(&selected), ["cfg_select"]);
    assert!(conditional(&selected));
    let imported = normalize("use core::cfg_select as pick; pick! { test => {} _ => {} }");
    assert_eq!(configuration_predicates(&imported), ["cfg_select"]);
    let ordinary = normalize("fn cfg_for_pool() {} fn b() { let cfg = 1; cfg_for_pool(); }");
    assert_eq!(configuration_predicates(&ordinary), Vec::<String>::new());
}

// A family can reach the generator's internal arms, so the emitting arm
// itself admits only a literal or a literal include path. rustc compiles the
// generator against a stub: a declared literal builds, and a constant handed
// straight to the emitting arm does not.
#[test]
fn the_statement_generator_admits_only_literal_text() {
    let source = fs::read_to_string(manifest_directory().join("src/statements.rs"))
        .expect("statements.rs is readable");
    let start = source
        .find("macro_rules! statements {")
        .expect("statements.rs defines the generator");
    let end = start + source[start..].find("\n}\n").expect("the generator ends") + 3;
    let generator = &source[start..end];
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "runlimit-generator-{}-{started}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create the probe directory");
    let build = |family: &str| {
        let file = directory.join("probe.rs");
        fs::write(
            &file,
            format!(
                "pub mod statements {{ #[derive(Clone, Copy)] pub struct Statement; }}\n\
                 {generator}\n\
                 pub mod family {{\n\
                 impl Id {{ pub(crate) const fn statement(self) -> crate::statements::Statement {{\n\
                 crate::statements::Statement }} }}\n\
                 {family}\n}}\n"
            ),
        )
        .expect("write the probe");
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .current_dir(manifest_directory())
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "lib",
                "--emit",
                "metadata",
                "-o",
            ])
            .arg(directory.join("probe.rmeta"))
            .arg(&file)
            .output()
            .expect("run rustc");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let declared = build("statements! { SAMPLE = \"SELECT 1\"; }");
    let constant =
        build("const TEXT: &str = \"SELECT 1\"; statements!(@emit { [] SAMPLE literal TEXT });");
    fs::remove_dir_all(&directory).expect("remove the probe directory");
    assert!(declared.0, "a declared literal builds:\n{}", declared.1);
    assert!(!constant.0, "a constant must not build");
    assert!(
        constant
            .1
            .contains("a statement's text is a string literal"),
        "{}",
        constant.1
    );
}

#[test]
fn the_suppression_scan_reads_each_lint_list() {
    let code = normalize(
        "/// Quoting #[allow(clippy::all)] here is documentation.
         #[allow(dead_code, /* a note */ warnings)] fn a() { x.expect(\"warnings\"); }
         #[cfg_attr(test, expect($lint, reason = \"r\"))] fn b() {}
         #[deny(clippy::disallowed_methods)] fn c() {}
         #[warn(warnings)] fn d() {}
         Option::expect(value, \"$1\");",
    );
    let lists = lint_lists(&code);
    assert_eq!(
        lists,
        [
            (Level::Allow, vec!["dead_code", "warnings"]),
            (Level::Expect, vec!["$lint"]),
            (Level::Warn, vec!["warnings"])
        ]
    );
    let flagged: Vec<_> = lists
        .iter()
        .flat_map(|(_, items)| items.iter().copied().filter(|item| names_the_gate(item)))
        .collect();
    assert_eq!(flagged, ["warnings", "$lint", "warnings"]);
    assert!(names_the_gate("clippy::disallowed_method"));
    assert!(!names_the_gate("clippy::pedantic"));
}

#[test]
fn the_inventory_reader_counts_only_active_entries() {
    let config = concat!(
        "x = 1\n",
        "list = [\n",
        "    { path = \"a\", reason = \"r\" },\n",
        "    # { path = \"b\", reason = \"r\" },\n",
        "]\n",
    );
    assert_eq!(listed(config, "list"), BTreeSet::from(["a".to_owned()]));
    for unreadable in [
        "list = [\n    { path = \"a\", reason = \"r\" }, # trailing\n]\n",
        "list = [\n    { path = \"a\", reason = \"r\" },\n",
    ] {
        assert!(std::panic::catch_unwind(|| listed(unreadable, "list")).is_err());
    }
}
