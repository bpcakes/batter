//! Every path through which a source reaches `std::env`, found in code with
//! comments and literals removed, so no spelling of an import or a call is
//! missed: a leading `::`, an alias, `self`, a glob, a group wrapped over
//! several lines and a macro fragment such as `std::env::$function` all
//! count.
//!
//! The reader does not resolve names. It counts every `use` path and every
//! expression path that starts with `std::env`, or that globs all of `std`,
//! unless the segment after `env` is one of the items in [`ALLOWED`], which
//! name types or read the process's arguments and executable, never its
//! environment. It cannot see a read through `extern crate std as other;`, a
//! path that a procedural macro generates or code that `include!` inserts.

/// Items of `std::env` that read nothing of the environment.
pub(crate) const ALLOWED: [&str; 8] = [
    "VarError",
    "JoinPathsError",
    "Args",
    "ArgsOs",
    "args",
    "args_os",
    "current_exe",
    "consts",
];

/// `source` with every comment, string and character literal replaced by
/// spaces, so their text can name nothing. Lifetimes are kept.
pub(crate) fn code_of(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    let blank = |out: &mut String, text: &[char]| {
        out.extend(text.iter().map(|c| if *c == '\n' { '\n' } else { ' ' }));
    };
    while index < chars.len() {
        let rest = &chars[index..];
        let end = if rest.starts_with(&['/', '/']) {
            rest.iter().position(|c| *c == '\n').unwrap_or(rest.len())
        } else if rest.starts_with(&['/', '*']) {
            block_comment(rest)
        } else if let Some(length) = raw_string(rest) {
            length
        } else if rest[0] == '"' || rest.starts_with(&['b', '"']) {
            quoted(rest, '"')
        } else if rest[0] == '\'' {
            character(rest)
        } else {
            out.push(rest[0]);
            index += 1;
            continue;
        };
        blank(&mut out, &rest[..end]);
        index += end;
    }
    out
}

fn block_comment(text: &[char]) -> usize {
    let mut depth = 0;
    let mut index = 0;
    while index < text.len() {
        if text[index..].starts_with(&['/', '*']) {
            depth += 1;
            index += 2;
        } else if text[index..].starts_with(&['*', '/']) {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    text.len()
}

/// The length of a raw string literal at the start of `text`, if one is.
fn raw_string(text: &[char]) -> Option<usize> {
    let start = match text {
        ['r', ..] => 1,
        ['b', 'r', ..] => 2,
        _ => return None,
    };
    let hashes = text[start..].iter().take_while(|c| **c == '#').count();
    if text.get(start + hashes) != Some(&'"') {
        return None;
    }
    let open = start + hashes + 1;
    let close: Vec<char> = std::iter::once('"')
        .chain(std::iter::repeat_n('#', hashes))
        .collect();
    let length = text[open..]
        .windows(close.len())
        .position(|window| window == close.as_slice())
        .map_or(text.len(), |at| open + at + close.len());
    Some(length)
}

fn quoted(text: &[char], quote: char) -> usize {
    let mut index = text.iter().position(|c| *c == quote).unwrap_or(0) + 1;
    while index < text.len() {
        match text[index] {
            '\\' => index += 2,
            c if c == quote => return index + 1,
            _ => index += 1,
        }
    }
    text.len()
}

/// A character literal's length, or one for a lifetime's quote.
fn character(text: &[char]) -> usize {
    match text {
        ['\'', '\\', ..] => quoted(text, '\''),
        ['\'', _, '\'', ..] => 3,
        _ => 1,
    }
}

fn is_identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The paths of every `use` item in `code`, each as its segments, groups
/// expanded and aliases dropped: `use std::{env as e, fs};` gives
/// `std::env` and `std::fs`.
pub(crate) fn use_paths(code: &str) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    for (start, _) in code.match_indices("use") {
        let before = code[..start].chars().next_back();
        let after = code[start + 3..].chars().next();
        if before.is_some_and(is_identifier) || after.is_none_or(|c| !c.is_whitespace()) {
            continue;
        }
        let Some(end) = code[start..].find(';') else {
            continue;
        };
        expand(
            "",
            &without_aliases(&code[start + 3..start + end]),
            &mut paths,
        );
    }
    paths
}

/// A use tree's tokens joined without spaces, each `as name` dropped, so
/// `env as e` reads as `env`.
fn without_aliases(tree: &str) -> String {
    let tokens = tokens(tree);
    let mut joined = String::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == "as" {
            index += 2;
            continue;
        }
        joined.push_str(&tokens[index]);
        index += 1;
    }
    joined
}

/// Expands one use tree below `prefix` into its paths.
fn expand(prefix: &str, tree: &str, paths: &mut Vec<Vec<String>>) {
    let tree = tree.trim_start_matches("::");
    let Some(open) = tree.find('{') else {
        let full = match (prefix, tree) {
            (_, "self") => prefix.to_owned(),
            ("", _) => tree.to_owned(),
            _ => format!("{prefix}::{tree}"),
        };
        paths.push(segments(&full));
        return;
    };
    let head = tree[..open].trim_end_matches("::");
    let inner = &tree[open + 1..tree.rfind('}').unwrap_or(tree.len())];
    let prefix = match (prefix, head) {
        (_, "") => prefix.to_owned(),
        ("", _) => head.to_owned(),
        _ => format!("{prefix}::{head}"),
    };
    let mut depth = 0;
    let mut item = String::new();
    for c in inner.chars().chain(std::iter::once(',')) {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                if !item.is_empty() {
                    expand(&prefix, &item, paths);
                }
                item.clear();
                continue;
            }
            _ => {}
        }
        item.push(c);
    }
}

fn segments(path: &str) -> Vec<String> {
    path.trim_start_matches("::")
        .split("::")
        .map(str::to_owned)
        .collect()
}

/// Every expression path in `code` that names `std` followed by `env`,
/// with the segment after `env` if there is one, however it is spaced.
pub(crate) fn expression_paths(code: &str) -> Vec<Vec<String>> {
    let tokens = tokens(code);
    let mut paths = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token != "std" || tokens.get(index + 1).map(String::as_str) != Some("::") {
            continue;
        }
        let mut path = vec!["std".to_owned()];
        let mut at = index + 2;
        while let Some(segment) = tokens.get(at) {
            path.push(segment.clone());
            if tokens.get(at + 1).map(String::as_str) != Some("::") || path.len() == 3 {
                break;
            }
            at += 2;
        }
        paths.push(path);
    }
    paths
}

/// Identifiers, `$`-fragments, `::`, `*` and single punctuation.
fn tokens(code: &str) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c.is_whitespace() {
            index += 1;
        } else if is_identifier(c) || c == '$' {
            let start = index;
            index += 1;
            while index < chars.len() && is_identifier(chars[index]) {
                index += 1;
            }
            tokens.push(chars[start..index].iter().collect());
        } else if chars[index..].starts_with(&[':', ':']) {
            tokens.push("::".to_owned());
            index += 2;
        } else {
            tokens.push(c.to_string());
            index += 1;
        }
    }
    tokens
}

/// Whether a path reaches the environment: it names `std::env` itself, a
/// glob or group of it, or an item of it that [`ALLOWED`] does not list; or
/// it globs all of `std`.
pub(crate) fn reaches_environment(path: &[String]) -> bool {
    match path {
        [std, second] => std == "std" && (second == "*" || second == "env"),
        [std, env, item, ..] => std == "std" && env == "env" && !ALLOWED.contains(&item.as_str()),
        _ => false,
    }
}

/// How many paths of `source` reach the environment.
pub(crate) fn reads(source: &str) -> usize {
    let code = code_of(source);
    let uses = use_paths(&code);
    // Expression paths inside `use` items are counted once, as use paths.
    let mut expressions = code.clone();
    for (start, _) in code.match_indices("use") {
        let before = code[..start].chars().next_back();
        let after = code[start + 3..].chars().next();
        if before.is_some_and(is_identifier) || after.is_none_or(|c| !c.is_whitespace()) {
            continue;
        }
        if let Some(end) = code[start..].find(';') {
            expressions.replace_range(start..start + end, &" ".repeat(end));
        }
    }
    uses.iter()
        .chain(expression_paths(&expressions).iter())
        .filter(|path| reaches_environment(path))
        .count()
}
