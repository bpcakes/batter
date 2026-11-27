//! Rust source reduced to what a lint attribute can be made of.
//!
//! Comments and whitespace are dropped. Each string, byte string, raw string
//! and character literal keeps its delimiters but has its contents
//! hex-encoded, so no literal reads as code and two literals still compare by
//! content. Lifetimes and labels stay as they are. A lint list such as
//! `allow(/* note */ clippy::all)` therefore reads as `allow(clippy::all)`, and
//! documentation or a string that quotes an attribute reads as no attribute.

use std::fmt::Write as _;

/// `source` reduced as the module documentation describes.
pub(super) fn normalize(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut code = String::with_capacity(source.len());
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        let next = chars.get(index + 1).copied();
        index = match character {
            '/' if next == Some('/') => line_comment_end(&chars, index),
            '/' if next == Some('*') => block_comment_end(&chars, index),
            '"' => quoted(&chars, index, &mut code),
            'r' if raw_string_start(&chars, index) => raw_string(&chars, index, &mut code),
            '\'' => character_or_lifetime(&chars, index, &mut code),
            whitespace if whitespace.is_whitespace() => index + 1,
            other => {
                code.push(other);
                index + 1
            }
        };
    }
    code
}

fn line_comment_end(chars: &[char], mut index: usize) -> usize {
    while index < chars.len() && chars[index] != '\n' {
        index += 1;
    }
    index
}

/// Past the block comment at `index`; Rust block comments nest.
fn block_comment_end(chars: &[char], mut index: usize) -> usize {
    let mut depth = 0_usize;
    while index < chars.len() {
        match (chars[index], chars.get(index + 1)) {
            ('/', Some('*')) => {
                depth += 1;
                index += 2;
            }
            ('*', Some('/')) => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    index
}

/// Writes `delimiter`, the hex-encoded `contents` and `delimiter` again.
fn encode(code: &mut String, delimiter: char, contents: &[char]) {
    code.push(delimiter);
    for character in contents {
        write!(code, "{:06x}", u32::from(*character)).expect("writing to a String cannot fail");
    }
    code.push(delimiter);
}

/// A `"`-delimited literal starting at `index`, with `\` escapes.
fn quoted(chars: &[char], index: usize, code: &mut String) -> usize {
    let mut end = index + 1;
    while end < chars.len() && chars[end] != '"' {
        end += if chars[end] == '\\' { 2 } else { 1 };
    }
    let end = end.min(chars.len());
    encode(code, '"', &chars[index + 1..end]);
    end + 1
}

/// Whether the `r` at `index` opens a raw string: it starts a token, or
/// follows a `b` or `c` prefix that does, and `#`s then `"` come next.
fn raw_string_start(chars: &[char], index: usize) -> bool {
    let starts_token = |at: usize| at == 0 || !is_identifier_part(chars[at - 1]);
    let prefixed = index > 0 && matches!(chars[index - 1], 'b' | 'c') && starts_token(index - 1);
    if !(starts_token(index) || prefixed) {
        return false;
    }
    let hashes = chars[index + 1..].iter().take_while(|c| **c == '#').count();
    chars.get(index + 1 + hashes) == Some(&'"')
}

/// The raw string starting at the `r` at `index`.
fn raw_string(chars: &[char], index: usize, code: &mut String) -> usize {
    let hashes = chars[index + 1..].iter().take_while(|c| **c == '#').count();
    let open = index + 1 + hashes + 1;
    let mut end = open;
    while end < chars.len()
        && !(chars[end] == '"'
            && chars[end + 1..]
                .iter()
                .take(hashes)
                .filter(|c| **c == '#')
                .count()
                == hashes)
    {
        end += 1;
    }
    let end = end.min(chars.len());
    code.push('r');
    encode(code, '"', &chars[open..end]);
    end + 1 + hashes
}

/// A character literal, encoded, or a lifetime or label, kept.
fn character_or_lifetime(chars: &[char], index: usize, code: &mut String) -> usize {
    let close = match chars.get(index + 1) {
        Some('\\') => chars
            .get(index + 3..)
            .and_then(|rest| rest.iter().position(|c| *c == '\''))
            .map(|offset| index + 3 + offset),
        Some(_) if chars.get(index + 2) == Some(&'\'') => Some(index + 2),
        _ => None,
    };
    if let Some(close) = close {
        encode(code, '\'', &chars[index + 1..close]);
        close + 1
    } else {
        code.push('\'');
        index + 1
    }
}

const fn is_identifier_part(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

#[test]
fn comments_whitespace_and_literal_contents_never_read_as_code() {
    let source = concat!(
        "/// Writes #[allow(warnings)] for you.\n",
        "#[allow(/* note */ clippy::all, reason = \"r\")]\n",
        "fn f<'q>(x: &'q str) -> char { /* /* nested */ allow(x) */\n",
        "    let _ = \"allow(warnings)\"; let _ = r#\"expect(dead_code, \"x\")\"#; let _ = br\"a\";\n",
        "    'label: loop { break 'label; }\n",
        "    if x.is_empty() { '\\'' } else { 'a' }\n",
        "}\n",
    );
    let code = normalize(source);
    assert!(code.starts_with("#[allow(clippy::all,reason=\"000072\")]fnf<'q>(x:&'qstr)->char{"));
    assert!(!code.contains("warnings") && !code.contains("dead_code"));
    assert!(code.contains("'label:loop{break'label;}"));
    // An escaped quote keeps both characters of its contents.
    assert!(code.contains("{'00005c000027'}else{'000061'}"));
    assert!(code.contains("let_=r\"000065000078"));
    assert!(code.contains("let_=br\"000061\""));
}
