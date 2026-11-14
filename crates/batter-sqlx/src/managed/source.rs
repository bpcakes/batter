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
    single_statement(
        &tokens,
        if kind.is_routine() {
            StatementKind::Routine
        } else {
            StatementKind::Trigger
        },
    )
}

pub(super) fn scan(source: &str) -> Result<Vec<Token<'_>>, ManagedObjectError> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if let Some(end) = trivia(bytes, index)? {
            index = end;
            continue;
        }
        let next = bytes.get(index + 1).copied();
        match byte {
            b'\'' => {
                index = string(bytes, index, StringSyntax::Literal)?;
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
                    index = string(bytes, index, StringSyntax::Escaped)?;
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

/// Return the index after the whitespace byte or comment that starts at
/// `index`, or `None` when a token (or the end of the source) is there.
///
/// This is the one definition of what precedes and separates tokens: [`scan`]
/// and [`statement_start`] both skip through it, so they cannot disagree.
fn trivia(bytes: &[u8], index: usize) -> Result<Option<usize>, ManagedObjectError> {
    let next = bytes.get(index + 1).copied();
    Ok(match bytes.get(index) {
        Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) => Some(index + 1),
        Some(b'-') if next == Some(b'-') => Some(
            bytes[index..]
                .iter()
                .position(|byte| matches!(byte, b'\n' | b'\r'))
                .map_or(bytes.len(), |offset| index + offset),
        ),
        Some(b'/') if next == Some(b'*') => Some(block_comment(bytes, index)?),
        _ => None,
    })
}

/// Return `source` from its first token on.
///
/// Leading whitespace, `--` comments and nested `/* */` comments are skipped
/// exactly as manifest construction skips them. PostgreSQL stores nothing that
/// precedes a statement, so passing the returned slice to
/// [`ManagedObject::function`](super::ManagedObject::function), `procedure`,
/// `trigger` or `constraint_trigger` installs the same definition while
/// keeping a header comment out of the object's source checksum and the set
/// digest.
///
/// Only leading text is removed. Everything from the first token on stays in
/// the checksum, including text PostgreSQL does not store: comments and
/// whitespace inside the statement (PostgreSQL keeps those inside a string
/// constant, such as a dollar-quoted body, and no others) and whatever follows
/// the statement, such as a final newline. The scanner does not tell stored text from unstored
/// text there, so an edit to any of it is still a set change.
///
/// This is opt-in, per call. A [`ManagedObject`](super::ManagedObject)
/// checksums exactly the source it is given, so without this function an edit
/// to a header comment changes the checksum: the manifest then needs a higher
/// [`ManagedRevision`](super::ManagedRevision), and apply executes that object
/// again, taking the table locks its statement takes. Slice every source, in
/// every place that builds the set or computes its digest; a source passed
/// whole in one of them gives a different checksum.
///
/// Starting to pass slices is itself a set change. Every object whose source
/// had leading text gets a new checksum, so that release needs a higher
/// revision and apply executes all of those objects again in its one
/// transaction, with their table locks, although their definitions do not
/// change.
///
/// The function reads only the leading text. It validates nothing from the
/// first token on and applies no size bound; the constructor still validates
/// the returned statement, so a source that construction accepts whole is
/// accepted sliced. The reverse fails only at a size bound: a source that is
/// over one because of its leading text is refused whole and accepted sliced.
/// An empty slice is returned when the source holds no token.
///
/// ```
/// use batter_sqlx::managed::{ManagedObject, statement_start};
/// use batter_sqlx::verification::{RoutineSignature, RoutineType};
///
/// let signature = RoutineSignature::new("app", "total", [RoutineType::new("pg_catalog", "int8")?])?;
/// let statement = "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint
///     LANGUAGE sql /* kept: inside the statement */ RETURN amount * 2;";
/// let first = format!("-- Doubles an amount.\n{statement}");
/// let reworded = format!("/* Doubles /* any */ amount. */\n\n{statement}");
///
/// assert_eq!(statement_start(&first)?, statement);
/// assert_eq!(statement_start(&reworded)?, statement);
/// // Rewording the header no longer changes the checksum.
/// let declared = ManagedObject::function(signature.clone(), statement_start(&first)?.to_owned())?;
/// let edited = ManagedObject::function(signature.clone(), statement_start(&reworded)?.to_owned())?;
/// assert_eq!(declared.checksum(), edited.checksum());
/// // Passing the whole file still works, and still checksums the header.
/// assert_ne!(ManagedObject::function(signature, first)?.checksum(), declared.checksum());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
/// Returns [`ManagedObjectError::SourceStatement`] for a NUL byte in the
/// skipped text, which the slice would otherwise hide from construction, or a
/// leading block comment that is never closed.
pub fn statement_start(source: &str) -> Result<&str, ManagedObjectError> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while let Some(end) = trivia(bytes, index)? {
        index = end;
    }
    // Every skipped item ends after an ASCII byte, before a line terminator or
    // at the end of the source, so `index` is a character boundary.
    let (skipped, statement) = source.split_at(index);
    if skipped.contains('\0') {
        return Err(ManagedObjectError::SourceStatement);
    }
    Ok(statement)
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
fn string(bytes: &[u8], start: usize, escapes: StringSyntax) -> Result<usize, ManagedObjectError> {
    let escapes = escapes == StringSyntax::Escaped;
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
    // PostgreSQL has no `CREATE OR REPLACE CONSTRAINT TRIGGER`, and a plain
    // trigger's `OR REPLACE` header cannot name a constraint trigger.
    let keywords: &[&str] = match kind {
        ManagedObjectKind::Function => &["create", "or", "replace", "function"],
        ManagedObjectKind::Procedure => &["create", "or", "replace", "procedure"],
        ManagedObjectKind::Trigger => &["create", "or", "replace", "trigger"],
        ManagedObjectKind::ConstraintTrigger => &["create", "constraint", "trigger"],
    };
    for expected in keywords {
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
fn single_statement(
    tokens: &[Token<'_>],
    routine: StatementKind,
) -> Result<(), ManagedObjectError> {
    let routine = routine == StatementKind::Routine;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StringSyntax {
    Escaped,
    Literal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatementKind {
    Routine,
    Trigger,
}
