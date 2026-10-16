//! Pure manifest validation failures.

use std::fmt;

/// Invalid managed object input, rejected before any connection work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedObjectError {
    /// The source does not start with the declared kind's header:
    /// `CREATE OR REPLACE FUNCTION`, `PROCEDURE` or `TRIGGER`, or
    /// `CREATE CONSTRAINT TRIGGER` without `OR REPLACE`, followed by a
    /// recognizable trigger name, timing and events where the kind has them.
    /// A constraint trigger declared as
    /// [`ManagedObjectKind::Trigger`](super::ManagedObjectKind::Trigger), or a
    /// plain trigger declared as
    /// [`ManagedObjectKind::ConstraintTrigger`](super::ManagedObjectKind::ConstraintTrigger),
    /// is rejected here.
    SourceHeader,
    /// The header names another object than declared, or does not
    /// schema-qualify the routine name or trigger table.
    SourceIdentity,
    /// The source has an unterminated quote, comment or dollar quote, contains
    /// a NUL byte, or continues after its one statement.
    SourceStatement,
    /// One source exceeds 1 MiB.
    SourceTooLarge,
    /// An entry, retirement or the ledger lives in a session-local temporary
    /// namespace (`pg_temp`, `pg_temp_*` or `pg_toast_temp_*`), including a
    /// routine argument type.
    TemporaryNamespace,
    /// The ledger uses SQLx's reserved `_sqlx_migrations` relation name.
    ReservedLedger,
    /// Two entries share one identity. Functions and procedures share
    /// PostgreSQL's routine identities; triggers and constraint triggers share
    /// its per-table trigger names.
    DuplicateObject {
        /// The canonical identity declared twice.
        identity: String,
    },
    /// Two retirements share one identity.
    DuplicateRetirement {
        /// The canonical identity retired twice.
        identity: String,
    },
    /// A retirement names an identity the same manifest declares.
    RetiredObjectDeclared {
        /// The canonical identity both declared and retired.
        identity: String,
    },
    /// More than 10,000 entries plus retirements, or more than 64 MiB of source.
    Capacity,
    /// The manifest declares no object and no retirement.
    EmptyManifest,
}

impl fmt::Display for ManagedObjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceHeader => {
                formatter.write_str("managed object source does not start with its kind's header")
            }
            Self::SourceIdentity => formatter.write_str(
                "managed object source names another or unqualified object than declared",
            ),
            Self::SourceStatement => {
                formatter.write_str("managed object source is not exactly one complete statement")
            }
            Self::SourceTooLarge => formatter.write_str("managed object source exceeds 1 MiB"),
            Self::TemporaryNamespace => {
                formatter.write_str("managed object names a temporary namespace")
            }
            Self::ReservedLedger => {
                formatter.write_str("managed object ledger uses the SQLx migration ledger name")
            }
            Self::DuplicateObject { identity } => {
                write!(formatter, "managed object is declared twice: {identity}")
            }
            Self::DuplicateRetirement { identity } => {
                write!(formatter, "managed object is retired twice: {identity}")
            }
            Self::RetiredObjectDeclared { identity } => {
                write!(
                    formatter,
                    "managed object is both declared and retired: {identity}"
                )
            }
            Self::Capacity => formatter.write_str("managed object manifest capacity exceeded"),
            Self::EmptyManifest => {
                formatter.write_str("managed object manifest declares no object or retirement")
            }
        }
    }
}

impl std::error::Error for ManagedObjectError {}
