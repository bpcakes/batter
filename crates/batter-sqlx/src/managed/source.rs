//! Conservative lexical validation of one managed-object statement.
//!
//! The scanner follows PostgreSQL's lexical rules only as far as needed to find
//! the statement header and its end: whitespace, `--` and nested `/* */`
//! comments, standard and `E''` strings, quoted identifiers, dollar quotes and
//! positional parameters. Statement termination follows `psql`'s heuristic for
//! `BEGIN ATOMIC` routine bodies, so a source file that `psql` runs as one
//! statement is accepted here as one statement.

use super::{Identity, MAX_SOURCE_BYTES, ManagedObjectError, ManagedObjectKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Token<'a> {
    /// An unquoted word: keyword, identifier or number text, as written.
    Word(&'a str),
    /// A quoted identifier with doubled quotes collapsed.
    Quoted(String),
    /// One punctuation or operator character.
    Punct(u8),
    /// A string constant, dollar-quoted string or positional parameter.
    Literal,
}

pub(super) fn validate(
    kind: ManagedObjectKind,
    identity: &Identity,
    source: &str,
) -> Result<(), ManagedObjectError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ManagedObjectError::SourceTooLarge);
    }
    if source.contains('\0') {
        return Err(ManagedObjectError::SourceStatement);
    }
    let tokens = scan(source)?;
    header(kind, identity, &tokens)?;
    single_statement(&tokens, kind != ManagedObjectKind::Trigger)
}

pub(super) fn scan(source: &str) -> Result<Vec<Token<'_>>, ManagedObjectError> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => index += 1,
            b'-' if next == Some(b'-') => {
                index = bytes[index..]
                    .iter()
                    .position(|byte| matches!(byte, b'\n' | b'\r'))
                    .map_or(bytes.len(), |offset| index + offset);
            }
            b'/' if next == Some(b'*') => index = block_comment(bytes, index)?,
            b'\'' => {
                index = string(bytes, index, false)?;
                tokens.push(Token::Literal);
            }
            b'"' => {
                let (identifier, end) = quoted_identifier(bytes, index)?;
                tokens.push(Token::Quoted(identifier));
                index = end;
            }
            b'$' => {
                if let Some(opening) = dollar_tag(bytes, index) {
                    let tag = &bytes[index..opening];
                    let close =
                        find(&bytes[opening..], tag).ok_or(ManagedObjectError::SourceStatement)?;
                    index = opening + close + tag.len();
                    tokens.push(Token::Literal);
                } else if next.is_some_and(|byte| byte.is_ascii_digit()) {
                    index += 1;
                    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                        index += 1;
                    }
                    tokens.push(Token::Literal);
                } else {
                    tokens.push(Token::Punct(byte));
                    index += 1;
                }
            }
            byte if identifier_start(byte) => {
                let start = index;
                index += 1;
                while bytes.get(index).copied().is_some_and(identifier_continue) {
                    index += 1;
                }
                let word = &source[start..index];
                if bytes.get(index) == Some(&b'\'') && word.eq_ignore_ascii_case("e") {
                    index = string(bytes, index, true)?;
                    tokens.push(Token::Literal);
                } else {
                    tokens.push(Token::Word(word));
                }
            }
            byte if byte.is_ascii_digit() => {
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
                {
                    index += 1;
                }
                tokens.push(Token::Literal);
            }
            byte => {
                tokens.push(Token::Punct(byte));
                index += 1;
            }
        }
    }
    Ok(tokens)
}

const fn identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

const fn identifier_continue(byte: u8) -> bool {
    identifier_start(byte) || byte.is_ascii_digit() || byte == b'$'
}

fn block_comment(bytes: &[u8], start: usize) -> Result<usize, ManagedObjectError> {
    let mut depth = 0usize;
    let mut index = start;
    while index < bytes.len() {
        match (bytes[index], bytes.get(index + 1)) {
            (b'/', Some(b'*')) => {
                depth += 1;
                index += 2;
            }
            (b'*', Some(b'/')) => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => index += 1,
        }
    }
    Err(ManagedObjectError::SourceStatement)
}

/// Return the index after the closing quote of a string starting at `start`.
fn string(bytes: &[u8], start: usize, escapes: bool) -> Result<usize, ManagedObjectError> {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if escapes => index += 2,
            b'\'' if bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => return Ok(index + 1),
            _ => index += 1,
        }
    }
    Err(ManagedObjectError::SourceStatement)
}

fn quoted_identifier(bytes: &[u8], start: usize) -> Result<(String, usize), ManagedObjectError> {
    let mut identifier = Vec::new();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' if bytes.get(index + 1) == Some(&b'"') => {
                identifier.push(b'"');
                index += 2;
            }
            b'"' => {
                if identifier.is_empty() {
                    return Err(ManagedObjectError::SourceStatement);
                }
                let identifier = String::from_utf8(identifier)
                    .map_err(|_| ManagedObjectError::SourceStatement)?;
                return Ok((identifier, index + 1));
            }
            byte => {
                identifier.push(byte);
                index += 1;
            }
        }
    }
    Err(ManagedObjectError::SourceStatement)
}

/// Return the index after an opening `$tag$` delimiter at `start`.
fn dollar_tag(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    match bytes.get(index) {
        Some(b'$') => return Some(index + 1),
        Some(&byte) if identifier_start(byte) => index += 1,
        _ => return None,
    }
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'$' => return Some(index + 1),
            byte if identifier_start(byte) || byte.is_ascii_digit() => index += 1,
            _ => return None,
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn header(
    kind: ManagedObjectKind,
    identity: &Identity,
    tokens: &[Token<'_>],
) -> Result<(), ManagedObjectError> {
    let mut cursor = Cursor { tokens, index: 0 };
    let keyword = match kind {
        ManagedObjectKind::Function => "function",
        ManagedObjectKind::Procedure => "procedure",
        ManagedObjectKind::Trigger => "trigger",
    };
    for expected in ["create", "or", "replace", keyword] {
        if !cursor.keyword(expected) {
            return Err(ManagedObjectError::SourceHeader);
        }
    }
    match identity {
        Identity::Routine(signature) => {
            let (schema, name) = cursor.qualified_name()?;
            if schema != signature.schema()
                || name != signature.name()
                || cursor.next() != Some(&Token::Punct(b'('))
            {
                return Err(ManagedObjectError::SourceIdentity);
            }
        }
        Identity::Trigger { table, name } => {
            let trigger = cursor
                .identifier()
                .ok_or(ManagedObjectError::SourceHeader)?;
            if trigger != name.as_str() {
                return Err(ManagedObjectError::SourceIdentity);
            }
            cursor.trigger_events()?;
            let (schema, relation) = cursor.qualified_name()?;
            if schema != table.schema()
                || relation != table.name()
                || cursor.peek() == Some(&Token::Punct(b'.'))
            {
                return Err(ManagedObjectError::SourceIdentity);
            }
        }
    }
    Ok(())
}

struct Cursor<'t, 'a> {
    tokens: &'t [Token<'a>],
    index: usize,
}

impl<'a> Cursor<'_, 'a> {
    fn peek(&self) -> Option<&Token<'a>> {
        self.tokens.get(self.index)
    }

    fn next(&mut self) -> Option<&Token<'a>> {
        let token = self.tokens.get(self.index);
        self.index += 1;
        token
    }

    fn keyword(&mut self, expected: &str) -> bool {
        let matched =
            matches!(self.peek(), Some(Token::Word(word)) if word.eq_ignore_ascii_case(expected));
        if matched {
            self.index += 1;
        }
        matched
    }

    /// One identifier as PostgreSQL resolves it: unquoted words fold ASCII
    /// letters to lowercase; quoted identifiers keep their exact spelling.
    fn identifier(&mut self) -> Option<String> {
        let identifier = match self.peek()? {
            Token::Word(word) => word.to_ascii_lowercase(),
            Token::Quoted(identifier) => identifier.clone(),
            Token::Punct(_) | Token::Literal => return None,
        };
        self.index += 1;
        Some(identifier)
    }

    fn qualified_name(&mut self) -> Result<(String, String), ManagedObjectError> {
        let schema = self
            .identifier()
            .ok_or(ManagedObjectError::SourceIdentity)?;
        if self.next() != Some(&Token::Punct(b'.')) {
            return Err(ManagedObjectError::SourceIdentity);
        }
        let name = self
            .identifier()
            .ok_or(ManagedObjectError::SourceIdentity)?;
        Ok((schema, name))
    }

    /// `{BEFORE | AFTER | INSTEAD OF} event [OR event ...] ON`, where an event is
    /// `INSERT`, `DELETE`, `TRUNCATE` or `UPDATE [OF column [, ...]]`.
    fn trigger_events(&mut self) -> Result<(), ManagedObjectError> {
        let timed = self.keyword("before")
            || self.keyword("after")
            || (self.keyword("instead") && self.keyword("of"));
        if !timed {
            return Err(ManagedObjectError::SourceHeader);
        }
        loop {
            if self.keyword("update") {
                if self.keyword("of") {
                    loop {
                        self.identifier().ok_or(ManagedObjectError::SourceHeader)?;
                        if self.peek() != Some(&Token::Punct(b',')) {
                            break;
                        }
                        self.index += 1;
                    }
                }
            } else if !(self.keyword("insert")
                || self.keyword("delete")
                || self.keyword("truncate"))
            {
                return Err(ManagedObjectError::SourceHeader);
            }
            if !self.keyword("or") {
                break;
            }
        }
        if self.keyword("on") {
            Ok(())
        } else {
            Err(ManagedObjectError::SourceHeader)
        }
    }
}

/// Accept one statement with at most one trailing semicolon. Inside a
/// `CREATE FUNCTION` or `CREATE PROCEDURE` statement, `psql` treats unquoted
/// `BEGIN` ... `END` (and `CASE` ... `END` within it) outside parentheses as a
/// `BEGIN ATOMIC` body whose semicolons do not end the statement.
fn single_statement(tokens: &[Token<'_>], routine: bool) -> Result<(), ManagedObjectError> {
    let mut parentheses = 0usize;
    let mut begin = 0usize;
    let mut terminated = false;
    for token in tokens {
        if terminated {
            return Err(ManagedObjectError::SourceStatement);
        }
        match token {
            Token::Punct(b'(') => parentheses += 1,
            Token::Punct(b')') => parentheses = parentheses.saturating_sub(1),
            Token::Punct(b';') if parentheses == 0 && begin == 0 => terminated = true,
            Token::Word(word) if routine && parentheses == 0 => {
                if word.eq_ignore_ascii_case("begin")
                    || (word.eq_ignore_ascii_case("case") && begin > 0)
                {
                    begin += 1;
                } else if word.eq_ignore_ascii_case("end") && begin > 0 {
                    begin -= 1;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
