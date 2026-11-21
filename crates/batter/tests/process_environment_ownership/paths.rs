//! Every path through which a source reaches `std::env`, found in code with
//! comments and literals removed, so no spelling of an import or a call is
//! missed: a leading `::`, an alias, `self`, a glob, a group wrapped over
//! several lines and a macro fragment such as `std::env::$function` all
//! count.
//!
//! The reader does not resolve names, so it is conservative: an import or an
//! alias of `std` itself, `extern crate std`, any path below `std::env` that
//! names an item [`ALLOWED`] does not list, and any `env::` path to one of
//! [`READS`] whatever names its root all count. Raw identifiers are read as
//! their names and are never keywords. It cannot see a path that a
//! procedural macro generates or code that `include!` inserts.

/// Items of `std::env` that read nothing of the environment: types, the
/// process's arguments and executable, and the helpers that only transform
/// values the caller supplies.
pub(crate) const ALLOWED: [&str; 13] = [
    "VarError",
    "JoinPathsError",
    "SplitPaths",
    "Vars",
    "VarsOs",
    "Args",
    "ArgsOs",
    "args",
    "args_os",
    "current_exe",
    "consts",
    "join_paths",
    "split_paths",
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

/// One token of code with comments and literals removed.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// An identifier, a keyword or a macro fragment such as `$root`; a raw
    /// identifier `r#name` keeps its name and is marked raw, so it is never
    /// a keyword.
    Ident {
        name: String,
        raw: bool,
    },
    /// `::`.
    Path,
    Punct(char),
}

impl Token {
    fn named(&self, word: &str) -> bool {
        matches!(self, Self::Ident { name, .. } if name == word)
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self, Self::Ident { name, raw: false } if name == word)
    }
}

fn tokens(code: &str) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let raw = chars[index..].starts_with(&['r', '#'])
            && chars.get(index + 2).is_some_and(|c| is_identifier(*c));
        if c.is_whitespace() {
            index += 1;
        } else if raw || is_identifier(c) || c == '$' {
            let start = if raw { index + 2 } else { index };
            index = start + 1;
            while index < chars.len() && is_identifier(chars[index]) {
                index += 1;
            }
            tokens.push(Token::Ident {
                name: chars[start..index].iter().collect(),
                raw,
            });
        } else if chars[index..].starts_with(&[':', ':']) {
            tokens.push(Token::Path);
            index += 2;
        } else {
            tokens.push(Token::Punct(c));
            index += 1;
        }
    }
    tokens
}

/// The functions that read or change the environment, or a directory found
/// through it, counted below any `env` path, whatever names its root.
pub(crate) const READS: [&str; 10] = [
    "var",
    "var_os",
    "vars",
    "vars_os",
    "set_var",
    "remove_var",
    "current_dir",
    "set_current_dir",
    "home_dir",
    "temp_dir",
];

/// The paths of one use tree starting at `tokens[*at]`, each as its
/// segments, groups expanded, `self` resolved and aliases dropped.
fn use_tree(tokens: &[Token], at: &mut usize, prefix: &[String], out: &mut Vec<Vec<String>>) {
    let mut path = prefix.to_vec();
    if tokens.get(*at) == Some(&Token::Path) {
        *at += 1;
    }
    loop {
        match tokens.get(*at) {
            Some(Token::Ident { name, .. }) => {
                if !(name == "self" && path.len() == prefix.len() && !prefix.is_empty()) {
                    path.push(name.clone());
                }
                *at += 1;
            }
            Some(Token::Punct('*')) => {
                path.push("*".to_owned());
                *at += 1;
                break;
            }
            Some(Token::Punct('{')) => {
                *at += 1;
                loop {
                    match tokens.get(*at) {
                        Some(Token::Punct('}')) | None => {
                            *at += 1;
                            return;
                        }
                        Some(Token::Punct(',')) => *at += 1,
                        Some(_) => {
                            // A token no tree starts with, such as a macro
                            // repetition, is skipped, so the walk always ends.
                            let before = *at;
                            use_tree(tokens, at, &path, out);
                            if *at == before {
                                *at += 1;
                            }
                        }
                    }
                }
            }
            Some(_) | None => break,
        }
        if tokens.get(*at) == Some(&Token::Path) {
            *at += 1;
        } else {
            break;
        }
    }
    if tokens.get(*at).is_some_and(|token| token.keyword("as")) {
        *at += 2;
    }
    out.push(path);
}

/// Whether a `use` path reaches the environment: it imports `std` itself or
/// all of it, `std::env` itself, a glob or group of it, an item of it that
/// [`ALLOWED`] does not list, or one of [`READS`] below any `env`.
fn use_reaches(path: &[String]) -> bool {
    // A reading function below any `env`, whatever names its root, as in a
    // macro's `use $root::env::var`.
    if path
        .windows(2)
        .any(|pair| pair[0] == "env" && READS.contains(&pair[1].as_str()))
    {
        return true;
    }
    match path {
        [std] => std == "std",
        [std, second] => std == "std" && (second == "*" || second == "env"),
        [std, env, item, ..] => std == "std" && env == "env" && !ALLOWED.contains(&item.as_str()),
        [] => false,
    }
}

/// How many paths of `source` reach the environment: every `use` path that
/// does, `extern crate std`, every expression path below `std::env` that
/// names an item [`ALLOWED`] does not list, and every `env::` path to one of
/// [`READS`] whatever its root, so a macro fragment such as `$root::env::var`
/// or an alias of `std` is counted too.
pub(crate) fn reads(source: &str) -> usize {
    let tokens = tokens(&code_of(source));
    let mut count = 0;
    let mut index = 0;
    while index < tokens.len() {
        // `use<..>` is a precise capture bound, not an import.
        if tokens[index].keyword("use") && tokens.get(index + 1) != Some(&Token::Punct('<')) {
            let mut at = index + 1;
            let mut paths = Vec::new();
            while at < tokens.len() && tokens[at] != Token::Punct(';') {
                let before = at;
                use_tree(&tokens, &mut at, &[], &mut paths);
                if at == before {
                    at += 1;
                }
            }
            count += paths.iter().filter(|path| use_reaches(path)).count();
            index = at + 1;
            continue;
        }
        if tokens[index].keyword("extern")
            && tokens
                .get(index + 1)
                .is_some_and(|token| token.keyword("crate"))
            && tokens
                .get(index + 2)
                .is_some_and(|token| token.named("std"))
        {
            count += 1;
            index += 3;
            continue;
        }
        let segment = |offset: usize| match tokens.get(index + offset) {
            Some(Token::Ident { name, .. }) => Some(name.as_str()),
            Some(_) | None => None,
        };
        let path_at = |offset: usize| tokens.get(index + offset) == Some(&Token::Path);
        if tokens[index].named("std") && path_at(1) && segment(2) == Some("env") {
            // `std::env` and the item after it, if any.
            let reaches = if path_at(3) {
                match tokens.get(index + 4) {
                    Some(Token::Ident { name, .. }) => !ALLOWED.contains(&name.as_str()),
                    Some(_) | None => true,
                }
            } else {
                true
            };
            count += usize::from(reaches);
            index += 3;
            continue;
        }
        if tokens[index].named("env")
            && path_at(1)
            && segment(2).is_some_and(|item| READS.contains(&item))
        {
            count += 1;
            index += 3;
            continue;
        }
        index += 1;
    }
    count
}
