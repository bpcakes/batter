//! The SQL side of the catalog guard: finds unqualified catalog functions and
//! casts in one statement's text. It strips comments and quoted text and
//! reads SQL structurally; it is not a full parser.

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

/// Words that open a common table expression's query body.
const QUERY_KEYWORDS: &[&str] = &[
    "delete", "insert", "select", "table", "update", "values", "with",
];

/// Words before an identifier that make its parenthesis a column list.
const COLUMN_LIST_OWNERS: &[&str] = &["as", "into", "recursive", "with"];

/// The unqualified catalog functions and types `sql` uses, and any qualified
/// multi-array `unnest`, which PostgreSQL recognizes only unqualified: the
/// qualified spelling is `ROWS FROM (pg_catalog.unnest(a), ...)`.
pub(super) fn unqualified(sql: &str) -> Vec<String> {
    let mut found = outside_dialect(sql);
    let code = strip_comments_and_quotes(sql);
    let bytes = code.as_bytes();
    for (start, _) in code.to_ascii_lowercase().match_indices("pg_catalog.unnest") {
        let open = skip_spaces(bytes, start + "pg_catalog.unnest".len());
        if bytes.get(open) == Some(&b'(') && top_level_arguments(&code[open..]) > 1 {
            found.push("multi-array pg_catalog.unnest(...)".to_owned());
        }
    }
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
        if bytes.get(after) == Some(&b'(') && !cast_type {
            match qualifier.as_deref() {
                Some("pg_catalog") => {}
                Some(schema) => found.push(format!("{schema}.{word}(...)")),
                None if SYNTAX.contains(&word.as_str())
                    || owns_column_list(&code, start, after) => {}
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

/// The lowercase schema or relation that qualifies the identifier at `start`
/// (`schema.name`), if any.
fn qualifier_before(code: &str, start: usize) -> Option<String> {
    let bytes = code.as_bytes();
    if start == 0 || bytes[start - 1] != b'.' {
        return None;
    }
    let end = start - 1;
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
    while index < bytes.len() {
        match bytes[index] {
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                while index < bytes.len() && bytes[index] != b'\n' {
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
                let before = sql[..index].trim_end();
                if !(before.ends_with('(') || before.ends_with(',')) {
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
            _ => index += 1,
        }
    }
    found
}

/// Whether the parenthesis after `start..after` is a relation's column list:
/// an alias, insert target or common table expression name.
fn owns_column_list(code: &str, start: usize, paren: usize) -> bool {
    let before = code[..start].trim_end();
    let previous = before
        .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if COLUMN_LIST_OWNERS.contains(&previous.as_str()) {
        return true;
    }
    let bytes = code.as_bytes();
    let mut depth = 0_usize;
    for (offset, byte) in bytes[paren..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return is_cte_body(&code[paren + offset + 1..]);
                }
            }
            _ => {}
        }
    }
    false
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

/// Whether `rest` starts a common table expression body,
/// `AS [[NOT] MATERIALIZED] (query`, rather than a record-returning call's
/// column definition list, `AS (name type, ...)`.
fn is_cte_body(rest: &str) -> bool {
    let mut rest = rest.trim_start();
    for word in ["as", "not", "materialized"] {
        match rest.get(..word.len()) {
            Some(prefix)
                if prefix.eq_ignore_ascii_case(word)
                    && !rest[word.len()..]
                        .bytes()
                        .next()
                        .is_some_and(is_identifier_part) =>
            {
                rest = rest[word.len()..].trim_start();
            }
            _ if word == "as" => return false,
            _ => {}
        }
    }
    let Some(body) = rest.strip_prefix('(') else {
        return false;
    };
    let keyword: String = body
        .trim_start()
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    QUERY_KEYWORDS.contains(&keyword.to_ascii_lowercase().as_str())
}

/// `sql` with comments and string constants blanked. A double-quoted
/// identifier becomes `quoted_identifier`, unless it is exactly
/// `"pg_catalog"`, so `"clock_timestamp"()` still reads as an unqualified
/// call and `"PG_CATALOG".floor()` and `::"char"` are not mistaken for the
/// catalog or a keyword type.
fn strip_comments_and_quotes(sql: &str) -> String {
    let mut code = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '-' if chars.peek() == Some(&'-') => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        code.push('\n');
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
