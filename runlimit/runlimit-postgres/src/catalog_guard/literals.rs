//! The Rust side of the catalog guard: finds every string literal in a source
//! file that reads as a SQL statement, with the constant it initializes, and
//! the argument of every `sqlx` query constructor.

use std::{fs, path::Path};

use super::detector::is_identifier_part;

/// The `sqlx` functions that turn a string into an executable statement.
const QUERY_CONSTRUCTORS: &[&str] = &[
    "query",
    "query_as",
    "query_scalar",
    "query_with",
    "query_as_with",
    "query_scalar_with",
    "raw_sql",
];

/// Every file under `directory` with `extension`, recursively.
pub(super) fn files(directory: &Path, extension: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files(&path, extension));
        } else if path.extension().is_some_and(|actual| actual == extension) {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// The argument text of every `sqlx` query constructor call in `code`, which
/// must have its literals and comments blanked by [`sql_literals`].
pub(super) fn query_arguments(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let mut arguments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() || index > 0 && is_identifier_part(bytes[index - 1])
        {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_identifier_part(bytes[index]) {
            index += 1;
        }
        if !QUERY_CONSTRUCTORS.contains(&&code[start..index]) {
            continue;
        }
        let mut open = index;
        if code[open..].starts_with("::<") {
            let mut depth = 0_usize;
            for (offset, byte) in bytes[open..].iter().enumerate() {
                match byte {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            open += offset + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let open = open + code[open..].len() - code[open..].trim_start().len();
        if bytes.get(open) != Some(&b'(') {
            continue;
        }
        let mut depth = 0_usize;
        let mut end = open + 1;
        while end < bytes.len() {
            match (bytes[end], depth) {
                (b'(' | b'[' | b'{', _) => depth += 1,
                (b')' | b']' | b'}' | b',', 0) => break,
                (b')' | b']' | b'}', _) => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        arguments.push(code[open + 1..end].trim().to_owned());
        index = end;
    }
    arguments
}

/// Every string literal in Rust `source` whose text starts like a statement,
/// with the constant it initializes, if any; and `source` with every literal
/// and comment blanked.
pub(super) fn sql_literals(source: &str) -> (Vec<(Option<String>, String)>, String) {
    let mut literals = Vec::new();
    let bytes = source.as_bytes();
    let mut code = bytes.to_vec();
    let mut blank = |from: usize, to: usize| {
        for byte in &mut code[from..to.min(bytes.len())] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                blank(start, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index += 2;
                blank(start, index);
            }
            b'\'' => {
                let start = index;
                index = skip_char_literal(source, index);
                blank(start, index);
            }
            b'r' if matches!(bytes.get(index + 1), Some(b'"' | b'#'))
                && (index == 0 || !is_identifier_part(bytes[index - 1])) =>
            {
                let hashes = bytes[index + 1..]
                    .iter()
                    .take_while(|b| **b == b'#')
                    .count();
                let open = index + 1 + hashes;
                if bytes.get(open) != Some(&b'"') {
                    index += 1;
                    continue;
                }
                let terminator = format!("\"{}", "#".repeat(hashes));
                let end = source[open + 1..]
                    .find(&terminator)
                    .map_or(bytes.len(), |offset| open + 1 + offset);
                record(source, index, &source[open + 1..end], &mut literals);
                blank(index, end + terminator.len());
                index = end + terminator.len();
            }
            b'"' => {
                let mut end = index + 1;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                record(
                    source,
                    index,
                    &unescape(&source[index + 1..end.min(bytes.len())]),
                    &mut literals,
                );
                blank(index, end + 1);
                index = end + 1;
            }
            _ => index += 1,
        }
    }
    (literals, String::from_utf8(code).unwrap())
}

fn skip_char_literal(source: &str, index: usize) -> usize {
    // `'x'` or an escape such as `'\n'`; anything else opens a lifetime.
    let rest = &source[index + 1..];
    if let Some(escaped) = rest.strip_prefix('\\') {
        return escaped
            .find('\'')
            .map_or(source.len(), |end| index + end + 3);
    }
    match rest.chars().next() {
        Some(character) if rest[character.len_utf8()..].starts_with('\'') => {
            index + 2 + character.len_utf8()
        }
        _ => index + 1,
    }
}

/// Statement keywords; a literal that starts with one, after comments and
/// opening parentheses, is treated as SQL.
const STATEMENT_KEYWORDS: &[&str] = &[
    "ALTER",
    "ANALYZE",
    "BEGIN",
    "CALL",
    "COMMIT",
    "COPY",
    "CREATE",
    "DECLARE",
    "DELETE",
    "DISCARD",
    "DO",
    "DROP",
    "EXECUTE",
    "EXPLAIN",
    "FETCH",
    "GRANT",
    "INSERT",
    "LISTEN",
    "LOCK",
    "MERGE",
    "NOTIFY",
    "PREPARE",
    "REFRESH",
    "RELEASE",
    "RESET",
    "REVOKE",
    "ROLLBACK",
    "SAVEPOINT",
    "SELECT",
    "SET",
    "SHOW",
    "TABLE",
    "TRUNCATE",
    "UPDATE",
    "VACUUM",
    "VALUES",
    "WITH",
];

/// `text` without leading whitespace, SQL comments and opening parentheses.
fn statement_start(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim_start().trim_start_matches('(');
        text = if let Some(rest) = trimmed.strip_prefix("--") {
            rest.split_once('\n').map_or("", |(_, rest)| rest)
        } else if let Some(rest) = trimmed.strip_prefix("/*") {
            rest.split_once("*/").map_or("", |(_, rest)| rest)
        } else {
            return trimmed;
        };
    }
}

fn record(source: &str, start: usize, value: &str, literals: &mut Vec<(Option<String>, String)>) {
    let keyword = statement_start(value)
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if STATEMENT_KEYWORDS.contains(&keyword.as_str()) {
        literals.push((owner_before(source, start), value.to_owned()));
    }
}

/// The constant whose initializer starts at `start`: `const NAME: &str =`.
fn owner_before(source: &str, start: usize) -> Option<String> {
    let declaration = source[..start].trim_end().strip_suffix('=')?.trim_end();
    let declaration = declaration
        .strip_suffix("&str")
        .or_else(|| declaration.strip_suffix("&'static str"))
        .or_else(|| declaration.strip_suffix("&[u8]"))?
        .trim_end()
        .strip_suffix(':')?
        .trim_end();
    let name = declaration
        .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()?;
    declaration
        .strip_suffix(name)?
        .trim_end()
        .ends_with("const")
        .then(|| name.to_owned())
}

/// Every `include_str!`/`include_bytes!` in `source` (with `code` its blanked
/// copy): the constant it initializes and the literal path it names.
pub(super) fn includes(source: &str, code: &str) -> Vec<(Option<String>, String)> {
    let mut found = Vec::new();
    for macro_name in ["include_str!", "include_bytes!"] {
        for (start, _) in code.match_indices(macro_name) {
            let open = start + macro_name.len();
            let close = code[open..]
                .find(')')
                .map_or(code.len(), |offset| open + offset);
            let literal = source[open..close]
                .trim()
                .trim_start_matches(['(', '[', '{'])
                .trim()
                .trim_start_matches('r')
                .trim_matches('#')
                .trim_matches('"')
                .to_owned();
            found.push((owner_before(source, start), literal));
        }
    }
    found
}

/// `(path relative to src, production text)` for `lib.rs` and every module it
/// declares, recursively, except a module declared under `#[cfg(test)]`. An
/// inline `#[cfg(test)] mod name { .. }` is blanked from its file's text.
pub(super) fn production_sources(source: &Path) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut pending = vec!["lib.rs".to_owned()];
    while let Some(file) = pending.pop() {
        let text = fs::read_to_string(source.join(&file)).unwrap();
        let (_, code) = sql_literals(&text);
        let mut production = text.clone().into_bytes();
        for declaration in module_declarations(&text, &code) {
            match declaration {
                Module::OutOfLine { name, path, test } if !test => {
                    pending.push(child_file(source, &file, &name, path.as_deref()));
                }
                Module::InlineTest { body } => {
                    for byte in &mut production[body] {
                        if *byte != b'\n' {
                            *byte = b' ';
                        }
                    }
                }
                Module::OutOfLine { .. } => {}
            }
        }
        sources.push((file, String::from_utf8(production).unwrap()));
    }
    sources.sort();
    sources
}

enum Module {
    OutOfLine {
        name: String,
        path: Option<String>,
        test: bool,
    },
    InlineTest {
        body: std::ops::Range<usize>,
    },
}

/// Every `mod name;` and inline `#[cfg(test)] mod name { .. }` in `text`.
fn module_declarations(text: &str, code: &str) -> Vec<Module> {
    let bytes = code.as_bytes();
    let mut modules = Vec::new();
    for (start, _) in code.match_indices("mod ") {
        if start > 0 && is_identifier_part(bytes[start - 1]) {
            continue;
        }
        let name: String = code[start + 4..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let after_name = start + 4 + code[start + 4..].find(&name).unwrap_or(0) + name.len();
        let attributes = attributes_before(code, start);
        let test_only = attributes
            .iter()
            .any(|range| code[range.clone()].contains("cfg(test)"));
        let path = attributes.iter().find_map(|range| {
            let attribute = &text[range.clone()];
            let value = attribute.split_once("path")?.1.split_once('"')?.1;
            Some(value.split_once('"')?.0.to_owned())
        });
        match code[after_name..].trim_start().bytes().next() {
            Some(b';') => modules.push(Module::OutOfLine {
                name,
                path,
                test: test_only,
            }),
            Some(b'{') if test_only => {
                let open = after_name + code[after_name..].find('{').unwrap_or(0);
                let mut depth = 0_usize;
                let mut close = open;
                for (offset, byte) in bytes[open..].iter().enumerate() {
                    match byte {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = open + offset + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                modules.push(Module::InlineTest { body: start..close });
            }
            _ => {}
        }
    }
    modules
}

/// The `#[...]` attributes, and any visibility, directly before `start`.
fn attributes_before(code: &str, start: usize) -> Vec<std::ops::Range<usize>> {
    let mut end = code[..start].trim_end().len();
    let before = &code[..end];
    if let Some(stripped) = before.strip_suffix(')').filter(|_| before.contains("pub(")) {
        if let Some(open) = stripped.rfind("pub(") {
            end = code[..open].trim_end().len();
        }
    } else if before.ends_with("pub") {
        end = code[..end - 3].trim_end().len();
    }
    let mut attributes = Vec::new();
    while code[..end].ends_with(']') {
        let Some(open) = code[..end].rfind("#[") else {
            break;
        };
        attributes.push(open..end);
        end = code[..open].trim_end().len();
    }
    attributes
}

/// The file of module `name` declared in `file`, relative to `source`.
fn child_file(source: &Path, file: &str, name: &str, path: Option<&str>) -> String {
    let parent = Path::new(file).parent().unwrap_or_else(|| Path::new(""));
    if let Some(path) = path {
        return parent.join(path).to_str().unwrap().replace('\\', "/");
    }
    let directory = if file == "lib.rs" || file.ends_with("/mod.rs") {
        parent.to_path_buf()
    } else {
        Path::new(file).with_extension("")
    };
    let flat = directory.join(format!("{name}.rs"));
    let nested = directory.join(name).join("mod.rs");
    let chosen = if source.join(&flat).exists() {
        flat
    } else {
        nested
    };
    chosen.to_str().unwrap().replace('\\', "/")
}

/// The value of a non-raw string literal's body for the escapes SQL text uses:
/// line continuations, `\n`, `\t`, `\r`, `\0`, `\\`, `\"` and `\'`. Any other
/// escape is kept as written.
fn unescape(body: &str) -> String {
    let mut value = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            value.push(character);
            continue;
        }
        match chars.next() {
            Some('\n') => {
                while chars.peek().is_some_and(|next| next.is_whitespace()) {
                    chars.next();
                }
            }
            Some('n') => value.push('\n'),
            Some('t') => value.push('\t'),
            Some('r') => value.push('\r'),
            Some('0') => value.push('\0'),
            Some(escaped @ ('\\' | '"' | '\'')) => value.push(escaped),
            Some(other) => {
                value.push('\\');
                value.push(other);
            }
            None => value.push('\\'),
        }
    }
    value
}
