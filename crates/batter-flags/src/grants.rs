//! The exact privileges each flags operation needs, as a grant fragment.
//!
//! The flags tables live in the fixed `flags` schema, so the object policy a
//! caller passes, which carries its row-type and PUBLIC choices like every
//! other fragment's, must name that schema. Every inventory entry is read off
//! the store's statements for this source version; a store query change is
//! expected to change the inventory.
//!
//! ```
//! use batter_flags::grants::{FlagsOperation, SCHEMA, grant_fragment};
//! use batter_sqlx::verification::{FragmentObjectPolicy, Identifier};
//!
//! let policy = FragmentObjectPolicy::new(Identifier::new(SCHEMA).unwrap());
//! let serving = grant_fragment(
//!     &policy,
//!     [FlagsOperation::SnapshotRead, FlagsOperation::Presence, FlagsOperation::PresencePrune],
//! )
//! .unwrap();
//! # let _ = serving;
//! ```

mod inventory;

use std::collections::BTreeSet;

use batter_sqlx::verification::{
    FragmentObjectPolicy, GrantFragment, ManifestError, ObjectPrivilege,
};

pub use inventory::FlagsOperation;

/// The schema every flags table lives in.
pub const SCHEMA: &str = "flags";

/// Why a fragment could not be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FlagsGrantError {
    /// No operation was selected.
    #[error("a flags grant selection must name at least one operation")]
    EmptySelection,
    /// The object policy names a schema other than `flags`.
    #[error("the flags grant object policy must name the `flags` schema")]
    WrongSchema,
    /// The declarations cannot be expressed in a grant manifest.
    #[error("flags grant declarations cannot be expressed in this manifest")]
    Declaration(#[from] ManifestError),
}

/// The privileges `operations` need, and no others. None grants INSERT,
/// UPDATE or DELETE on `flag_audit`: its trigger writes it as the table owner.
pub fn grant_fragment(
    policy: &FragmentObjectPolicy,
    operations: impl IntoIterator<Item = FlagsOperation>,
) -> Result<GrantFragment, FlagsGrantError> {
    if policy.schema().as_str() != SCHEMA {
        return Err(FlagsGrantError::WrongSchema);
    }
    let operations: BTreeSet<_> = operations
        .into_iter()
        .flat_map(FlagsOperation::expand)
        .collect();
    if operations.is_empty() {
        return Err(FlagsGrantError::EmptySelection);
    }
    let mut fragment = policy.declare_schema(GrantFragment::new())?;
    for operation in operations {
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
