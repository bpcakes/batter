//! Qualified names resolved against the relation they read.
//!
//! PostgreSQL reads `x.f` as the call `f(x)` when the relation `x` has no
//! column `f`, resolving `f` through `search_path`. So every qualified name
//! must be a column of its own receiver, which is looked up in the query
//! block that holds the reference and then in each enclosing block: a common
//! table expression, an alias of a table, CTE, subquery or function, or a
//! table the bundled migrations leave (`schema`). `EXCLUDED` is the insert
//! target. A relation's columns come from its column list, the table it
//! names, or its select or `RETURNING` output, where an item names a column
//! only as a complete column reference or with `AS`. A field of anything but
//! a named relation, a name defined twice in one block, or a receiver or
//! output the reader cannot work out fails closed.

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

/// How a relation name gets its columns.
#[derive(Clone, Debug)]
enum Source {
    /// An explicit column list.
    Columns(BTreeSet<String>),
    /// The output of the select or DML statement in `tokens[start..end]`.
    Query(usize, usize),
    /// Another relation's name, as an alias of a table or CTE.
    Alias(String),
}

/// One relation name a query block defines, at token `at`.
struct Definition {
    name: String,
    source: Source,
    at: usize,
    /// A row name (a `FROM` or `INTO` alias), as opposed to a CTE.
    row: bool,
}

/// A query block: the statement itself, or a parenthesized query inside it.
struct Block {
    end: usize,
    parent: Option<usize>,
    definitions: Vec<Definition>,
    /// The table this block's `INSERT` writes, which `EXCLUDED` names.
    insert_target: Option<String>,
}

struct Scope<'t> {
    tokens: &'t [Token],
    tables: &'t BTreeMap<String, BTreeSet<String>>,
    blocks: Vec<Block>,
    /// The innermost block holding each token.
    block_of: Vec<usize>,
}

impl Scope<'_> {
    /// The columns of the row named `name` where token `at` stands: a row
    /// alias, else a CTE, from the innermost block outward, then a table.
    fn row(&self, name: &str, at: usize, depth: usize) -> Option<BTreeSet<String>> {
        self.lookup(name, at, true, depth)
    }

    /// The columns of the relation `name` names where token `at` stands: a
    /// CTE from the innermost block outward, then a table, never a row alias.
    fn relation(&self, name: &str, at: usize, depth: usize) -> Option<BTreeSet<String>> {
        self.lookup(name, at, false, depth)
    }

    fn lookup(&self, name: &str, at: usize, rows: bool, depth: usize) -> Option<BTreeSet<String>> {
        if depth > 32 {
            return None;
        }
        let mut block = Some(*self.block_of.get(at)?);
        while let Some(current) = block {
            let named = |row: bool| {
                let mut found = self.blocks[current]
                    .definitions
                    .iter()
                    .filter(move |definition| definition.name == name && definition.row == row);
                (found.next(), found.next())
            };
            let found = if rows {
                match named(true) {
                    (None, _) => named(false),
                    found => found,
                }
            } else {
                named(false)
            };
            match found {
                (Some(definition), None) => {
                    return match &definition.source {
                        Source::Columns(columns) => Some(columns.clone()),
                        Source::Query(start, end) => self.output(*start, *end, depth + 1),
                        Source::Alias(target) => self.relation(target, definition.at, depth + 1),
                    };
                }
                // Two of one name and kind in one block: ambiguous.
                (Some(_), Some(_)) => return None,
                (None, _) => block = self.blocks[current].parent,
            }
        }
        self.tables.get(name).cloned()
    }

    /// The columns of the table the nearest enclosing `INSERT` writes.
    fn excluded(&self, at: usize) -> Option<BTreeSet<String>> {
        let mut block = Some(*self.block_of.get(at)?);
        while let Some(current) = block {
            if let Some(target) = &self.blocks[current].insert_target {
                return self.relation(target, at, 0);
            }
            block = self.blocks[current].parent;
        }
        None
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
                [Token::Punct('*')] if select => columns.extend(self.star(list_end, end, depth)?),
                [Token::Word(relation), Token::Punct('.'), Token::Punct('*')] => {
                    columns.extend(self.row(relation, item_start, depth + 1)?);
                }
                item => columns.extend(output_name(item)),
            }
        }
        Some(columns)
    }

    /// The columns of every `FROM` item from `from` to `end`, joined by
    /// commas or `JOIN`s.
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
                    self.output(*start, *finish, depth + 1)?
                }
                (_, None, ItemSource::Relation(name)) => self.relation(name, index, depth + 1)?,
                (None, None, ItemSource::Function) => return None,
            });
            match next_from_item(tokens, item.end, end) {
                Some(next) => index = next,
                None => return Some(columns),
            }
        }
    }
}

/// Words that may open a join before `JOIN`.
const JOIN_WORDS: [&str; 7] = [
    "natural", "inner", "left", "right", "full", "outer", "cross",
];

/// Where the `FROM` item after the one ending at `index` starts, skipping a
/// join condition and the comma or join words, or `None` past the last one.
fn next_from_item(tokens: &[Token], mut index: usize, end: usize) -> Option<usize> {
    if is(tokens, index, "on") {
        let mut depth = 0_usize;
        index += 1;
        while index < end {
            match tokens[index] {
                Token::Punct('(') => depth += 1,
                Token::Punct(')') => depth = depth.saturating_sub(1),
                Token::Punct(',') if depth == 0 => break,
                _ if depth == 0
                    && (is(tokens, index, "join")
                        || JOIN_WORDS.iter().any(|w| is(tokens, index, w))
                        || LIST_ENDS
                            .iter()
                            .chain(&["for", "returning"])
                            .any(|w| is(tokens, index, w))) =>
                {
                    break;
                }
                _ => {}
            }
            index += 1;
        }
    } else if is(tokens, index, "using") && punct(tokens, index + 1, '(') {
        index = closing(tokens, index + 1) + 1;
    }
    if index >= end {
        return None;
    }
    if punct(tokens, index, ',') {
        return Some(index + 1);
    }
    while JOIN_WORDS.iter().any(|w| is(tokens, index, w)) {
        index += 1;
    }
    is(tokens, index, "join").then_some(index + 1)
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

/// The column name a select-list item gives its output: an `AS` name, or a
/// complete column reference, cast or not. Anything else names no column the
/// guard relies on.
fn output_name(item: &[Token]) -> Option<String> {
    if let [.., Token::Word(as_word), Token::Word(alias)] = item
        && as_word == "as"
    {
        return Some(alias.clone());
    }
    let cast = item
        .windows(2)
        .position(|pair| pair == [Token::Punct(':'), Token::Punct(':')])
        .unwrap_or(item.len());
    // A column keeps its name through casts, and only through casts.
    if !only_casts(&item[cast..]) {
        return None;
    }
    match &item[..cast] {
        [Token::Word(name)] | [Token::Word(_), Token::Punct('.'), Token::Word(name)] => {
            Some(name.clone())
        }
        _ => None,
    }
}

/// Whether `tokens` is nothing but casts: `::type` each, a type being words
/// (`double precision`), an optional schema, a parenthesized modifier and
/// array brackets.
fn only_casts(tokens: &[Token]) -> bool {
    let mut index = 0;
    while index < tokens.len() {
        if !(punct(tokens, index, ':') && punct(tokens, index + 1, ':')) {
            return false;
        }
        index += 2;
        if word(tokens, index).is_none() {
            return false;
        }
        while word(tokens, index).is_some()
            || punct(tokens, index, '.') && word(tokens, index + 1).is_some()
        {
            index += 1;
        }
        if punct(tokens, index, '(') {
            index = closing(tokens, index) + 1;
        }
        while punct(tokens, index, '[') {
            while index < tokens.len() && !punct(tokens, index, ']') {
                index += 1;
            }
            index += 1;
        }
    }
    true
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

/// Whether the parenthesis at `open` holds a query, which is its own block.
fn opens_query(tokens: &[Token], open: usize) -> bool {
    ["select", "with", "values", "insert", "update", "delete"]
        .iter()
        .any(|w| is(tokens, open + 1, w))
}

/// Every query block of the statement and the relations each defines.
fn scope<'t>(tokens: &'t [Token], tables: &'t BTreeMap<String, BTreeSet<String>>) -> Scope<'t> {
    let mut blocks = vec![Block {
        end: tokens.len(),
        parent: None,
        definitions: Vec::new(),
        insert_target: None,
    }];
    let mut block_of = Vec::with_capacity(tokens.len());
    let mut open = vec![0_usize];
    for index in 0..tokens.len() {
        while open.len() > 1 && index >= blocks[*open.last().expect("the statement block")].end {
            open.pop();
        }
        block_of.push(*open.last().expect("the statement block"));
        if punct(tokens, index, '(') && opens_query(tokens, index) {
            blocks.push(Block {
                end: closing(tokens, index),
                parent: open.last().copied(),
                definitions: Vec::new(),
                insert_target: None,
            });
            open.push(blocks.len() - 1);
        }
    }
    let mut scope = Scope {
        tokens,
        tables,
        blocks,
        block_of,
    };
    for index in 0..tokens.len() {
        if is(tokens, index, "with") && !is(tokens, index + 1, "ordinality") {
            define_ctes(&mut scope, index);
        }
        let previous = word(tokens, index.wrapping_sub(1));
        let lists = match word(tokens, index) {
            // `ROWS FROM (...)` is a single FROM item, read by `from_item`.
            Some("from") if previous == Some("rows") => continue,
            Some("from" | "using") => true,
            Some("join") => false,
            Some("update") if !matches!(previous, Some("for" | "do")) => false,
            Some("into") if previous == Some("insert") => {
                if let Some(target) = word(tokens, index + 1) {
                    let block = scope.block_of[index];
                    scope.blocks[block].insert_target = Some(target.to_owned());
                    if is(tokens, index + 2, "as")
                        && let Some(alias) = word(tokens, index + 3)
                    {
                        define(
                            &mut scope,
                            index,
                            alias,
                            Source::Alias(target.to_owned()),
                            true,
                        );
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
                define(&mut scope, item_start, alias, source, true);
            }
            if !(lists && punct(tokens, item.end, ',')) {
                break;
            }
            item_start = item.end + 1;
        }
    }
    scope
}

/// Defines `name`, a row alias or a CTE, in the block that holds token `at`.
fn define(scope: &mut Scope<'_>, at: usize, name: &str, source: Source, row: bool) {
    let block = scope.block_of[at];
    scope.blocks[block].definitions.push(Definition {
        name: name.to_owned(),
        source,
        at,
        row,
    });
}

/// Defines the common table expressions of the `WITH` list at `with`.
fn define_ctes(scope: &mut Scope<'_>, with: usize) {
    let tokens = scope.tokens;
    let mut index = with + 1;
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
        define(scope, with, &name, source, false);
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
    // A row or CTE named pg_catalog would be exempt below as the schema.
    if scope
        .blocks
        .iter()
        .flat_map(|block| &block.definitions)
        .any(|definition| definition.name == "pg_catalog")
    {
        found.push("a relation named pg_catalog".to_owned());
    }
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
        if receiver == "pg_catalog"
            || punct(&tokens, dot.wrapping_sub(2), '.')
            || punct(&tokens, dot.wrapping_sub(2), ':')
        {
            continue;
        }
        let columns = if receiver == "excluded" {
            scope.excluded(dot)
        } else {
            scope.row(receiver, dot, 0)
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
