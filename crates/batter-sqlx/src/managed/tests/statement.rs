use super::*;

const STATEMENT: &str =
    "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint LANGUAGE sql
    -- inside the statement
    RETURN amount /* also inside */ * 2;";

/// Leading text that is only whitespace and comments.
const LEADING: &[&str] = &[
    " ",
    "\t",
    "\n",
    "\r",
    "\u{b}",
    "\u{c}",
    " \t\r\n\u{b}\u{c}",
    "-- ended by a line feed\n",
    "-- ended by a carriage return\r",
    "--\n",
    "-- one\n-- two; with a semicolon\n",
    "/* block */",
    "/**/",
    "/* outer /* inner */ still outer */",
    "/* a /* b /* c */ b */ a */",
    "/* a lone * and a lone / and ** / close nothing */",
    "/* -- not a line comment */",
    "-- /* not a block comment\n",
    "/* multibyte: \u{e9}\u{4e16}\u{1f600} */\n-- \u{e9}\u{4e16}\u{1f600}\n",
    "\n\n/* header */\n-- note\n\n",
];

#[test]
fn a_statement_starts_at_its_first_token() {
    assert_eq!(statement_start(STATEMENT), Ok(STATEMENT));
    for leading in LEADING {
        let source = format!("{leading}{STATEMENT}");
        assert_eq!(statement_start(&source), Ok(STATEMENT), "{leading:?}");
    }
    // A block comment ends at the close of its outermost opening, so what
    // follows it is statement text, not a comment.
    assert_eq!(
        statement_start("/* a /* b */ c */ d */ CREATE"),
        Ok("d */ CREATE")
    );
    // A lone `-` or `/` is a token, and a line comment needs no terminator.
    for (source, expected) in [
        ("", ""),
        (" \n\t", ""),
        ("-- only a comment", ""),
        ("/* only a comment */", ""),
        (" - 1", "- 1"),
        (" / 2", "/ 2"),
        (" -/* x */", "-/* x */"),
        ("\n;", ";"),
        ("\u{a0}CREATE", "\u{a0}CREATE"),
    ] {
        assert_eq!(statement_start(source), Ok(expected), "{source:?}");
    }
}

#[test]
fn unreadable_leading_text_is_refused_as_construction_refuses_it() {
    for source in [
        "/* never closed",
        "/* outer /* inner */ never closed",
        "/*/",
        "-- a NUL in a comment \0\nCREATE",
        "/* a NUL in a block comment \0 */ CREATE",
        "-- a NUL in a comment that ends the source \0",
    ] {
        assert_eq!(
            statement_start(source),
            Err(ManagedObjectError::SourceStatement),
            "{source:?}"
        );
        assert_eq!(
            function(source.to_owned().leak()),
            Err(ManagedObjectError::SourceStatement),
            "{source:?}"
        );
    }
}

/// Only the skipped text is read. A NUL from the first token on is still in
/// the slice, where construction refuses it.
#[test]
fn text_from_the_first_token_on_is_left_to_construction() {
    const ROUTINE: &str =
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1; \0";
    for (source, expected) in [
        ("\0", "\0"),
        (" \0CREATE", "\0CREATE"),
        (format!("-- header\n{ROUTINE}").leak() as &str, ROUTINE),
        (
            "/* header */ CREATE /* never closed",
            "CREATE /* never closed",
        ),
    ] {
        assert_eq!(statement_start(source), Ok(expected), "{source:?}");
        for refused in [source, expected] {
            assert_eq!(
                function(refused),
                Err(ManagedObjectError::SourceStatement),
                "{refused:?}"
            );
        }
    }
}

/// The slice carries no size bound of its own, so leading text can be what
/// puts a source over the per-source bound.
#[test]
fn leading_text_counts_toward_the_bound_only_when_it_is_passed() {
    const HEADER: &str = "-- twenty byte header\n";
    let statement = format!("{TOTAL}{}", " ".repeat(MAX_SOURCE_BYTES - TOTAL.len()));
    assert_eq!(statement.len(), MAX_SOURCE_BYTES);
    let source: &'static str = format!("{HEADER}{statement}").leak();
    assert_eq!(function(source), Err(ManagedObjectError::SourceTooLarge));
    let slice = statement_start(source).unwrap();
    assert_eq!(slice, statement);
    assert!(function(slice).is_ok());
}

/// The slice is validated exactly as the whole source is, tokenizes to the
/// same tokens, and differs in checksum exactly when something was skipped.
#[test]
fn the_slice_is_the_same_statement_to_construction_and_the_scanner() {
    let statements = [
        STATEMENT,
        TOTAL,
        // Rejected sources stay rejected, with the same error.
        "CREATE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1; SELECT 1;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql AS 'unterminated",
        "",
    ];
    for statement in statements {
        for leading in LEADING.iter().copied().chain([""]) {
            let source: &'static str = format!("{leading}{statement}").leak();
            let slice = statement_start(source).unwrap();
            assert!(source.ends_with(slice));
            assert_eq!(slice, statement_start(statement).unwrap(), "{source:?}");
            assert_eq!(scan(slice), scan(source), "{source:?}");
            let whole = function(source);
            let sliced = function(slice);
            assert_eq!(
                whole.as_ref().map(|_| ()),
                sliced.as_ref().map(|_| ()),
                "{source:?}"
            );
            if let (Ok(whole), Ok(sliced)) = (whole, sliced) {
                assert_eq!(whole.identity(), sliced.identity());
                assert_eq!(
                    whole.checksum() == sliced.checksum(),
                    slice.len() == source.len(),
                    "{source:?}"
                );
            }
        }
    }
}
