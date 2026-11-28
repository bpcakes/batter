//! Where a SQL keyword is syntax and where it is only a name, and the
//! keyword forms the declaration scan does not read.
//!
//! PostgreSQL accepts every keyword as a field after `.` and as a label
//! after `AS`, and unreserved ones as aliases, so a word opens a clause or a
//! command only in its own position.

use super::{Token, closing, from_items::using_columns, is, punct, word};

/// The index of the parenthesis opening the one closing at `close`.
pub(super) fn opening(tokens: &[Token], close: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for index in (0..=close).rev() {
        match tokens[index] {
            Token::Punct(')') => depth += 1,
            Token::Punct('(') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether the word at `index` is a name whatever it spells: a field after
/// `.` or a label after `AS`, where PostgreSQL accepts every keyword
/// (`k.limit`, `SELECT 1 AS from`). Such a word never bounds a clause.
///
/// The `AS` must itself be the keyword: in `SELECT 1 AS as FROM t` the
/// second `as` is the label, and the `FROM` after it is a keyword again.
pub(super) fn is_name(tokens: &[Token], index: usize) -> bool {
    index > 0
        && (punct(tokens, index - 1, '.')
            || is(tokens, index - 1, "as") && !is_name(tokens, index - 1))
}

/// The longest identifier PostgreSQL keeps whole; a longer one is truncated
/// to this many bytes, so two spellings could name one relation.
pub(super) const IDENTIFIER_BYTES: usize = 63;

/// Whether the `ON` at `on` opens an INSERT's `ON CONFLICT` clause, which
/// continues with a conflict target or `DO`. A join condition may begin
/// with a row named conflict instead (`ON conflict.id = a.id`); an
/// unqualified call `conflict(...)` fails the detector.
pub(super) fn opens_on_conflict(tokens: &[Token], on: usize) -> bool {
    is(tokens, on, "on")
        && is(tokens, on + 1, "conflict")
        && (punct(tokens, on + 2, '(')
            || is(tokens, on + 2, "do")
            || is(tokens, on + 2, "on") && is(tokens, on + 3, "constraint"))
}

/// Whether the parenthesis at `open` opens a CTE body: `AS (` or
/// `AS [NOT] MATERIALIZED (`.
pub(super) fn opens_cte_body(tokens: &[Token], open: usize) -> bool {
    punct(tokens, open, '(')
        && (is(tokens, open.wrapping_sub(1), "as")
            || is(tokens, open.wrapping_sub(1), "materialized"))
}

/// Whether the command word at `index` begins a command: the statement's
/// own, a data-modifying CTE body's (`AS (DELETE ...`), or the main one
/// after the CTE list (`WITH c AS (...) UPDATE ...`). Anywhere else the
/// word is a name, such as a column label (`SELECT 1 AS delete FROM t`).
pub(super) fn opens_command(tokens: &[Token], index: usize) -> bool {
    index == 0
        || opens_cte_body(tokens, index - 1)
        || punct(tokens, index - 1, ')')
            && opening(tokens, index - 1).is_some_and(|open| opens_cte_body(tokens, open))
}

/// Whether the `WITH` at `index` is part of a type or a FROM item rather
/// than a CTE list: `... WITH TIME ZONE`, or `f(...) WITH ORDINALITY` that is
/// not a CTE named `ordinality` (`WITH ordinality AS (` or `(columns)`).
pub(super) fn is_with_modifier(tokens: &[Token], index: usize) -> bool {
    let time_zone = is(tokens, index + 1, "time") && is(tokens, index + 2, "zone");
    let ordinality = punct(tokens, index.wrapping_sub(1), ')')
        && is(tokens, index + 1, "ordinality")
        && !punct(tokens, index + 2, '(')
        && !(is(tokens, index + 2, "as") && punct(tokens, index + 3, '('));
    time_zone || ordinality
}

/// The forms the declaration scan does not read, found token by token: an
/// identifier PostgreSQL would truncate, and keywords that open a clause
/// the guard does not follow. A field or label is a name, never a keyword.
pub(super) fn unread_forms(tokens: &[Token]) -> Vec<String> {
    let mut found = Vec::new();
    for index in 0..tokens.len() {
        if word(tokens, index).is_some_and(|name| name.len() > IDENTIFIER_BYTES) {
            found.push(format!(
                "an identifier longer than {IDENTIFIER_BYTES} bytes, which PostgreSQL truncates"
            ));
        }
        if is_name(tokens, index) {
            continue;
        }
        // A CTE is visible only in its own query, so a WITH that is not the
        // statement's own could shadow a relation elsewhere; RETURNING WITH
        // names OLD and NEW rows this scan does not read.
        if is(tokens, index, "with") && index > 0 && !is_with_modifier(tokens, index) {
            found.push("a WITH clause that is not the statement's own".to_owned());
        }
        if is(tokens, index, "returning") && is(tokens, index + 1, "with") {
            found.push("a RETURNING WITH clause".to_owned());
        }
        // TABLE is reserved, and no accepted command creates or locks a
        // table, so the word opens a `TABLE name` query, which reads a
        // relation outside any FROM list.
        if is(tokens, index, "table") {
            found.push("a TABLE query".to_owned());
        }
        // A recursive CTE's SEARCH or CYCLE clause follows its body. The
        // scan reads neither, and the command after one would go
        // unrecognized, so neither is part of the dialect.
        if (is(tokens, index, "search") || is(tokens, index, "cycle"))
            && index > 0
            && punct(tokens, index - 1, ')')
            && opening(tokens, index - 1).is_some_and(|open| opens_cte_body(tokens, open))
        {
            found.push("a CTE SEARCH or CYCLE clause".to_owned());
        }
        // `SELECT ... INTO` creates a table; only an INSERT that begins a
        // command writes one. INTO is reserved, so the word is the keyword.
        if is(tokens, index, "into")
            && !(index > 0 && is(tokens, index - 1, "insert") && opens_command(tokens, index - 1))
        {
            found.push("a SELECT INTO, which creates a table".to_owned());
        }
        // `JOIN ... USING (columns) AS alias` names a join row the
        // declaration scan does not read.
        if is(tokens, index, "using")
            && punct(tokens, index + 1, '(')
            && using_columns(tokens, index + 1)
            && is(tokens, closing(tokens, index + 1) + 1, "as")
        {
            found.push("a JOIN USING alias".to_owned());
        }
    }
    found
}
