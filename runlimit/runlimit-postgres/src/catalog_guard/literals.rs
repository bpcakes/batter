//! The Rust side of the catalog guard: finds every string literal in a source
//! file that reads as a SQL statement, with the constant it initializes.

use std::{fs, path::Path};

use super::detector::is_identifier_part;

pub(super) fn rust_files(directory: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// Every string literal in Rust `source` whose text starts like a statement,
/// with the constant it initializes, if any.
pub(super) fn sql_literals(source: &str) -> Vec<(Option<String>, String)> {
    let mut literals = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index += 2;
            }
            b'\'' => index = skip_char_literal(source, index),
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
                    &source[index + 1..end.min(bytes.len())],
                    &mut literals,
                );
                index = end + 1;
            }
            _ => index += 1,
        }
    }
    literals
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

fn record(source: &str, start: usize, literal: &str, literals: &mut Vec<(Option<String>, String)>) {
    let statement = literal.trim_start();
    let keyword = statement
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if !matches!(
        keyword.as_str(),
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "WITH" | "SET" | "LOCK" | "CALL" | "DO"
    ) {
        return;
    }
    let before = source[..start].trim_end();
    let owner = before
        .strip_suffix('=')
        .map(str::trim_end)
        .and_then(|declaration| {
            declaration
                .strip_suffix("&str")
                .or_else(|| declaration.strip_suffix("&'static str"))
        })
        .map(str::trim_end)
        .and_then(|declaration| declaration.strip_suffix(':'))
        .and_then(|declaration| {
            let name = declaration
                .trim_end()
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()?;
            declaration
                .trim_end()
                .strip_suffix(name)?
                .trim_end()
                .ends_with("const")
                .then(|| name.to_owned())
        });
    literals.push((owner, statement.to_owned()));
}
