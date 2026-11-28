//! `FROM` items as the declaration scan reads them: their source, alias and
//! column list, the join structure between them, and the keyword uses of
//! `FROM` that hold no list.

use std::collections::BTreeSet;

use super::{Token, closing, is, is_name, items, opens_on_conflict, punct, word};

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

/// The explicit column list after a declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ColumnList {
    /// No list follows the name.
    Absent,
    /// A list of plain names.
    Names(BTreeSet<String>),
    /// A list with more than names, such as a column definition list
    /// (`AS r(value text)`), whose types this guard does not check.
    Typed,
}

/// The column list opening at `open`.
pub(super) fn column_list(tokens: &[Token], open: usize) -> ColumnList {
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
pub(super) enum ItemSource {
    Relation(String),
    Qualified(String),
    Query(usize, usize),
    Function,
    Join,
}

#[derive(Debug)]
pub(super) struct FromItem {
    pub(super) start: usize,
    pub(super) source: ItemSource,
    pub(super) alias: Option<String>,
    pub(super) columns: ColumnList,
    pub(super) end: usize,
}

/// The `FROM` item starting at `index`.
pub(super) fn from_item(tokens: &[Token], mut index: usize) -> FromItem {
    let start = index;
    if is(tokens, index, "lateral") || is(tokens, index, "only") {
        index += 1;
    }
    let source = if punct(tokens, index, '(') {
        let close = closing(tokens, index);
        // A parenthesized FROM item is a subquery or a join, whose operands'
        // aliases the declaration scan would not see.
        let source = if opens_query(tokens, index) {
            ItemSource::Query(index + 1, close)
        } else {
            ItemSource::Join
        };
        index = close + 1;
        source
    } else if is(tokens, index, "rows") && is(tokens, index + 1, "from") {
        index = closing(tokens, index + 2) + 1;
        ItemSource::Function
    } else {
        let mut name = word(tokens, index).unwrap_or_default().to_owned();
        index += 1;
        let mut qualified = false;
        if punct(tokens, index, '.') && word(tokens, index + 1).is_some() {
            name = word(tokens, index + 1).unwrap_or_default().to_owned();
            qualified = true;
            index += 2;
        }
        if punct(tokens, index, '(') {
            index = closing(tokens, index) + 1;
            ItemSource::Function
        } else if qualified {
            // A schema-qualified relation is no migrated table or CTE.
            ItemSource::Qualified(name)
        } else {
            ItemSource::Relation(name)
        }
    };
    if is(tokens, index, "with") && is(tokens, index + 1, "ordinality") {
        index += 2;
    }
    let explicit = is(tokens, index, "as");
    if explicit {
        index += 1;
    }
    // After AS any word is the alias, `AS set` included; a bare word is one
    // unless it opens a clause.
    let alias = word(tokens, index)
        .filter(|candidate| explicit || !CLAUSE_WORDS.contains(candidate))
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
        start,
        source,
        alias,
        columns,
        end: index,
    }
}

/// Whether the `FROM` at `index` ends `IS [NOT] DISTINCT FROM`, a
/// comparison; `distinct` alone, as in `SELECT 1 AS distinct FROM t`, does
/// not make it one.
pub(super) fn is_distinct_from(tokens: &[Token], index: usize) -> bool {
    let before = |back: usize| index.checked_sub(back);
    is(tokens, index, "from")
        && before(1).is_some_and(|at| is(tokens, at, "distinct"))
        && (before(2).is_some_and(|at| is(tokens, at, "is"))
            || before(2).is_some_and(|at| is(tokens, at, "not"))
                && before(3).is_some_and(|at| is(tokens, at, "is")))
}

/// Whether the parenthesis at `open` holds a join's `USING` column list:
/// names separated by commas. A `DELETE ... USING (...)` source holds a
/// query or a join instead, and is read as a FROM item.
pub(super) fn using_columns(tokens: &[Token], open: usize) -> bool {
    let close = closing(tokens, open);
    close > open + 1
        && tokens[open + 1..close]
            .iter()
            .enumerate()
            .all(|(at, token)| match token {
                Token::Word(_) => at % 2 == 0,
                Token::Punct(',') => at % 2 == 1,
                _ => false,
            })
}

/// Whether token `index` sits directly inside a call whose syntax uses
/// `FROM` for an argument: `EXTRACT`, `SUBSTRING`, `TRIM`, `OVERLAY` or
/// `POSITION`.
pub(super) fn in_call_syntax(tokens: &[Token], index: usize) -> bool {
    let mut depth = 0_usize;
    for open in (0..index).rev() {
        match tokens[open] {
            Token::Punct(')') => depth += 1,
            Token::Punct('(') if depth == 0 => {
                return word(tokens, open.wrapping_sub(1)).is_some_and(|w| {
                    ["extract", "substring", "trim", "overlay", "position"].contains(&w)
                });
            }
            Token::Punct('(') => depth -= 1,
            _ => {}
        }
    }
    false
}

/// Words that may open a join before `JOIN`.
const JOIN_WORDS: [&str; 7] = [
    "natural", "inner", "left", "right", "full", "outer", "cross",
];

/// Words that end a join condition and the FROM list holding it.
const FROM_ENDS: [&str; 13] = [
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
    "returning",
];

/// Whether the token at `index` may follow a FROM item: the end of its
/// block or statement, a comma, a join or its condition, a clause that ends
/// the list, or `SET` after an UPDATE target. Anything else, such as a
/// keyword read as a bare alias (`FROM t set, u`), would hide the rest of
/// the list.
pub(super) fn ends_from_item(tokens: &[Token], index: usize, target: bool) -> bool {
    match tokens.get(index) {
        None => true,
        Some(Token::Punct(c)) => matches!(c, ',' | ')' | ';'),
        Some(Token::Word(w)) => {
            let w = w.as_str();
            ["on", "using", "join"].contains(&w)
                || JOIN_WORDS.contains(&w)
                || FROM_ENDS.contains(&w)
                || target && w == "set"
        }
        Some(Token::Other) => false,
    }
}

/// Where the FROM item after the one ending at `index` starts, past its
/// join condition and the comma or join words, or `None` after the last.
pub(super) fn next_from_item(tokens: &[Token], mut index: usize) -> Option<usize> {
    if is(tokens, index, "on") && !opens_on_conflict(tokens, index) {
        let mut depth = 0_usize;
        index += 1;
        while index < tokens.len() {
            match tokens[index] {
                Token::Punct('(' | '[') => depth += 1,
                Token::Punct(')') if depth == 0 => return None,
                Token::Punct(')' | ']') => depth = depth.saturating_sub(1),
                Token::Punct(',') if depth == 0 => break,
                _ if depth == 0
                    && !is_name(tokens, index)
                    && (is(tokens, index, "join")
                        || JOIN_WORDS
                            .iter()
                            .chain(&FROM_ENDS)
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
    if punct(tokens, index, ',') {
        return Some(index + 1);
    }
    while JOIN_WORDS.iter().any(|w| is(tokens, index, w)) {
        index += 1;
    }
    is(tokens, index, "join").then_some(index + 1)
}

/// Whether the parenthesis at `open` holds a query, which is its own block.
pub(super) fn opens_query(tokens: &[Token], open: usize) -> bool {
    ["select", "with", "values", "insert", "update", "delete"]
        .iter()
        .any(|w| is(tokens, open + 1, w))
}
