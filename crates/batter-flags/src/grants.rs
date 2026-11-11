//! The exact privileges each flags operation needs, as a grant fragment.
//!
//! The flags tables live in the fixed `flags` schema, so the object policy a
//! caller passes, which carries its row-type and PUBLIC choices like every
//! other fragment's, must name that schema. Every inventory entry is read off
//! the store's statements for this source version; a store query change is
//! expected to change the inventory.
//!
//! A login is granted a [`FlagsRole`], the exact operations one kind of login
//! runs, or a [`FlagsGrant`] of at least one operation.
//!
//! ```
//! use batter_flags::grants::{FlagsGrant, FlagsOperation, FlagsRole, SCHEMA, grant_fragment};
//! use batter_sqlx::verification::{FragmentObjectPolicy, Identifier};
//!
//! let policy = FragmentObjectPolicy::new(Identifier::new(SCHEMA).unwrap());
//! let serving = grant_fragment(&policy, &FlagsRole::ServingProcess.into()).unwrap();
//! let history = grant_fragment(&policy, &FlagsGrant::operations(FlagsOperation::HistoryRead, [])).unwrap();
//! # let _ = (serving, history);
//! ```

mod inventory;

use std::collections::BTreeSet;

use batter_sqlx::verification::{
    FragmentObjectPolicy, GrantFragment, ManifestError, ObjectPrivilege,
};

pub use inventory::FlagsOperation;

/// The schema every flags table lives in.
pub const SCHEMA: &str = "flags";

/// The exact operations one kind of login runs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FlagsRole {
    /// A process with a registered control plane: the refresh's read, the
    /// heartbeat and stop marker, and the prune, which
    /// `control_plane::register_in` always registers. A login missing one of
    /// them heartbeats or prunes with recoverable failures forever.
    ServingProcess,
    /// The `batter-flags` operator command: every write and read it runs.
    OperatorCli,
}

impl FlagsRole {
    /// The operations this role runs.
    pub const fn operations(self) -> &'static [FlagsOperation] {
        match self {
            Self::ServingProcess => &[
                FlagsOperation::SnapshotRead,
                FlagsOperation::Presence,
                FlagsOperation::PresencePrune,
            ],
            Self::OperatorCli => &[
                FlagsOperation::AuditedWrite,
                FlagsOperation::SnapshotRead,
                FlagsOperation::CatalogRead,
                FlagsOperation::HistoryRead,
                FlagsOperation::FleetRead,
            ],
        }
    }
}

/// A selection of at least one operation to grant one login.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlagsGrant {
    operations: BTreeSet<FlagsOperation>,
}

impl FlagsGrant {
    /// The operations `role` runs.
    pub fn role(role: FlagsRole) -> Self {
        let operations = role.operations().iter().copied().collect();
        Self { operations }
    }

    /// `first` and every operation of `rest`.
    pub fn operations(
        first: FlagsOperation,
        rest: impl IntoIterator<Item = FlagsOperation>,
    ) -> Self {
        let operations = std::iter::once(first).chain(rest).collect();
        Self { operations }
    }

    /// The operations of both grants, for a login that plays two roles.
    #[must_use]
    pub fn and(mut self, other: Self) -> Self {
        self.operations.extend(other.operations);
        self
    }
}

impl From<FlagsRole> for FlagsGrant {
    fn from(role: FlagsRole) -> Self {
        Self::role(role)
    }
}

/// Why a fragment could not be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FlagsGrantError {
    /// The object policy names a schema other than `flags`.
    #[error("the flags grant object policy must name the `flags` schema")]
    WrongSchema,
    /// The declarations cannot be expressed in a grant manifest.
    #[error("flags grant declarations cannot be expressed in this manifest")]
    Declaration(#[from] ManifestError),
}

/// The privileges `grant` needs, and no others. None grants INSERT, UPDATE or
/// DELETE on `flag_audit`: its trigger writes it as the table owner.
///
/// # Errors
///
/// [`FlagsGrantError::WrongSchema`] when `policy` names another schema, and
/// [`FlagsGrantError::Declaration`] when the manifest cannot express it.
pub fn grant_fragment(
    policy: &FragmentObjectPolicy,
    grant: &FlagsGrant,
) -> Result<GrantFragment, FlagsGrantError> {
    if policy.schema().as_str() != SCHEMA {
        return Err(FlagsGrantError::WrongSchema);
    }
    let mut fragment = policy.declare_schema(GrantFragment::new())?;
    for &operation in &grant.operations {
        for requirement in inventory::requirements(operation) {
            fragment = policy.declare_relations(
                fragment,
                [requirement.relation],
                requirement.relation_privileges.iter().copied(),
            )?;
            for (columns, privilege) in [
                (requirement.select, ObjectPrivilege::Select),
                (requirement.insert, ObjectPrivilege::Insert),
                (requirement.update, ObjectPrivilege::Update),
            ] {
                if columns.is_empty() {
                    continue;
                }
                fragment = policy.declare_columns(
                    fragment,
                    requirement.relation,
                    columns.iter().copied(),
                    [privilege],
                )?;
            }
        }
    }
    Ok(fragment)
}
