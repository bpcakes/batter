//! Exact-role grant requirements for this Runledger schema version.
//!
//! Runledger owns its SQL and migrations. This module reads the statements of
//! this source version and publishes the matching object, column and privilege
//! requirements as a [`GrantFragment`], so an application composes one
//! [`batter_sqlx::verification::ExactRoleManifest`] and uses the same compiled
//! value for exact-role verification and inert `GRANT` rendering.
//!
//! The requirements describe operations, not installed schema. A fragment is not
//! evidence that the migrations are applied, that a remote effect occurred, or
//! that the application's authorization policy is correct, and rendering grants
//! cannot remove privileges a role already holds.
//!
//! # Authority these requirements really carry
//!
//! * PostgreSQL requires `UPDATE` on at least one column for a locking `SELECT`,
//!   including `FOR KEY SHARE` and `FOR NO KEY UPDATE`. The `UPDATE(id)`
//!   declarations on `job_enqueue_intents` and `job_queue` exist only to take
//!   those locks, and nothing in PostgreSQL restricts them to locking: they are
//!   ordinary primary-key mutation authority usable by arbitrary SQL. An
//!   immutability trigger that prevents it is application-owned.
//! * `job_enqueue_intents.enqueue_request` holds the submitted request. A
//!   column-restricted fragment bounds which columns a login reads; it is not a
//!   payload-secrecy, row-level or tenant-isolation claim.
//! * Required `UPDATE` and `DELETE` privileges remain arbitrary SQL authority
//!   over those rows. [`RunledgerOperation::DirectJobExecution`] needs `DELETE`
//!   on `job_attempts` so an unstarted claim can be released, and no general
//!   history-protection claim follows from the narrower column lists.
//! * [`RunledgerOperation::SchemaSnapshot`] needs relation-level `SELECT`
//!   because `LOCK TABLE ... IN ACCESS SHARE MODE` checks relation privileges
//!   rather than column privileges. It is a separate selection, never a hidden
//!   prerequisite of intent submission.
//! * Catalog synchronization and scheduled dispatch take table locks stronger
//!   than `ACCESS SHARE` on `job_definitions` and `job_schedules`. PostgreSQL
//!   checks those against relation-level `UPDATE`, `DELETE`, `TRUNCATE` or
//!   `MAINTAIN`, never column privileges, so the requirement is relation-level
//!   `MAINTAIN`: it admits the lock without widening data mutation to every
//!   column. `MAINTAIN` is still real authority, including `VACUUM`, `ANALYZE`,
//!   `REINDEX` and `CLUSTER` on those relations.
//! * Job cancellation is outside the supported direct-job contract. The
//!   installed release trigger's cancellation branch marks a resource claim with
//!   `release_after` instead of deleting it, and that write is deliberately not
//!   required here.
//! * Installed trigger ownership and body safety stay application prerequisites.
//!   Native trigger functions need no serving `EXECUTE`, so none is declared.

mod inventory;

use batter_sqlx::verification::{
    FragmentObjectPolicy, GrantFragment, ManifestError, ObjectPrivilege,
};
use std::{collections::BTreeSet, error::Error, fmt};

pub use inventory::RunledgerOperation;

/// A native Runledger grant selection that cannot be turned into requirements.
#[derive(Debug)]
#[non_exhaustive]
pub enum RunledgerGrantError {
    /// The selection named no operation.
    EmptySelection,
    /// The resulting declarations are invalid or exceed the manifest bound.
    Declaration(ManifestError),
}

impl fmt::Display for RunledgerGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptySelection => {
                "a native Runledger grant selection must name at least one operation"
            }
            Self::Declaration(_) => {
                "native Runledger grant declarations cannot be expressed in this manifest"
            }
        })
    }
}

impl Error for RunledgerGrantError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EmptySelection => None,
            Self::Declaration(error) => Some(error),
        }
    }
}

impl From<ManifestError> for RunledgerGrantError {
    fn from(error: ManifestError) -> Self {
        Self::Declaration(error)
    }
}

/// Build the additive grant requirements for an explicit operation selection.
///
/// The returned fragment declares only objects. The application's
/// current-database declaration, discovery scope and defaults, role ceilings and
/// ownership guard stay on its own manifest, and the supplied
/// [`FragmentObjectPolicy`] carries its schema, PUBLIC-delivery, row-type and
/// ownership choices. Overlapping selections are normalized by the manifest
/// compiler, so a worker that also promotes intents renders one deterministic
/// grant plan.
///
/// ```
/// use batter_sqlx::verification::{
///     DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
/// };
/// use runledger_postgres::grants::{RunledgerOperation, grant_fragment};
///
/// let schema = Identifier::new("jobs")?;
/// let policy = FragmentObjectPolicy::new(schema.clone());
/// let fragment = grant_fragment(&policy, [RunledgerOperation::IntentSubmission])?;
/// let compiled = ExactRoleManifest::new(schema, DiscoveryScope::Declared)?
///     .with_fragment(fragment)?
///     .compile()?;
/// let rendered = compiled
///     .grant_plan()
///     .render(&Identifier::new("intent_writer")?, None)?;
/// assert!(rendered.contains(
///     "GRANT SELECT (\"id\"), UPDATE (\"id\") ON TABLE \"jobs\".\"job_enqueue_intents\"",
/// ));
/// assert!(!rendered.contains("job_queue"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
/// Returns [`RunledgerGrantError::EmptySelection`] for an empty selection, or
/// [`RunledgerGrantError::Declaration`] when the declarations cannot be
/// expressed within the manifest's declaration bound.
pub fn grant_fragment(
    policy: &FragmentObjectPolicy,
    operations: impl IntoIterator<Item = RunledgerOperation>,
) -> Result<GrantFragment, RunledgerGrantError> {
    let operations: BTreeSet<_> = operations.into_iter().collect();
    if operations.is_empty() {
        return Err(RunledgerGrantError::EmptySelection);
    }
    let mut fragment = policy.declare_schema(GrantFragment::new())?;
    for operation in operations {
        for requirement in inventory::requirements(operation) {
            fragment = declare(policy, fragment, requirement)?;
        }
    }
    Ok(fragment)
}

fn declare(
    policy: &FragmentObjectPolicy,
    fragment: GrantFragment,
    requirement: &inventory::Requirement,
) -> Result<GrantFragment, ManifestError> {
    let mut fragment = policy.declare_relations(
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
    Ok(fragment)
}
