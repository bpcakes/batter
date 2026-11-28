//! Qualified names checked against the relation they read.
//!
//! PostgreSQL reads `x.f` as the call `f(x)` when the relation `x` has no
//! column `f`, resolving `f` through `search_path`. So every qualified name
//! must be a column of its receiver, and the receiver must be declared in a
//! form whose columns are stated, never inferred:
//!
//! - a table the statement's own family installs, as
//!   `migrated_columns.txt` lists it, by its name or through an alias
//!   (`FROM table AS x`); a write target (`INSERT INTO`, `UPDATE`,
//!   `DELETE FROM`) is always such a table, even beside a CTE of its name;
//! - a common table expression or `FROM` item with an explicit column list
//!   (`name(a, b) AS (...)`, `... AS x(a, b)`), or an alias of one;
//! - a common table expression or subquery without one, whose columns are
//!   the select or `RETURNING` items that name themselves: a bare column
//!   reference (`col`, `x.col`) or an item ending in `AS name`. Any other
//!   item, `*` included, names no column, so a reference to it fails;
//! - `EXCLUDED`, in a statement with exactly one `INSERT`, into a migrated
//!   table.
//!
//! Each declared name is declared once per statement and names neither a
//! migrated table nor `pg_catalog`, so it means one relation wherever it
//! appears and no query-block scoping is needed. A field of anything but a
//! declared relation fails.

use std::collections::{BTreeMap, BTreeSet};

use super::detector::strip_comments_and_quotes;

mod blocks;
mod from_items;
mod keywords;

use blocks::Blocks;
use from_items::{
    ColumnList, FromItem, ItemSource, column_list, ends_from_item, from_item, in_call_syntax,
    is_distinct_from, next_from_item, opens_query, using_columns,
};
use keywords::{is_name, is_with_modifier, opens_command, opens_on_conflict, unread_forms};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Punct(char),
    Other,
}

/// Comment- and string-stripped, lowercased SQL as tokens.
fn tokenize(sql: &str) -> Vec<Token> {
    let code = strip_comments_and_quotes(sql).to_ascii_lowercase();
    let mut tokens = Vec::new();
    let mut chars = code.char_indices().peekable();
    while let Some((start, character)) = chars.next() {
        if character.is_ascii_alphabetic() || character == '_' {
            let mut end = start + 1;
            while let Some(&(at, next)) = chars.peek() {
                if !(next.is_ascii_alphanumeric() || next == '_' || next == '$') {
                    break;
                }
                end = at + next.len_utf8();
                chars.next();
            }
            tokens.push(Token::Word(code[start..end].to_owned()));
        } else if character.is_ascii_digit() || character == '$' {
            while chars
                .peek()
                .is_some_and(|(_, next)| next.is_ascii_alphanumeric())
            {
                chars.next();
            }
            tokens.push(Token::Other);
        } else if "(),.;*:[]".contains(character) {
            tokens.push(Token::Punct(character));
        } else if !character.is_whitespace() {
            tokens.push(Token::Other);
        }
    }
    tokens
}

fn word(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index) {
        Some(Token::Word(word)) => Some(word),
        _ => None,
    }
}

fn is(tokens: &[Token], index: usize, expected: &str) -> bool {
    word(tokens, index) == Some(expected)
}

fn punct(tokens: &[Token], index: usize, expected: char) -> bool {
    tokens.get(index) == Some(&Token::Punct(expected))
}

/// The index of the parenthesis closing the one at `open`.
fn closing(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}

/// The top-level comma-separated items of `tokens[start..end]`.
fn items(tokens: &[Token], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut depth = 0_usize;
    let mut item = start;
    for (index, token) in tokens.iter().enumerate().take(end).skip(start) {
        match token {
            Token::Punct('(' | '[') => depth += 1,
            Token::Punct(')' | ']') => depth = depth.saturating_sub(1),
            Token::Punct(',') if depth == 0 => {
                found.push((item, index));
                item = index + 1;
            }
            _ => {}
        }
    }
    found.push((item, end));
    found
}

/// Where a declared name gets its columns.
#[derive(Clone, Debug)]
enum Declared {
    /// An explicit column list.
    Columns(BTreeSet<String>),
    /// An alias of the relation of this name.
    Alias(String),
    /// An alias of a write target, `INSERT INTO`, `UPDATE` or `DELETE FROM`,
    /// which PostgreSQL resolves as a physical table even beside a CTE of
    /// the same name.
    Target(String),
    /// The output of the query in `tokens[start..end]`.
    Query(usize, usize),
    /// A relation without a column list, whose columns this guard does not
    /// infer.
    Unlisted,
}

/// Words that end a select list.
const LIST_ENDS: [&str; 13] = [
    "from",
    "where",
    "group",
    "having",
    "window",
    "union",
    "intersect",
    "except",
    "order",
    "limit",
    "offset",
    "fetch",
    "for",
];

/// The columns that name themselves in the select or `RETURNING` list of
/// the query in `tokens[start..end]`, or `None` for any other query.
fn output_names(tokens: &[Token], start: usize, end: usize) -> Option<BTreeSet<String>> {
    let (list, list_end) = if is(tokens, start, "select") {
        let mut list = start + 1;
        if is(tokens, list, "distinct") && is(tokens, list + 1, "on") {
            list = closing(tokens, list + 2) + 1;
        } else if is(tokens, list, "distinct") || is(tokens, list, "all") {
            list += 1;
        }
        // The FROM of `IS DISTINCT FROM` is a comparison, not the FROM clause.
        let mut from = list;
        let list_end = loop {
            match top_level_word(tokens, from, end, &LIST_ENDS) {
                // A field or label is a name (`k.limit`, `AS limit`), and
                // `WITHIN GROUP (...)` belongs to an aggregate call.
                Some(at)
                    if is_distinct_from(tokens, at)
                        || is_name(tokens, at)
                        || is(tokens, at, "group") && is(tokens, at - 1, "within") =>
                {
                    from = at + 1;
                }
                found => break found.unwrap_or(end),
            }
        };
        (list, list_end)
    } else if ["insert", "update", "delete"]
        .iter()
        .any(|w| is(tokens, start, w))
    {
        (
            top_level_word(tokens, start, end, &["returning"]).map_or(end, |at| at + 1),
            end,
        )
    } else {
        return None;
    };
    Some(
        items(tokens, list, list_end)
            .into_iter()
            .filter_map(|(item, item_end)| match &tokens[item..item_end] {
                [.., Token::Word(keyword), Token::Word(alias)] if keyword == "as" => {
                    Some(alias.clone())
                }
                // A literal word names no column: PostgreSQL calls it ?column?.
                [Token::Word(name)] if ["true", "false", "null"].contains(&name.as_str()) => None,
                [Token::Word(name)] | [Token::Word(_), Token::Punct('.'), Token::Word(name)] => {
                    Some(name.clone())
                }
                _ => None,
            })
            .collect(),
    )
}

/// The first of `words` at the top level of `tokens[start..end]`.
fn top_level_word(tokens: &[Token], start: usize, end: usize, words: &[&str]) -> Option<usize> {
    let mut depth = 0_usize;
    (start..end).find(|&index| {
        match tokens[index] {
            Token::Punct('(' | '[') => depth += 1,
            Token::Punct(')' | ']') => depth = depth.saturating_sub(1),
            _ => {}
        }
        // A field or label (`x.returning`, `AS returning`) bounds nothing.
        depth == 0 && !is_name(tokens, index) && words.iter().any(|w| is(tokens, index, w))
    })
}

/// The names a statement declares, and the problems declaring them.
struct Declarations<'t> {
    tokens: &'t [Token],
    tables: &'t BTreeMap<String, BTreeSet<String>>,
    names: BTreeMap<String, Declared>,
    /// The names that are common table expressions, the only declared names
    /// a FROM item can read as a relation.
    ctes: BTreeSet<String>,
    /// Each CTE in declaration order, with its body's token range, and
    /// whether the list is recursive, which lets any CTE read any other.
    cte_bodies: Vec<(String, usize, usize)>,
    recursive: bool,
    /// The `FROM` of each `ROWS FROM` a FROM-list walk read as one item, so
    /// the scan does not take it for a FROM clause of its own.
    rows_from: BTreeSet<usize>,
    /// The query blocks, and the block declaring each name.
    blocks: Blocks,
    scopes: BTreeMap<String, usize>,
    /// The tables and CTEs that appear unaliased, with the blocks they
    /// appear in: their own names may qualify a field there and in nested
    /// blocks. An alias hides the name it stands for.
    visible: BTreeMap<String, BTreeSet<usize>>,
    problems: Vec<String>,
}

impl Declarations<'_> {
    /// Declares `name`, at token `at`, in that token's block.
    fn declare(&mut self, name: &str, declared: Declared, at: usize) {
        self.scopes.insert(name.to_owned(), self.blocks.of(at));
        if ["pg_catalog", "excluded", "old", "new"].contains(&name)
            || self.tables.contains_key(name)
        {
            self.problems.push(format!(
                "{name} is declared, but names a table, the catalog, EXCLUDED or a RETURNING row"
            ));
        }
        if self.names.insert(name.to_owned(), declared).is_some() {
            self.problems.push(format!("{name} is declared twice"));
        }
    }

    /// Records that the table or CTE `name` appears unaliased at token `at`.
    fn show(&mut self, name: &str, at: usize) {
        let block = self.blocks.of(at);
        self.visible
            .entry(name.to_owned())
            .or_default()
            .insert(block);
    }

    /// The columns of the receiver `name` used at token `at`: an alias, or a
    /// table or CTE that appears unaliased, declared in that token's block
    /// or one enclosing it. `FROM t AS a` hides `t`, and an alias inside
    /// `EXISTS (...)` is no receiver outside it.
    fn receiver_columns(&self, name: &str, at: usize) -> Option<BTreeSet<String>> {
        let used = self.blocks.of(at);
        let alias = self.names.contains_key(name) && !self.ctes.contains(name);
        let in_scope = if alias {
            self.scopes
                .get(name)
                .is_some_and(|&declared| self.blocks.sees(used, declared))
        } else {
            self.visible.get(name).is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|&declared| self.blocks.sees(used, declared))
            })
        };
        if in_scope {
            self.columns(name, 0)
        } else {
            None
        }
    }

    /// The columns of the relation `name`, through aliases.
    fn columns(&self, name: &str, depth: usize) -> Option<BTreeSet<String>> {
        match self.names.get(name) {
            _ if depth > 8 => None,
            Some(Declared::Columns(columns)) => Some(columns.clone()),
            // An alias reads a CTE or a migrated table, never a row alias.
            Some(Declared::Alias(target)) if self.ctes.contains(target) => {
                self.columns(target, depth + 1)
            }
            Some(Declared::Alias(target)) => self.tables.get(target).cloned(),
            Some(Declared::Target(table)) => self.tables.get(table).cloned(),
            Some(Declared::Query(start, end)) => output_names(self.tokens, *start, *end),
            Some(Declared::Unlisted) => None,
            None => self.tables.get(name).cloned(),
        }
    }
}

/// Every name the statement declares: its common table expressions and its
/// `FROM`, `JOIN`, `USING`, `UPDATE` and `INSERT INTO` aliases.
fn declarations<'t>(
    tokens: &'t [Token],
    tables: &'t BTreeMap<String, BTreeSet<String>>,
) -> Declarations<'t> {
    let mut declarations = Declarations {
        tokens,
        tables,
        names: BTreeMap::new(),
        ctes: BTreeSet::new(),
        cte_bodies: Vec::new(),
        recursive: false,
        rows_from: BTreeSet::new(),
        blocks: Blocks::new(tokens),
        scopes: BTreeMap::new(),
        visible: BTreeMap::new(),
        problems: Vec::new(),
    };
    for index in 0..tokens.len() {
        if is_name(tokens, index) {
            continue;
        }
        if is(tokens, index, "with") && !is_with_modifier(tokens, index) {
            declare_ctes(&mut declarations, tokens, index + 1);
        }
        let previous = word(tokens, index.wrapping_sub(1));
        let lists = match word(tokens, index) {
            // `ROWS FROM (...)` is a single FROM item, which the walk of its
            // FROM list already read; a column named rows before a FROM
            // clause (`SELECT 1, rows FROM ...`) opens no such item.
            // `IS DISTINCT FROM` and `EXTRACT(field FROM ...)` and its kin
            // hold no FROM list.
            Some("from") if declarations.rows_from.contains(&index) => continue,
            Some("from") if is_distinct_from(tokens, index) || in_call_syntax(tokens, index) => {
                continue;
            }
            // A join's `USING (columns)` holds names; `USING (query)` is a
            // DELETE source, read as a FROM item.
            Some("using") if punct(tokens, index + 1, '(') && using_columns(tokens, index + 1) => {
                continue;
            }
            // A FROM or USING list is walked whole, joins included.
            Some("from") if previous == Some("delete") && opens_command(tokens, index - 1) => false,
            Some("from" | "using") => true,
            Some("update") if opens_command(tokens, index) => false,
            Some("into") if previous == Some("insert") && opens_command(tokens, index - 1) => {
                // The target is a table the family installs, aliased or not.
                match word(tokens, index + 1) {
                    Some(target) if punct(tokens, index + 2, '.') => declarations
                        .problems
                        .push(format!("{target}. is a schema-qualified INSERT target")),
                    Some(target) if !declarations.tables.contains_key(target) => declarations
                        .problems
                        .push(format!("{target} is written, but is no migrated table")),
                    Some(_) => {}
                    None => declarations
                        .problems
                        .push("an INSERT target the guard cannot read".to_owned()),
                }
                match (
                    word(tokens, index + 1),
                    is(tokens, index + 2, "as"),
                    word(tokens, index + 3),
                ) {
                    (Some(target), true, Some(alias)) => {
                        declarations.declare(alias, Declared::Target(target.to_owned()), index);
                    }
                    // INSERT takes an alias only after AS.
                    (Some(target), false, _) => declarations.show(target, index),
                    _ => {}
                }
                continue;
            }
            _ => continue,
        };
        let mut item_start = index + 1;
        loop {
            let item = from_item(tokens, item_start);
            if let Some(from) = item.rows_from {
                declarations.rows_from.insert(from);
            }
            // UPDATE and DELETE FROM name a write target, not a FROM source.
            record_from_item(&mut declarations, tokens, &item, !lists);
            match next_from_item(tokens, item.end) {
                Some(next) if lists => item_start = next,
                _ => break,
            }
        }
    }
    declarations
}

impl Declarations<'_> {
    /// Whether the CTE `name`, read at token `at`, is declared too late for
    /// it: in a nonrecursive list a CTE's body sees only the CTEs before it.
    fn reads_ahead(&self, name: &str, at: usize) -> bool {
        let position = |wanted: &str| self.cte_bodies.iter().position(|(cte, ..)| cte == wanted);
        let container = self
            .cte_bodies
            .iter()
            .position(|(_, start, end)| (*start..*end).contains(&at));
        match (self.recursive, container, position(name)) {
            (false, Some(container), Some(read)) => read >= container,
            _ => false,
        }
    }
}

/// Declares one FROM item's alias and records what keeps it from being read.
fn record_from_item(
    declarations: &mut Declarations<'_>,
    tokens: &[Token],
    item: &FromItem,
    target: bool,
) {
    // A FROM-item subquery reads its sibling FROM items only after LATERAL.
    if let ItemSource::Query(start, _) = item.source
        && !is(tokens, item.start, "lateral")
    {
        declarations.blocks.skip_level(start - 1);
    }
    let problem =
        |declarations: &mut Declarations<'_>, problem: String| declarations.problems.push(problem);
    if let ItemSource::Relation(name) = &item.source
        && declarations.reads_ahead(name, item.start)
    {
        problem(
            declarations,
            format!(
                "{name} is read before its nonrecursive declaration, so it names another relation"
            ),
        );
    }
    if matches!(item.source, ItemSource::Join) {
        problem(declarations, "a parenthesized join".to_owned());
    }
    // `table *` and other unread punctuation would hide an alias.
    if !ends_from_item(tokens, item.end, target) {
        problem(declarations, "unread tokens in a FROM item".to_owned());
    }
    // Every relation an item reads or writes is a table the statement's
    // family installs or, when read, a CTE of this statement, whether or
    // not it is aliased; a write target is always such a table.
    match &item.source {
        ItemSource::Relation(name) if target && !declarations.tables.contains_key(name) => {
            problem(
                declarations,
                format!("{name} is written, but is no migrated table"),
            );
        }
        ItemSource::Relation(name)
            if !declarations.tables.contains_key(name) && !declarations.ctes.contains(name) =>
        {
            problem(
                declarations,
                format!("{name} in FROM is neither a migrated table nor a CTE"),
            );
        }
        ItemSource::Qualified(name) => problem(
            declarations,
            format!("{name} in FROM is schema-qualified, so it is no migrated table or CTE"),
        ),
        _ => {}
    }
    let Some(alias) = &item.alias else {
        match &item.source {
            ItemSource::Relation(name) => declarations.show(name, item.start),
            // A function's implicit row name could shadow another relation.
            ItemSource::Function => problem(
                declarations,
                "a function in FROM without an alias".to_owned(),
            ),
            _ => {}
        }
        return;
    };
    let declared = match (&item.columns, &item.source) {
        (ColumnList::Names(columns), _) => Declared::Columns(columns.clone()),
        (ColumnList::Typed, _) => {
            problem(
                declarations,
                format!("{alias}'s column definition list types columns the guard does not check"),
            );
            Declared::Unlisted
        }
        (ColumnList::Absent, ItemSource::Relation(name)) if target => {
            Declared::Target(name.clone())
        }
        (ColumnList::Absent, ItemSource::Relation(name)) => Declared::Alias(name.clone()),
        (ColumnList::Absent, ItemSource::Qualified(name)) => {
            problem(
                declarations,
                format!("{name} in FROM is schema-qualified, so it is no migrated table or CTE"),
            );
            Declared::Unlisted
        }
        (ColumnList::Absent, ItemSource::Query(start, end)) => Declared::Query(*start, *end),
        (ColumnList::Absent, ItemSource::Function | ItemSource::Join) => Declared::Unlisted,
    };
    declarations.declare(alias, declared, item.start);
}

/// Declares the common table expressions of the `WITH` list at `index`.
fn declare_ctes(declarations: &mut Declarations<'_>, tokens: &[Token], mut index: usize) {
    if is(tokens, index, "recursive") {
        declarations.recursive = true;
        index += 1;
    }
    while let Some(name) = word(tokens, index).map(str::to_owned) {
        let mut next = index + 1;
        let mut listed = ColumnList::Absent;
        if punct(tokens, next, '(') {
            listed = column_list(tokens, next);
            next = closing(tokens, next) + 1;
        }
        if !is(tokens, next, "as") {
            return;
        }
        next += 1;
        for modifier in ["not", "materialized"] {
            if is(tokens, next, modifier) {
                next += 1;
            }
        }
        if !punct(tokens, next, '(') {
            return;
        }
        let close = closing(tokens, next);
        declarations.blocks.isolate(next);
        let declared = match listed {
            ColumnList::Absent => Declared::Query(next + 1, close),
            ColumnList::Names(columns) => Declared::Columns(columns),
            ColumnList::Typed => {
                declarations
                    .problems
                    .push(format!("{name}'s column list holds more than names"));
                Declared::Unlisted
            }
        };
        declarations.ctes.insert(name.clone());
        declarations
            .cte_bodies
            .push((name.clone(), next + 1, close));
        declarations.declare(&name, declared, index);
        if !punct(tokens, close + 1, ',') {
            return;
        }
        index = close + 2;
    }
}

/// The end of the parenthesized query holding token `index`, or of the
/// statement.
fn enclosing_end(tokens: &[Token], index: usize) -> usize {
    let mut depth = 0_usize;
    for open in (0..index).rev() {
        match tokens[open] {
            Token::Punct(')') => depth += 1,
            Token::Punct('(') if depth == 0 => return closing(tokens, open),
            Token::Punct('(') => depth -= 1,
            _ => {}
        }
    }
    tokens.len()
}

/// The qualified names in `sql` that are not columns of their receiver, and
/// the declarations that break the rules above. A qualified call `x.f(...)`
/// and a `pg_catalog` name are [`unqualified`]'s to judge.
///
/// [`unqualified`]: super::detector::unqualified
pub(super) fn unresolved(sql: &str, tables: &BTreeMap<String, BTreeSet<String>>) -> Vec<String> {
    let tokens = tokenize(sql);
    let declarations = declarations(&tokens, tables);
    let mut found = declarations.problems.clone();
    found.extend(unread_forms(&tokens));
    // `AS (...)` around anything but a query is a column definition list,
    // with or without an alias, `ROWS FROM` items included.
    for index in 0..tokens.len() {
        if is(&tokens, index, "as")
            && punct(&tokens, index + 1, '(')
            && !opens_query(&tokens, index + 1)
        {
            found
                .push("a column definition list types columns the guard does not check".to_owned());
        }
    }
    // EXCLUDED is the target of the statement's one INSERT, inside that
    // INSERT's ON CONFLICT clause.
    let inserts: Vec<usize> = (1..tokens.len())
        .filter(|&index| is(&tokens, index, "into") && is(&tokens, index - 1, "insert"))
        .collect();
    let excluded = match inserts.as_slice() {
        [into] => {
            let end = enclosing_end(&tokens, *into);
            // Only the DO UPDATE action reads EXCLUDED: not the conflict
            // target, DO NOTHING or the RETURNING list after it.
            (*into..end)
                .find(|&index| opens_on_conflict(&tokens, index))
                .and_then(|conflict| {
                    let action = (conflict..end).find(|&index| {
                        is(&tokens, index, "do") && is(&tokens, index + 1, "update")
                    })?;
                    let action_end =
                        top_level_word(&tokens, action, end, &["returning"]).unwrap_or(end);
                    let target = word(&tokens, into + 1)?;
                    Some((action..action_end, tables.get(target).cloned()?))
                })
        }
        _ => None,
    };
    for dot in 1..tokens.len() {
        let Some(field) = word(&tokens, dot + 1) else {
            continue;
        };
        if !punct(&tokens, dot, '.') || punct(&tokens, dot + 2, '(') {
            continue;
        }
        let Some(receiver) = word(&tokens, dot - 1) else {
            found.push(format!(
                "({{expression}}).{field} (a field of an expression)"
            ));
            continue;
        };
        if punct(&tokens, dot.wrapping_sub(2), '.') {
            // `schema.table.f` qualifies the table, not `f`.
            found.push(format!("{receiver}.{field} (a field of a qualified name)"));
            continue;
        }
        // `::schema.type` is a cast; one `:` is an array-slice bound.
        let cast =
            punct(&tokens, dot.wrapping_sub(2), ':') && punct(&tokens, dot.wrapping_sub(3), ':');
        if receiver == "pg_catalog" || cast {
            continue;
        }
        let columns = if receiver == "excluded" {
            excluded
                .as_ref()
                .filter(|(clause, _)| clause.contains(&dot))
                .map(|(_, columns)| columns.clone())
        } else {
            declarations.receiver_columns(receiver, dot)
        };
        match columns {
            None => found.push(format!(
                "{receiver}.{field} ({receiver} is not a table or a relation with a column list)"
            )),
            Some(columns) if !columns.contains(field) => {
                found.push(format!("{receiver}.{field} (not a column of {receiver})"));
            }
            Some(_) => {}
        }
    }
    found
}
