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

/// The text from the first token on, as `FromStatementStart` checksums it.
fn from_start(source: &str) -> Result<&str, ManagedObjectError> {
    statement_start(source).map(|index| &source[index..])
}

fn set(rule: SourceChecksum, objects: Vec<ManagedObject>) -> ManagedObjectManifest {
    ManagedObjectManifest::new(
        ManagedRevision::new(1).expect("valid revision"),
        ledger(),
        PublicExecute::Revoke,
        rule,
        objects,
        [],
    )
    .unwrap()
}

fn recorded(manifest: &ManagedObjectManifest) -> [u8; 32] {
    let mut checksums = manifest.object_checksums();
    assert_eq!(checksums.len(), 1);
    *checksums.next().unwrap().1
}

fn sha(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

#[test]
fn a_statement_starts_at_its_first_token() {
    assert_eq!(from_start(STATEMENT), Ok(STATEMENT));
    for leading in LEADING {
        let source = format!("{leading}{STATEMENT}");
        assert_eq!(from_start(&source), Ok(STATEMENT), "{leading:?}");
    }
    // A block comment ends at the close of its outermost opening, so what
    // follows it is statement text, not a comment.
    assert_eq!(
        from_start("/* a /* b */ c */ d */ CREATE"),
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
        assert_eq!(from_start(source), Ok(expected), "{source:?}");
    }
}

#[test]
fn unreadable_leading_text_is_refused_at_construction() {
    for source in [
        "/* never closed",
        "/* outer /* inner */ never closed",
        "/*/",
        "-- a NUL in a comment \0\nCREATE",
        "/* a NUL in a block comment \0 */ CREATE",
        "-- a NUL in a comment that ends the source \0",
    ] {
        assert_eq!(
            function(source),
            Err(ManagedObjectError::SourceStatement),
            "{source:?}"
        );
    }
}

/// The bounds apply to the whole source under either rule: leading text that
/// is not checksummed is still executed.
#[test]
fn leading_text_counts_toward_the_source_bound_under_either_rule() {
    const HEADER: &str = "-- a header comment\n";
    let statement = format!("{TOTAL}{}", " ".repeat(MAX_SOURCE_BYTES - TOTAL.len()));
    assert_eq!(statement.len(), MAX_SOURCE_BYTES);
    assert!(function(statement.clone().leak()).is_ok());
    assert_eq!(
        function(format!("{HEADER}{statement}").leak()),
        Err(ManagedObjectError::SourceTooLarge)
    );
}

/// Under `FromStatementStart` every recorded checksum and the set digest are
/// those of the bare statement, whatever leads it; under `WholeSource` they
/// change exactly when something leads it. Apply still executes the whole
/// source under both rules.
#[test]
fn the_rule_decides_checksums_and_digest_for_the_whole_set() {
    for statement in [STATEMENT, TOTAL] {
        let bare = |rule| set(rule, vec![function(statement).unwrap()]);
        assert_eq!(recorded(&bare(SourceChecksum::WholeSource)), sha(statement));
        assert_eq!(
            bare(SourceChecksum::WholeSource).digest(),
            bare(SourceChecksum::FromStatementStart).digest()
        );
        for leading in LEADING.iter().copied().chain([""]) {
            let source: &'static str = format!("{leading}{statement}").leak();
            assert_eq!(
                scan(from_start(source).unwrap()),
                scan(source),
                "{source:?}"
            );
            let led = |rule| set(rule, vec![function(source).unwrap()]);
            let sliced = led(SourceChecksum::FromStatementStart);
            assert_eq!(sliced.objects()[0].source(), source);
            assert_eq!(recorded(&sliced), sha(statement), "{source:?}");
            assert_eq!(
                sliced.digest(),
                bare(SourceChecksum::FromStatementStart).digest()
            );
            let whole = led(SourceChecksum::WholeSource);
            assert_eq!(recorded(&whole), sha(source), "{source:?}");
            assert_eq!(
                whole.digest() == bare(SourceChecksum::WholeSource).digest(),
                leading.is_empty(),
                "{source:?}"
            );
        }
    }
}

/// A manifest sets every checksum from its own rule, so objects taken from a
/// set under one rule carry nothing of it into a set under the other.
#[test]
fn objects_reused_across_sets_take_the_new_sets_rule() {
    let source: &'static str = format!("-- header\n{TOTAL}").leak();
    let whole = set(SourceChecksum::WholeSource, vec![function(source).unwrap()]);
    let sliced = set(SourceChecksum::FromStatementStart, whole.objects().to_vec());
    assert_eq!(recorded(&sliced), sha(TOTAL));
    let back = set(SourceChecksum::WholeSource, sliced.objects().to_vec());
    assert_eq!(recorded(&back), sha(source));
    assert_eq!(back.digest(), whole.digest());
    assert_eq!(back, whole);
}
