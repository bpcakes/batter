//! Exact-role grant requirements for native Runlimit PostgreSQL storage.
//!
//! Native `runlimit-postgres` owns its SQL and migrations and depends on no
//! Batter package. This module reads that source version's statements and
//! publishes the matching object, column and privilege requirements as a
//! [`GrantFragment`], so an application composes one
//! [`batter_sqlx::verification::ExactRoleManifest`] instead of hand-copying
//! native privilege lists.
//!
//! A fragment describes the operations this source version performs. It is not
//! proof that the schema is installed, that remote effects occurred, or that an
//! application's authorization policy is correct. Rendering grants cannot remove
//! privileges a role already holds.
//!
//! # Authority these requirements really carry
//!
//! * Row-locking statements need `UPDATE` on at least one column in addition to
//!   `SELECT`. `UPDATE(capacity_shard)` on the fixed-window capacity ledger and
//!   `UPDATE(window_expires_at)` on the cleanup path exist only to take those
//!   locks, and PostgreSQL does not restrict them to locking: they are ordinary
//!   mutation authority usable by arbitrary SQL.
//! * Fixed-window `policy_id` and `scope_id` updates are intentional metadata
//!   refreshes performed by the admission upsert.
//! * Capacity `row_count` is readable by an admitting login, which compares it
//!   against the configured per-shard ceiling, but it receives no `UPDATE`
//!   authority on it: the native `SECURITY DEFINER` capacity triggers are the
//!   only writers. The counter-key columns `config_fingerprint` and
//!   `subject_key` are never updatable.
//! * `DELETE` on an expiry-cleanup store is relation-wide authority over those
//!   rows. No history-protection, tenant-isolation or secrecy claim follows.
//! * Fixed-window expiry cleanup additionally needs relation-level `SELECT`,
//!   because its bounded delete identifies victims by the `ctid` system column
//!   and PostgreSQL checks system-column reads against relation privileges. A
//!   login that selects that cleanup therefore reads every counter column,
//!   including the retained `policy_id` and `scope_id` metadata.

mod inventory;

use batter_sqlx::verification::{
    FragmentObjectPolicy, GrantFragment, ManifestError, ObjectPrivilege,
};
use std::collections::BTreeSet;

pub use inventory::RunlimitOperation;

/// A native Runlimit grant selection that cannot be turned into requirements.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunlimitGrantError {
    /// The selection named no operation.
    #[error("a native Runlimit grant selection must name at least one operation")]
    EmptySelection,
    /// The resulting declarations are invalid or exceed the manifest bound.
    #[error("native Runlimit grant declarations cannot be expressed in this manifest")]
    Declaration(#[from] ManifestError),
}

/// Build the additive grant requirements for an explicit operation selection.
///
/// The returned fragment declares only objects: the application's database,
/// discovery, role and ownership policy stays on its own manifest, and the
/// supplied [`FragmentObjectPolicy`] carries its schema, PUBLIC-delivery,
/// row-type and ownership choices. Overlapping selections are normalized by the
/// manifest compiler, so composing an admission and a cleanup selection renders
/// one deterministic grant plan.
///
/// ```
/// use batter_runlimit::grants::{RunlimitOperation, grant_fragment};
/// use batter_sqlx::verification::{
///     DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
/// };
///
/// let schema = Identifier::new("quotas")?;
/// let policy = FragmentObjectPolicy::new(schema.clone()).allow_row_type_public_usage(true);
/// let fragment = grant_fragment(&policy, [RunlimitOperation::GcraAdmission])?;
/// let compiled = ExactRoleManifest::new(schema, DiscoveryScope::Declared)?
///     .with_fragment(fragment)?
///     .compile()?;
/// assert!(compiled.grant_plan().render(&Identifier::new("quota_service")?, None)?.contains(
///     "GRANT SELECT (\"observed_at_ms\"), UPDATE (\"observed_at_ms\") \
///      ON TABLE \"quotas\".\"runlimit_gcra_shards\" TO \"quota_service\";",
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
/// Returns [`RunlimitGrantError::EmptySelection`] for an empty selection, or
/// [`RunlimitGrantError::Declaration`] when the declarations cannot be expressed
/// within the manifest's declaration bound.
pub fn grant_fragment(
    policy: &FragmentObjectPolicy,
    operations: impl IntoIterator<Item = RunlimitOperation>,
) -> Result<GrantFragment, RunlimitGrantError> {
    let operations: BTreeSet<_> = operations.into_iter().collect();
    if operations.is_empty() {
        return Err(RunlimitGrantError::EmptySelection);
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
