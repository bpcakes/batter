//! Qualified names checked against the relation they read.
//!
//! PostgreSQL reads `x.f` as the call `f(x)` when the relation `x` has no
//! column `f`, resolving `f` through `search_path`. So every qualified name
//! must be a column of its receiver, and the receiver must be declared in a
//! form whose columns are stated, never inferred:
//!
//! - a table the bundled migrations leave (`schema`), by its name or through
//!   an alias (`FROM table AS x`);
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

mod schema;

pub(super) use schema::tables;

/// Words that end a `FROM` item instead of naming its alias.
const CLAUSE_WORDS: &[&str] = &[
    "and",
    "as",
    "cross",
    "do",
    "else",
    "end",
    "except",
    "fetch",
    "for",
    "from",
    "full",
    "group",
    "having",
    "inner",
    "intersect",
    "is",
    "join",
    "lateral",
    "left",
    "limit",
    "natural",
    "not",
    "offset",
    "on",
    "or",
    "order",
    "returning",
    "right",
    "select",
    "set",
    "then",
    "union",
    "using",
    "values",
    "when",
    "where",
    "window",
    "with",
];

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

/// The identifiers of a parenthesized name list opening at `open`.
/// The explicit column list after a declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ColumnList {
    /// No list follows the name.
    Absent,
    /// A list of plain names.
    Names(BTreeSet<String>),
    /// A list with more than names, such as a column definition list
    /// (`AS r(value text)`), whose types this guard does not check.
    Typed,
}

/// The column list opening at `open`.
fn column_list(tokens: &[Token], open: usize) -> ColumnList {
    items(tokens, open + 1, closing(tokens, open))
        .into_iter()
        .map(|(start, end)| match &tokens[start..end] {
            [Token::Word(name)] => Some(name.clone()),
            _ => None,
        })
        .collect::<Option<_>>()
        .map_or(ColumnList::Typed, ColumnList::Names)
}

#[derive(Clone, Debug)]
enum ItemSource {
    Relation(String),
    Query(usize, usize),
    Function,
}

#[derive(Debug)]
struct FromItem {
    source: ItemSource,
    alias: Option<String>,
    columns: ColumnList,
    end: usize,
}

/// The `FROM` item starting at `index`.
fn from_item(tokens: &[Token], mut index: usize) -> FromItem {
    if is(tokens, index, "lateral") || is(tokens, index, "only") {
        index += 1;
    }
    let source = if punct(tokens, index, '(') {
        let close = closing(tokens, index);
        let source = if opens_query(tokens, index) {
            ItemSource::Query(index + 1, close)
        } else {
            ItemSource::Function
        };
        index = close + 1;
        source
    } else if is(tokens, index, "rows") && is(tokens, index + 1, "from") {
        index = closing(tokens, index + 2) + 1;
        ItemSource::Function
    } else {
        let mut name = word(tokens, index).unwrap_or_default().to_owned();
        index += 1;
        if punct(tokens, index, '.') && word(tokens, index + 1).is_some() {
            name = word(tokens, index + 1).unwrap_or_default().to_owned();
            index += 2;
        }
        if punct(tokens, index, '(') {
            index = closing(tokens, index) + 1;
            ItemSource::Function
        } else {
            ItemSource::Relation(name)
        }
    };
    if is(tokens, index, "with") && is(tokens, index + 1, "ordinality") {
        index += 2;
    }
    if is(tokens, index, "as") {
        index += 1;
    }
    let alias = word(tokens, index)
        .filter(|candidate| !CLAUSE_WORDS.contains(candidate))
        .map(str::to_owned);
    if alias.is_some() {
        index += 1;
    }
    // Only an explicit list states the columns; `WITH ORDINALITY` adds one
    // to columns the guard does not otherwise know.
    let mut columns = ColumnList::Absent;
    if alias.is_some() && punct(tokens, index, '(') {
        columns = column_list(tokens, index);
        index = closing(tokens, index) + 1;
    }
    FromItem {
        source,
        alias,
        columns,
        end: index,
    }
}

/// Whether the parenthesis at `open` holds a query, which is its own block.
fn opens_query(tokens: &[Token], open: usize) -> bool {
    ["select", "with", "values", "insert", "update", "delete"]
        .iter()
        .any(|w| is(tokens, open + 1, w))
}

/// Where a declared name gets its columns.
#[derive(Clone, Debug)]
enum Declared {
    /// An explicit column list.
    Columns(BTreeSet<String>),
    /// An alias of the relation of this name.
    Alias(String),
    /// The output of the query in `tokens[start..end]`.
    Query(usize, usize),
    /// A relation without a column list, whose columns this guard does not
    /// infer.
    Unlisted,
}

/// Words that end a select list.
const LIST_ENDS: [&str; 10] = [
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
        (
            list,
            top_level_word(tokens, list, end, &LIST_ENDS).unwrap_or(end),
        )
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
        depth == 0 && words.iter().any(|w| is(tokens, index, w))
    })
}

/// The names a statement declares, and the problems declaring them.
struct Declarations<'t> {
    tokens: &'t [Token],
    tables: &'t BTreeMap<String, BTreeSet<String>>,
    names: BTreeMap<String, Declared>,
    problems: Vec<String>,
}

impl Declarations<'_> {
    fn declare(&mut self, name: &str, declared: Declared) {
        if ["pg_catalog", "excluded"].contains(&name) || self.tables.contains_key(name) {
            self.problems.push(format!(
                "{name} is declared, but names a table, the catalog or EXCLUDED"
            ));
        }
        if self.names.insert(name.to_owned(), declared).is_some() {
            self.problems.push(format!("{name} is declared twice"));
        }
    }

    /// The columns of the relation `name`, through aliases.
    fn columns(&self, name: &str, depth: usize) -> Option<BTreeSet<String>> {
        match self.names.get(name) {
            _ if depth > 8 => None,
            Some(Declared::Columns(columns)) => Some(columns.clone()),
            Some(Declared::Alias(target)) => self.columns(target, depth + 1),
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
        problems: Vec::new(),
    };
    for index in 0..tokens.len() {
        if is(tokens, index, "with") && !is(tokens, index + 1, "ordinality") {
            declare_ctes(&mut declarations, tokens, index + 1);
        }
        let previous = word(tokens, index.wrapping_sub(1));
        let lists = match word(tokens, index) {
            // `ROWS FROM (...)` is a single FROM item, read by `from_item`.
            Some("from") if previous == Some("rows") => continue,
            Some("from" | "using") => true,
            Some("join") => false,
            Some("update") if !matches!(previous, Some("for" | "do")) => false,
            Some("into") if previous == Some("insert") => {
                if let (Some(target), true, Some(alias)) = (
                    word(tokens, index + 1),
                    is(tokens, index + 2, "as"),
                    word(tokens, index + 3),
                ) {
                    declarations.declare(alias, Declared::Alias(target.to_owned()));
                }
                continue;
            }
            _ => continue,
        };
        let mut item_start = index + 1;
        loop {
            let item = from_item(tokens, item_start);
            if let Some(alias) = &item.alias {
                let declared = match (&item.columns, &item.source) {
                    (ColumnList::Names(columns), _) => Declared::Columns(columns.clone()),
                    (ColumnList::Typed, _) => {
                        declarations.problems.push(format!(
                            "{alias}'s column definition list types columns the guard does not check"
                        ));
                        Declared::Unlisted
                    }
                    (ColumnList::Absent, ItemSource::Relation(name)) => {
                        Declared::Alias(name.clone())
                    }
                    (ColumnList::Absent, ItemSource::Query(start, end)) => {
                        Declared::Query(*start, *end)
                    }
                    (ColumnList::Absent, ItemSource::Function) => Declared::Unlisted,
                };
                declarations.declare(alias, declared);
            }
            if !(lists && punct(tokens, item.end, ',')) {
                break;
            }
            item_start = item.end + 1;
        }
    }
    declarations
}

/// Declares the common table expressions of the `WITH` list at `index`.
fn declare_ctes(declarations: &mut Declarations<'_>, tokens: &[Token], mut index: usize) {
    if is(tokens, index, "recursive") {
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
        declarations.declare(&name, declared);
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
            (*into..end)
                .find(|&index| is(&tokens, index, "conflict") && is(&tokens, index - 1, "on"))
                .and_then(|conflict| {
                    let target = word(&tokens, into + 1)?;
                    Some((conflict..end, tables.get(target).cloned()?))
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
        if receiver == "pg_catalog" || punct(&tokens, dot.wrapping_sub(2), ':') {
            continue;
        }
        let columns = if receiver == "excluded" {
            excluded
                .as_ref()
                .filter(|(clause, _)| clause.contains(&dot))
                .map(|(_, columns)| columns.clone())
        } else {
            declarations.columns(receiver, 0)
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
