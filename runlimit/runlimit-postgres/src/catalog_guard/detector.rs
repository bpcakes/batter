//! The SQL side of the catalog guard: finds unqualified catalog functions and
//! casts in one statement's text. It strips comments and quoted text and
//! reads SQL structurally; it is not a full parser.

use std::collections::BTreeSet;

/// Call-shaped words that are SQL syntax, not catalog functions.
const SYNTAX: &[&str] = &[
    "all",
    "and",
    "any",
    "array",
    "as",
    "coalesce",
    "conflict",
    "distinct",
    "else",
    "exists",
    "extract",
    "filter",
    "from",
    "greatest",
    "in",
    "lateral",
    "least",
    "materialized",
    "not",
    "on",
    "or",
    "over",
    "recursive",
    "row",
    "select",
    "set",
    "then",
    "using",
    "values",
    "when",
    "where",
    "with",
];

/// Type names the grammar maps to `pg_catalog` types.
const KEYWORD_TYPES: &[&str] = &[
    "bigint",
    "bit",
    "boolean",
    "char",
    "character",
    "dec",
    "decimal",
    "float",
    "int",
    "integer",
    "interval",
    "numeric",
    "real",
    "smallint",
    "time",
    "timestamp",
    "varchar",
];

/// Words before an identifier that make its parenthesis a column list.
const COLUMN_LIST_OWNERS: &[&str] = &["as", "into"];

/// The unqualified catalog functions and types `sql` uses, and any qualified
/// multi-array `unnest`, which PostgreSQL recognizes only unqualified: the
/// qualified spelling is `ROWS FROM (pg_catalog.unnest(a), ...)`.
pub(super) fn unqualified(sql: &str) -> Vec<String> {
    let mut found = outside_dialect(sql);
    let code = strip_comments_and_quotes(sql);
    let ctes = cte_names(&code);
    let bytes = code.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !is_identifier_start(bytes[index]) || index > 0 && is_identifier_part(bytes[index - 1]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_identifier_part(bytes[index]) {
            index += 1;
        }
        let word = code[start..index].to_ascii_lowercase();
        let qualifier = qualifier_before(&code, start);
        let after = skip_spaces(bytes, index);
        let qualified_next = bytes.get(after) == Some(&b'.');
        let back = skip_spaces_back(bytes, start);
        let cast_type = back >= 2 && &code[back - 2..back] == "::";
        // `::numeric(39, 0)` is a type modifier, not a call.
        if bytes.get(after) == Some(&b'(')
            && word == "unnest"
            && qualifier.as_deref() == Some("pg_catalog")
            && top_level_arguments(&code[after..]) > 1
        {
            found.push("multi-array pg_catalog.unnest(...)".to_owned());
        }
        if bytes.get(after) == Some(&b'(') && !cast_type {
            match qualifier.as_deref() {
                Some("pg_catalog") => {}
                Some(schema) => found.push(format!("{schema}.{word}(...)")),
                None if is_syntax(&code, start, &word) || owns_column_list(&code, start, &ctes) => {
                }
                None => found.push(format!("{word}(...)")),
            }
        }
        if qualifier.is_none() && cast_type {
            if qualified_next {
                if word != "pg_catalog" {
                    found.push(format!("::{word}.(...)"));
                }
            } else if word == "double" {
                // Only the two-word spelling is grammar; `double` alone
                // resolves through `search_path`.
                if !next_word_is(&code, after, "precision") {
                    found.push("::double without precision".to_owned());
                }
            } else if !KEYWORD_TYPES.contains(&word.as_str()) {
                found.push(format!("::{word}"));
            }
        }
    }
    found
}

/// Whether the identifier at `start` in `code` is `word`, ignoring case.
fn next_word_is(code: &str, start: usize, word: &str) -> bool {
    let end = start + word.len();
    code.get(start..end)
        .is_some_and(|found| found.eq_ignore_ascii_case(word))
        && !code
            .as_bytes()
            .get(end)
            .copied()
            .is_some_and(is_identifier_part)
}

/// Unreserved keywords in `SYNTAX`: PostgreSQL also accepts each as a
/// function name, so `word(` is grammar only in the clause it spells.
const CONTEXTUAL_SYNTAX: &[&str] = &[
    "conflict",
    "filter",
    "materialized",
    "over",
    "recursive",
    "set",
];

/// Whether `word(` at `start` is grammar rather than a function call: a
/// reserved or column-name keyword anywhere, or an unreserved one only in
/// its own clause.
fn is_syntax(code: &str, start: usize, word: &str) -> bool {
    if !SYNTAX.contains(&word) {
        return false;
    }
    if !CONTEXTUAL_SYNTAX.contains(&word) {
        return true;
    }
    let bytes = code.as_bytes();
    let open = skip_spaces(bytes, start + word.len());
    let close = closing_paren(bytes, open);
    let inside = token_at(code, open + 1);
    let after = token_at(code, close + 1);
    let previous = previous_tokens(code, start, 4);
    let before = |index: usize, token: &str| previous.get(index).is_some_and(|p| p == token);
    let opens_query =
        ["select", "with", "values", "insert", "update", "delete"].contains(&inside.as_str());
    match word {
        // `name [(columns)] AS [NOT] MATERIALIZED (query)`.
        "materialized" => (before(0, "as") || before(0, "not") && before(1, "as")) && opens_query,
        // `ON CONFLICT (target) DO ...`; a conflict target's WHERE is outside
        // this dialect.
        "conflict" => before(0, "on") && after == "do",
        // `aggregate(...) FILTER (WHERE ...)`.
        "filter" => before(0, ")") && inside == "where",
        // `function(...) OVER (window specification)`.
        "over" => {
            before(0, ")")
                && ["partition", "order", "rows", "range", "groups", ")"].contains(&inside.as_str())
        }
        // `UPDATE table [AS alias] SET (columns) = ...`, or `DO UPDATE SET`.
        "set" => {
            (before(0, "update") || before(1, "update") || before(1, "as") && before(3, "update"))
                && after == "="
        }
        _ => false,
    }
}

/// The lowercase identifier, or the single character, at the first
/// non-space byte from `index`.
fn token_at(code: &str, index: usize) -> String {
    let bytes = code.as_bytes();
    let start = skip_spaces(bytes, index);
    let mut end = start;
    while end < bytes.len() && is_identifier_part(bytes[end]) {
        end += 1;
    }
    if end == start {
        end = (start + 1).min(bytes.len());
    }
    code.get(start..end)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Up to `count` lowercase tokens before `start`, nearest first: each an
/// identifier or a single punctuation character.
fn previous_tokens(code: &str, start: usize, count: usize) -> Vec<String> {
    let bytes = code.as_bytes();
    let mut tokens = Vec::new();
    let mut index = start;
    while tokens.len() < count {
        index = skip_spaces_back(bytes, index);
        if index == 0 {
            break;
        }
        let end = index;
        while index > 0 && is_identifier_part(bytes[index - 1]) {
            index -= 1;
        }
        if index == end {
            index -= 1;
        }
        tokens.push(code[index..end].to_ascii_lowercase());
    }
    tokens
}

/// The lowercase schema or relation that qualifies the identifier at `start`
/// (`schema.name`), if any.
fn qualifier_before(code: &str, start: usize) -> Option<String> {
    let bytes = code.as_bytes();
    // Whitespace, and the comments `code` blanks, may stand on either side
    // of the dot.
    let dot = skip_spaces_back(bytes, start);
    if dot == 0 || bytes[dot - 1] != b'.' {
        return None;
    }
    let end = skip_spaces_back(bytes, dot - 1);
    let mut begin = end;
    while begin > 0 && is_identifier_part(bytes[begin - 1]) {
        begin -= 1;
    }
    Some(code[begin..end].to_ascii_lowercase())
}

/// The constructs operational SQL leaves out so that this detector reads it
/// completely: dollar quoting, quoted identifiers, `CAST(...)` (casts use
/// `::`) and a string constant anywhere but directly after `(` or `,`, which
/// excludes typed literals such as `text 'value'`.
fn outside_dialect(sql: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = sql.as_bytes();
    let mut index = 0;
    // The last byte outside comments and whitespace, which decides where a
    // string constant stands.
    let mut previous = None;
    while index < bytes.len() {
        let token = index;
        match bytes[index] {
            // PostgreSQL ends a line comment at either line terminator.
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    if bytes[index] == b'/' && bytes[index + 1] == b'*' {
                        found.push("a nested block comment".to_owned());
                    }
                    index += 1;
                }
                index += 2;
            }
            b'\'' => {
                if !matches!(previous, Some(b'(' | b',')) {
                    found.push("a string constant outside a function argument".to_owned());
                }
                index += 1;
                // A doubled quote is an escaped quote inside the same constant.
                while index < bytes.len()
                    && (bytes[index] != b'\'' || bytes.get(index + 1) == Some(&b'\''))
                {
                    index += if bytes[index] == b'\'' { 2 } else { 1 };
                }
                index += 1;
            }
            b'"' => {
                found.push("a quoted identifier".to_owned());
                index += 1;
            }
            b'$' if bytes
                .get(index + 1)
                .is_some_and(|next| *next == b'$' || is_identifier_start(*next)) =>
            {
                found.push("dollar quoting".to_owned());
                index += 1;
            }
            byte if is_identifier_start(byte)
                && (index == 0 || !is_identifier_part(bytes[index - 1])) =>
            {
                let start = index;
                while index < bytes.len() && is_identifier_part(bytes[index]) {
                    index += 1;
                }
                if sql[start..index].eq_ignore_ascii_case("cast")
                    && bytes.get(skip_spaces(bytes, index)) == Some(&b'(')
                {
                    found.push("CAST(...) instead of ::".to_owned());
                }
            }
            // PostgreSQL accepts non-ASCII letters in unquoted identifiers,
            // which this reader does not tokenize; count each character once.
            byte if byte >= 0x80 => {
                if byte & 0xC0 != 0x80 {
                    found.push("a non-ASCII character outside a comment or string".to_owned());
                }
                index += 1;
            }
            _ => index += 1,
        }
        // Comments and whitespace leave `previous` where it was.
        let comment = bytes[token] == b'-' && bytes.get(token + 1) == Some(&b'-')
            || bytes[token] == b'/' && bytes.get(token + 1) == Some(&b'*');
        if !comment && !bytes[token].is_ascii_whitespace() {
            previous = Some(bytes[index.min(bytes.len()) - 1]);
        }
    }
    found
}

/// Whether the parenthesis after the identifier at `start` is a column list
/// rather than a call: an alias's or insert target's (`AS name(...)`,
/// `INTO name(...)`) or a common table expression's.
fn owns_column_list(code: &str, start: usize, ctes: &BTreeSet<usize>) -> bool {
    let before = code[..start].trim_end();
    let previous = before
        .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    COLUMN_LIST_OWNERS.contains(&previous.as_str()) || ctes.contains(&start)
}

/// Where each common table expression's name starts: the first after `WITH`
/// or `WITH RECURSIVE`, and each after a `,` that follows a body. Only
/// `name [(columns)] AS [NOT] [MATERIALIZED] (` declares one, so
/// `WITH ORDINALITY AS input(...)` and a record's column definition list
/// such as `json_to_record($1) AS (values integer)` are not.
fn cte_names(code: &str) -> BTreeSet<usize> {
    let bytes = code.as_bytes();
    let mut names = BTreeSet::new();
    let mut search = 0;
    while let Some(found) = find_word(code, search, "with") {
        search = found + "with".len();
        let mut index = skip_spaces(bytes, search);
        if word_at(code, index, "recursive") {
            index = skip_spaces(bytes, index + "recursive".len());
        }
        loop {
            let name = index;
            let mut next = identifier_end(bytes, index);
            if next == name {
                break;
            }
            next = skip_spaces(bytes, next);
            if bytes.get(next) == Some(&b'(') {
                next = skip_spaces(bytes, closing_paren(bytes, next) + 1);
            }
            if !word_at(code, next, "as") {
                break;
            }
            next = skip_spaces(bytes, next + "as".len());
            for modifier in ["not", "materialized"] {
                if word_at(code, next, modifier) {
                    next = skip_spaces(bytes, next + modifier.len());
                }
            }
            if bytes.get(next) != Some(&b'(') {
                break;
            }
            names.insert(name);
            let after = skip_spaces(bytes, closing_paren(bytes, next) + 1);
            if bytes.get(after) != Some(&b',') {
                break;
            }
            index = skip_spaces(bytes, after + 1);
        }
    }
    names
}

/// The next whole-word `word` at or after `from`, ignoring case.
fn find_word(code: &str, from: usize, word: &str) -> Option<usize> {
    (from..code.len()).find(|&index| word_at(code, index, word))
}

/// Whether the whole word at `index` is `word`, ignoring case.
fn word_at(code: &str, index: usize, word: &str) -> bool {
    let bytes = code.as_bytes();
    (index == 0 || !is_identifier_part(bytes[index - 1])) && next_word_is(code, index, word)
}

/// The end of the identifier starting at `index`, or `index` if none does.
fn identifier_end(bytes: &[u8], index: usize) -> usize {
    if !bytes.get(index).copied().is_some_and(is_identifier_start) {
        return index;
    }
    let mut end = index;
    while bytes.get(end).copied().is_some_and(is_identifier_part) {
        end += 1;
    }
    end
}

/// The index of the parenthesis closing the one at `open`, or the end.
fn closing_paren(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0_usize;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

/// The number of top-level arguments in the parenthesized list `call` opens.
fn top_level_arguments(call: &str) -> usize {
    let mut depth = 0_usize;
    let mut arguments = 1;
    for byte in call.bytes() {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            b',' if depth == 1 => arguments += 1,
            _ => {}
        }
    }
    arguments
}

/// `sql` with comments and string constants blanked. A double-quoted
/// identifier becomes `quoted_identifier`, unless it is exactly
/// `"pg_catalog"`, so `"clock_timestamp"()` still reads as an unqualified
/// call and `"PG_CATALOG".floor()` and `::"char"` are not mistaken for the
/// catalog or a keyword type.
pub(super) fn strip_comments_and_quotes(sql: &str) -> String {
    let mut code = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '-' if chars.peek() == Some(&'-') => {
                for skipped in chars.by_ref() {
                    if matches!(skipped, '\n' | '\r') {
                        code.push(skipped);
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                for skipped in chars.by_ref() {
                    if previous == '*' && skipped == '/' {
                        break;
                    }
                    previous = skipped;
                }
                code.push(' ');
            }
            '\'' => {
                // A doubled quote is an escaped quote inside the same constant.
                while let Some(skipped) = chars.next() {
                    if skipped == '\'' && chars.next_if_eq(&'\'').is_none() {
                        break;
                    }
                }
                code.push(' ');
            }
            '"' => {
                // A quoted name keeps its exact case and is never a keyword,
                // so only `"pg_catalog"` itself still reads as the catalog.
                let identifier: String = chars.by_ref().take_while(|c| *c != '"').collect();
                code.push_str(if identifier == "pg_catalog" {
                    "pg_catalog"
                } else {
                    "quoted_identifier"
                });
            }
            other => code.push(other),
        }
    }
    code
}

const fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_identifier_part(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn skip_spaces(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    index
}

fn skip_spaces_back(bytes: &[u8], mut index: usize) -> usize {
    while index > 0 && bytes[index - 1].is_ascii_whitespace() {
        index -= 1;
    }
    index
}
