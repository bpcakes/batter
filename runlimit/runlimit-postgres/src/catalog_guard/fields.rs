//! Qualified names resolved against the relation they read.
//!
//! PostgreSQL reads `x.f` as the call `f(x)` when the relation `x` has no
//! column `f`, resolving `f` through `search_path`. So every qualified name
//! must be a column of its own receiver: a table the bundled migrations
//! create, a common table expression, or an alias of a table, CTE, subquery
//! or function, whose columns come from its column list, its select or
//! `RETURNING` output, or the table it names. `EXCLUDED` is the insert
//! target. A receiver or output the reader cannot work out fails closed.

use std::collections::{BTreeMap, BTreeSet};

use super::detector::strip_comments_and_quotes;

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
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => depth = depth.saturating_sub(1),
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
fn name_list(tokens: &[Token], open: usize) -> BTreeSet<String> {
    items(tokens, open + 1, closing(tokens, open))
        .into_iter()
        .filter_map(|(start, _)| word(tokens, start).map(str::to_owned))
        .collect()
}

/// The table and column names the migrations create, by table.
pub(super) fn tables<'a>(
    migrations: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut tables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for migration in migrations {
        let tokens = tokenize(migration);
        let mut altered: Option<String> = None;
        let mut index = 0;
        while index < tokens.len() {
            if punct(&tokens, index, ';') {
                altered = None;
            }
            if is(&tokens, index, "table") {
                let mut name = index + 1;
                while ["if", "not", "exists", "only"]
                    .iter()
                    .any(|w| is(&tokens, name, w))
                {
                    name += 1;
                }
                if let Some(table) = word(&tokens, name).map(str::to_owned) {
                    let columns = tables.entry(table.clone()).or_default();
                    if is(&tokens, index.wrapping_sub(1), "create") && punct(&tokens, name + 1, '(')
                    {
                        for (start, _) in items(&tokens, name + 2, closing(&tokens, name + 1)) {
                            if let Some(column) = word(&tokens, start)
                                && !CONSTRAINT_WORDS.contains(&column)
                            {
                                columns.insert(column.to_owned());
                            }
                        }
                    }
                    if is(&tokens, index.wrapping_sub(1), "alter") {
                        altered = Some(table);
                    }
                }
            }
            if is(&tokens, index, "add")
                && let Some(table) = &altered
            {
                let mut column = index + 1;
                while ["column", "if", "not", "exists"]
                    .iter()
                    .any(|w| is(&tokens, column, w))
                {
                    column += 1;
                }
                if let Some(name) = word(&tokens, column)
                    && !CONSTRAINT_WORDS.contains(&name)
                {
                    tables
                        .entry(table.clone())
                        .or_default()
                        .insert(name.to_owned());
                }
            }
            index += 1;
        }
    }
    tables
}

/// How a relation name gets its columns.
#[derive(Clone, Debug)]
enum Source {
    /// A table, or an explicit column list.
    Columns(BTreeSet<String>),
    /// The output of the select or DML statement in `tokens[start..end]`.
    Query(usize, usize),
    /// Another relation's name, as an alias of a table or CTE.
    Alias(String),
}

struct Scope<'t> {
    tokens: &'t [Token],
    relations: BTreeMap<String, Vec<Source>>,
    insert_target: Option<String>,
}

impl Scope<'_> {
    fn define(&mut self, name: &str, source: Source) {
        self.relations
            .entry(name.to_owned())
            .or_default()
            .push(source);
    }

    /// The columns every definition of `name` has, or `None` when `name`
    /// is unknown or any definition is unreadable.
    fn columns(&self, name: &str, depth: usize) -> Option<BTreeSet<String>> {
        let definitions = self.relations.get(name)?;
        let mut common: Option<BTreeSet<String>> = None;
        for source in definitions {
            let columns = self.source_columns(source, depth)?;
            common = Some(match common {
                None => columns,
                Some(previous) => previous.intersection(&columns).cloned().collect(),
            });
        }
        common
    }

    fn source_columns(&self, source: &Source, depth: usize) -> Option<BTreeSet<String>> {
        if depth > 16 {
            return None;
        }
        match source {
            Source::Columns(columns) => Some(columns.clone()),
            Source::Query(start, end) => self.output(*start, *end, depth + 1),
            Source::Alias(name) => self.columns(name, depth + 1),
        }
    }

    /// The output columns of the query in `tokens[start..end]`.
    fn output(&self, start: usize, end: usize, depth: usize) -> Option<BTreeSet<String>> {
        let tokens = self.tokens;
        let (list_start, list_end, select) = if is(tokens, start, "select") {
            let mut list = start + 1;
            if is(tokens, list, "distinct") && is(tokens, list + 1, "on") {
                list = closing(tokens, list + 2) + 1;
            } else if is(tokens, list, "distinct") || is(tokens, list, "all") {
                list += 1;
            }
            (
                list,
                top_level_word(tokens, list, end, &LIST_ENDS).unwrap_or(end),
                true,
            )
        } else if ["insert", "update", "delete"]
            .iter()
            .any(|w| is(tokens, start, w))
        {
            match top_level_word(tokens, start, end, &["returning"]) {
                Some(returning) => (returning + 1, end, false),
                None => return Some(BTreeSet::new()),
            }
        } else {
            return None;
        };
        let mut columns = BTreeSet::new();
        for (item_start, item_end) in items(tokens, list_start, list_end) {
            match &tokens[item_start..item_end] {
                [Token::Punct('*')] if select => {
                    columns.extend(self.star(list_end, end, depth)?);
                }
                [Token::Word(relation), Token::Punct('.'), Token::Punct('*')] => {
                    columns.extend(self.columns(relation, depth)?);
                }
                item => {
                    if let Some(name) = output_name(item) {
                        columns.insert(name);
                    }
                }
            }
        }
        Some(columns)
    }

    /// The columns of every `FROM` item from `from` to `end`.
    fn star(&self, from: usize, end: usize, depth: usize) -> Option<BTreeSet<String>> {
        let tokens = self.tokens;
        if !is(tokens, from, "from") {
            return None;
        }
        let mut columns = BTreeSet::new();
        let mut index = from + 1;
        loop {
            let item = from_item(tokens, index);
            columns.extend(match (&item.alias, item.columns.clone(), &item.source) {
                (_, Some(listed), _) => listed,
                (Some(alias), None, ItemSource::Function) => BTreeSet::from([alias.clone()]),
                (_, None, ItemSource::Query(start, finish)) => {
                    self.output(*start, *finish, depth)?
                }
                (_, None, ItemSource::Relation(name)) => self.columns(name, depth)?,
                _ => return None,
            });
            index = item.end;
            if !(punct(tokens, index, ',') && index < end) {
                return Some(columns);
            }
            index += 1;
        }
    }
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

/// The first of `words` at the top level of `tokens[start..end]`.
fn top_level_word(tokens: &[Token], start: usize, end: usize, words: &[&str]) -> Option<usize> {
    let mut depth = 0_usize;
    (start..end).find(|&index| {
        match tokens[index] {
            Token::Punct('(') => depth += 1,
            Token::Punct(')') => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && words.iter().any(|w| is(tokens, index, w))
    })
}

/// The column name PostgreSQL gives one select-list item, if any.
fn output_name(item: &[Token]) -> Option<String> {
    if let [.., Token::Word(as_word), Token::Word(alias)] = item
        && as_word == "as"
    {
        return Some(alias.clone());
    }
    // A trailing cast keeps the name of what it casts.
    let mut end = item.len();
    while let Some(colon) = item[..end]
        .windows(2)
        .rposition(|pair| pair == [Token::Punct(':'), Token::Punct(':')])
    {
        end = colon;
    }
    match &item[..end] {
        [Token::Word(name)] | [.., Token::Punct('.'), Token::Word(name)] => Some(name.clone()),
        [.., Token::Word(function), Token::Punct('(')] => Some(function.clone()),
        tokens => {
            let open = tokens.iter().position(|t| *t == Token::Punct('('))?;
            match tokens.get(open.checked_sub(1)?) {
                Some(Token::Word(function)) if closing(tokens, open) == tokens.len() - 1 => {
                    Some(function.clone())
                }
                _ => None,
            }
        }
    }
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
    columns: Option<BTreeSet<String>>,
    end: usize,
}

/// The `FROM` item starting at `index`.
fn from_item(tokens: &[Token], mut index: usize) -> FromItem {
    if is(tokens, index, "lateral") || is(tokens, index, "only") {
        index += 1;
    }
    let source = if punct(tokens, index, '(') {
        let close = closing(tokens, index);
        let source = if ["select", "with", "values"]
            .iter()
            .any(|w| is(tokens, index + 1, w))
        {
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
    let mut ordinality = false;
    if is(tokens, index, "with") && is(tokens, index + 1, "ordinality") {
        ordinality = true;
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
    let mut columns = None;
    if alias.is_some() && punct(tokens, index, '(') {
        columns = Some(name_list(tokens, index));
        index = closing(tokens, index) + 1;
    } else if ordinality && let Some(alias) = &alias {
        columns = Some(BTreeSet::from([alias.clone(), "ordinality".to_owned()]));
    }
    FromItem {
        source,
        alias,
        columns,
        end: index,
    }
}

/// Every relation the statement names: the migrations' tables, its common
/// table expressions and its `FROM`, `JOIN`, `USING`, `UPDATE` and `INTO`
/// items.
fn scope<'t>(tokens: &'t [Token], tables: &BTreeMap<String, BTreeSet<String>>) -> Scope<'t> {
    let mut scope = Scope {
        tokens,
        relations: BTreeMap::new(),
        insert_target: None,
    };
    for (table, columns) in tables {
        scope.define(table, Source::Columns(columns.clone()));
    }
    for index in 0..tokens.len() {
        if is(tokens, index, "with") {
            define_ctes(&mut scope, index + 1);
        }
        let previous = word(tokens, index.wrapping_sub(1));
        let lists = match word(tokens, index) {
            Some("from" | "using") => true,
            Some("join") => false,
            Some("update") if !matches!(previous, Some("for" | "do")) => false,
            Some("into") if previous == Some("insert") => {
                if let Some(target) = word(tokens, index + 1) {
                    scope.insert_target = Some(target.to_owned());
                    if is(tokens, index + 2, "as")
                        && let Some(alias) = word(tokens, index + 3)
                    {
                        scope.define(alias, Source::Alias(target.to_owned()));
                    }
                }
                continue;
            }
            _ => continue,
        };
        let mut item_start = index + 1;
        loop {
            let item = from_item(tokens, item_start);
            if let Some(alias) = &item.alias {
                let source = match (&item.columns, &item.source) {
                    (Some(columns), _) => Source::Columns(columns.clone()),
                    (None, ItemSource::Query(start, end)) => Source::Query(*start, *end),
                    (None, ItemSource::Relation(name)) => Source::Alias(name.clone()),
                    (None, ItemSource::Function) => {
                        Source::Columns(BTreeSet::from([alias.clone()]))
                    }
                };
                scope.define(alias, source);
            }
            if !(lists && punct(tokens, item.end, ',')) {
                break;
            }
            item_start = item.end + 1;
        }
    }
    scope
}

/// Defines the common table expressions of the `WITH` list at `index`.
fn define_ctes(scope: &mut Scope<'_>, mut index: usize) {
    let tokens = scope.tokens;
    if is(tokens, index, "recursive") {
        index += 1;
    }
    while let Some(name) = word(tokens, index).map(str::to_owned) {
        let mut next = index + 1;
        let mut listed = None;
        if punct(tokens, next, '(') {
            listed = Some(name_list(tokens, next));
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
        let source = listed.map_or(Source::Query(next + 1, close), Source::Columns);
        scope.define(&name, source);
        if !punct(tokens, close + 1, ',') {
            return;
        }
        index = close + 2;
    }
}

/// The qualified names in `sql` that are not columns of their receiver.
/// A qualified call `x.f(...)` and a `pg_catalog` name are [`unqualified`]'s
/// to judge.
///
/// [`unqualified`]: super::detector::unqualified
pub(super) fn unresolved(sql: &str, tables: &BTreeMap<String, BTreeSet<String>>) -> Vec<String> {
    let tokens = tokenize(sql);
    let scope = scope(&tokens, tables);
    let mut found = Vec::new();
    for index in 0..tokens.len() {
        let (Some(receiver), Some(field)) = (word(&tokens, index), word(&tokens, index + 2)) else {
            continue;
        };
        if !punct(&tokens, index + 1, '.')
            || receiver == "pg_catalog"
            || punct(&tokens, index + 3, '(')
            || punct(&tokens, index.wrapping_sub(1), '.')
            || punct(&tokens, index.wrapping_sub(1), ':')
        {
            continue;
        }
        let columns = if receiver == "excluded" {
            scope
                .insert_target
                .as_deref()
                .and_then(|target| scope.columns(target, 0))
        } else {
            scope.columns(receiver, 0)
        };
        match columns {
            None => found.push(format!(
                "{receiver}.{field} (cannot read {receiver}'s columns)"
            )),
            Some(columns) if !columns.contains(field) => {
                found.push(format!("{receiver}.{field} (not a column of {receiver})"));
            }
            Some(_) => {}
        }
    }
    found
}
