//! The one set-wide rule for which source bytes a checksum covers.

/// Which bytes of every declared source a [`ManagedObjectManifest`](super::ManagedObjectManifest)
/// checksums.
///
/// The rule belongs to the whole set, not to an object: the manifest computes
/// every object's source checksum, which its ledger row records, and the set
/// digest from one rule, so no object's checksum can follow a different rule
/// from the others. There is no default. A set states its rule just as it
/// states its [`PublicExecute`](super::PublicExecute) policy.
///
/// Apply always executes, and [`ManagedObject::source`](super::ManagedObject::source)
/// always returns, the whole declared source. The rule changes only what is
/// checksummed.
///
/// Changing the rule of an existing set is a set change when any source has
/// leading text: every such object gets a new checksum, so that release needs
/// a higher [`ManagedRevision`](super::ManagedRevision) and apply executes all
/// of those objects again in its one transaction, although their definitions
/// do not change. Each holds its table locks until commit: `SHARE ROW
/// EXCLUSIVE` for a plain trigger, and `ACCESS EXCLUSIVE`, which also blocks
/// reads, for a constraint trigger, which apply drops and re-creates. When no
/// source has leading text, both rules give the same checksums and digest.
///
/// ```
/// use batter_sqlx::managed::{ManagedObject, ManagedObjectManifest, ManagedRevision, PublicExecute, SourceChecksum};
/// use batter_sqlx::verification::{QualifiedName, RoutineSignature, RoutineType};
///
/// let statement = "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint
///     LANGUAGE sql /* inside the statement */ RETURN amount * 2;";
/// let set = |rule, header: &str| {
///     let signature = RoutineSignature::new("app", "total", [RoutineType::new("pg_catalog", "int8")?])?;
///     ManagedObjectManifest::new(
///         ManagedRevision::new(1).expect("valid revision"),
///         QualifiedName::new("app", "managed_objects")?,
///         PublicExecute::Revoke,
///         rule,
///         [ManagedObject::function(signature, format!("{header}{statement}"))?],
///         [],
///     )
///     .map_err(Box::<dyn std::error::Error>::from)
/// };
/// let first = set(SourceChecksum::FromStatementStart, "-- Doubles an amount.\n")?;
/// let reworded = set(SourceChecksum::FromStatementStart, "/* Doubles /* any */ amount. */\n\n")?;
/// // Rewording the header changes neither the set digest nor any object's checksum.
/// assert_eq!(first.digest(), reworded.digest());
/// // Apply still executes the whole source.
/// assert!(reworded.objects()[0].source().starts_with("/* Doubles"));
/// // Under the whole-source rule the same edit is a set change.
/// assert_ne!(
///     set(SourceChecksum::WholeSource, "-- Doubles an amount.\n")?.digest(),
///     set(SourceChecksum::WholeSource, "/* Doubles /* any */ amount. */\n\n")?.digest(),
/// );
/// // Without leading text, the two rules agree.
/// assert_eq!(
///     set(SourceChecksum::WholeSource, "")?.digest(),
///     set(SourceChecksum::FromStatementStart, "")?.digest(),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceChecksum {
    /// Checksum every byte of each source.
    ///
    /// Every ledger written before this rule could be chosen records these
    /// checksums, so an existing set keeps its digest under this rule. Any
    /// edit, including to a header comment, is a set change.
    WholeSource,
    /// Checksum each source from its first token on.
    ///
    /// Leading whitespace, `--` comments and nested `/* */` comments are left
    /// out, exactly as manifest construction skips them before the statement
    /// header. PostgreSQL stores nothing that precedes a statement, so editing
    /// that text changes neither the installed definition nor the set.
    ///
    /// Only leading text is left out. Everything from the first token on is
    /// checksummed, including text PostgreSQL does not store: comments and
    /// whitespace inside the statement (PostgreSQL keeps those inside a string
    /// constant, such as a dollar-quoted body, and no others) and whatever
    /// follows the statement, such as a final newline. An edit to any of it is
    /// still a set change. Which bytes are leading is fixed: a source keeps the
    /// same checksum under this rule in every later release.
    FromStatementStart,
}
