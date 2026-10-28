use std::fmt;

/// One reason a declared online index does not match its bundled migration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OnlineIndexProblem {
    /// The migration is recorded as applied but the index does not exist,
    /// including when its live definition has disappeared while the inspection
    /// snapshot retains its old catalog row.
    Missing,
    /// The index exists but PostgreSQL marks it `INVALID`, typically after a
    /// failed or interrupted concurrent build.
    Invalid,
    /// A relation with the declared name exists that is not the declared
    /// index: a different definition, an index on another table, or another
    /// kind of relation.
    DefinitionMismatch,
    /// The bundled migration SQL does not contain the declared statement
    /// exactly once. This is a packaging defect, not a database state.
    StatementNotEmbedded,
    /// Another session in this database may still be building the index: its
    /// progress identifies the index, concurrent reindex holds its original
    /// index lock, hidden progress is correlated with a build lock on its table,
    /// or another session holds the migration lock while the index is missing
    /// or `INVALID`. Hidden identity conservatively
    /// defers other indexes on the same table. A definition removed since the
    /// inspection snapshot follows the same migration-lock rule. Retry after
    /// the build ends.
    BuildInProgress,
    /// The inspected INVALID index was replaced, removed or renamed away
    /// before repair locked it. No replacement is removed; inspect the
    /// database before trying again. A removed index is built by the next run.
    ChangedDuringRepair,
    /// A pending canonical statement lacks `IF NOT EXISTS`, but its matching
    /// index already exists. External tooling would fail, not adopt that index.
    ///
    /// ```rust
    /// use runledger_postgres::OnlineIndexProblem;
    /// let problem = OnlineIndexProblem::ExternalCannotAdopt;
    /// assert_ne!(problem, OnlineIndexProblem::BuildInProgress);
    /// // Review the existing object and migration rollout; do not drop it blindly.
    /// ```
    ExternalCannotAdopt,
}

impl fmt::Display for OnlineIndexProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "is missing although its migration is recorded as applied",
            Self::Invalid => {
                "is INVALID after an interrupted concurrent build; the protected migration path rebuilds it"
            }
            Self::DefinitionMismatch => {
                "names a relation that is not the bundled index definition on its declared table"
            }
            Self::StatementNotEmbedded => {
                "declares a statement the bundled migration SQL does not contain exactly once"
            }
            Self::ChangedDuringRepair => "changed identity before repair; no replacement was removed",
            Self::ExternalCannotAdopt => "already exists but the pending external statement cannot adopt it",
            Self::BuildInProgress => {
                "may have a build in progress in another session; retry after that build ends"
            }
        })
    }
}

/// Outcome of [`crate::prepare_online_indexes_for_external_migrator`].
#[derive(Debug, Default)]
#[must_use = "inspect deferred indexes and external blocking builds before running external tooling"]
pub struct OnlineIndexPreparation {
    pub(super) built: Vec<&'static str>,
    pub(super) already_valid: Vec<&'static str>,
    pub(super) deferred: Vec<&'static str>,
    pub(super) external_build_blocks: Vec<&'static str>,
}

impl OnlineIndexPreparation {
    /// Indexes this call built with `CREATE INDEX CONCURRENTLY`.
    #[must_use]
    pub fn built(&self) -> &[&'static str] {
        &self.built
    }

    /// Indexes that already existed, valid and with the bundled definition.
    #[must_use]
    pub fn already_valid(&self) -> &[&'static str] {
        &self.already_valid
    }

    /// Pending indexes not built by this call: an adoptable index whose
    /// migration cannot run yet because an earlier bundled migration is
    /// unapplied, or any index whose table does not exist. On a fresh database
    /// the pending migrations build these on empty tables. On a populated
    /// database, apply the migrations through the version preceding the
    /// adoptable index's own migration, then call
    /// [`crate::prepare_online_indexes_for_external_migrator`] again; otherwise
    /// the external migrator builds the index with a blocking statement.
    #[must_use]
    pub fn deferred(&self) -> &[&'static str] {
        &self.deferred
    }

    /// Pending indexes on an existing table whose canonical statement has no
    /// `IF NOT EXISTS`, and whose index name is absent at inspection, so an external migrator builds them with a blocking
    /// `CREATE INDEX`. Only [`crate::migrate_after_idempotency_cutover`] builds
    /// them online.
    #[must_use]
    pub fn external_build_blocks(&self) -> &[&'static str] {
        &self.external_build_blocks
    }
}
