use super::{
    CompiledExactRole, Identifier, MigrationExpectation, PolicyError, QualifiedName,
    policy::{MAX_MIGRATION_CHECKSUM_BYTES, MAX_MIGRATION_LEDGER_ROWS},
};
use std::collections::HashSet;

/// How a protected SQLx 0.9 migration ledger is compared with its manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlxLedgerMode {
    /// The ledger must exist and contain exactly the expected successful rows.
    Exact,
    /// The ledger may be absent; every installed row must belong to the
    /// expected set, but expected rows may remain uninstalled.
    InstalledSubset,
}

/// An exact SQLx 0.9 migration-ledger request.
///
/// This type retains only versions and checksums. Applications that start from
/// native SQLx migrations must include `Simple` and `ReversibleUp` migrations,
/// exclude `ReversibleDown`, and pass the resulting metadata here. Batter does
/// not run a migrator or retain migration SQL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlxMigrationManifest {
    ledger: QualifiedName,
    mode: SqlxLedgerMode,
    expected: Vec<MigrationExpectation>,
}

impl SqlxMigrationManifest {
    /// Construct a bounded SQLx migration manifest.
    pub fn new(
        ledger: QualifiedName,
        mode: SqlxLedgerMode,
        expected: impl IntoIterator<Item = MigrationExpectation>,
    ) -> Result<Self, PolicyError> {
        let expected = collect_bounded(expected, MAX_MIGRATION_LEDGER_ROWS)?;
        let manifest = Self {
            ledger,
            mode,
            expected,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Return the qualified SQLx ledger relation.
    pub fn ledger(&self) -> &QualifiedName {
        &self.ledger
    }

    /// Return the requested comparison mode.
    pub const fn mode(&self) -> SqlxLedgerMode {
        self.mode
    }

    /// Return the expected version/checksum metadata.
    pub fn expected(&self) -> &[MigrationExpectation] {
        &self.expected
    }

    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        if self.expected.len() > MAX_MIGRATION_LEDGER_ROWS {
            return Err(PolicyError::AuthorityCapacity);
        }
        let mut versions = HashSet::new();
        for migration in &self.expected {
            if migration.checksum.len() > MAX_MIGRATION_CHECKSUM_BYTES {
                return Err(PolicyError::MigrationChecksumTooLarge);
            }
            if !versions.insert(migration.version) {
                return Err(PolicyError::DuplicateMigrationVersion);
            }
        }
        Ok(())
    }
}

/// Exact stored `search_path` policy for every SECURITY DEFINER routine in a
/// bounded set of schemas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaInspectionPolicy {
    schemas: Vec<Identifier>,
    expected_search_path: String,
}

impl SchemaInspectionPolicy {
    /// Construct an explicit schema scope and exact stored `search_path` value.
    ///
    /// PostgreSQL stores function-local settings as `name=value` strings. This
    /// checker deliberately compares that representation exactly rather than
    /// accepting equivalent SQL spellings.
    pub fn new(
        schemas: impl IntoIterator<Item = Identifier>,
        expected_search_path: impl Into<String>,
    ) -> Result<Self, PolicyError> {
        let schemas = collect_bounded(schemas, MAX_MIGRATION_LEDGER_ROWS)?;
        let expected_search_path = expected_search_path.into();
        let mut policy = Self {
            schemas,
            expected_search_path,
        };
        policy
            .schemas
            .sort_by(|left, right| left.as_str().cmp(right.as_str()));
        policy.schemas.dedup();
        policy.validate()?;
        Ok(policy)
    }

    /// Construct the canonical SQLx/application schema setting used by the
    /// protected path: `search_path=pg_catalog, pg_temp`.
    pub fn canonical(schemas: impl IntoIterator<Item = Identifier>) -> Result<Self, PolicyError> {
        Self::new(schemas, "search_path=pg_catalog, pg_temp")
    }

    /// Return the explicitly selected schemas.
    pub fn schemas(&self) -> &[Identifier] {
        &self.schemas
    }

    /// Return the exact stored setting required on each definer routine.
    pub fn expected_search_path(&self) -> &str {
        &self.expected_search_path
    }

    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        if self.schemas.is_empty() {
            return Err(PolicyError::EmptySchemaInspection);
        }
        if self.schemas.len() > MAX_MIGRATION_LEDGER_ROWS {
            return Err(PolicyError::AuthorityCapacity);
        }
        if self.expected_search_path.len() > MAX_MIGRATION_CHECKSUM_BYTES
            || self.expected_search_path.contains('\0')
            || !self.expected_search_path.starts_with("search_path=")
            || self.expected_search_path.len() == "search_path=".len()
        {
            return Err(PolicyError::InvalidSearchPathSetting);
        }
        Ok(())
    }

    pub(crate) fn requests_temporary_namespace(&self) -> bool {
        self.schemas
            .iter()
            .any(|schema| super::policy::coverage::temporary_namespace(schema.as_str()))
    }
}

fn collect_bounded<T>(
    values: impl IntoIterator<Item = T>,
    limit: usize,
) -> Result<Vec<T>, PolicyError> {
    let mut collected = Vec::new();
    for value in values {
        if collected.len() == limit {
            return Err(PolicyError::AuthorityCapacity);
        }
        collected.push(value);
    }
    Ok(collected)
}

/// Borrowed components for one protected verification snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct VerificationRequest<'a> {
    pub(crate) exact_role: Option<&'a CompiledExactRole>,
    pub(crate) sqlx_migrations: Option<&'a SqlxMigrationManifest>,
    pub(crate) schema: Option<&'a SchemaInspectionPolicy>,
}

impl<'a> VerificationRequest<'a> {
    /// Start an empty request and add independent components with the builder
    /// methods below. An empty request is invalid when executed.
    pub const fn new() -> Self {
        Self {
            exact_role: None,
            sqlx_migrations: None,
            schema: None,
        }
    }

    /// Add a compiled exact-role authority request and its safeguards.
    pub const fn with_exact_role(mut self, role: &'a CompiledExactRole) -> Self {
        self.exact_role = Some(role);
        self
    }

    /// Add SQLx 0.9 ledger shape and history verification.
    pub const fn with_sqlx_migrations(mut self, manifest: &'a SqlxMigrationManifest) -> Self {
        self.sqlx_migrations = Some(manifest);
        self
    }

    /// Add schema-only SECURITY DEFINER configuration verification.
    pub const fn with_schema(mut self, policy: &'a SchemaInspectionPolicy) -> Self {
        self.schema = Some(policy);
        self
    }

    pub(crate) fn validate(self) -> Result<(), PolicyError> {
        if self.exact_role.is_none() && self.sqlx_migrations.is_none() && self.schema.is_none() {
            return Err(PolicyError::EmptyVerificationRequest);
        }
        if let Some(role) = self.exact_role {
            role.authority_policy().validate()?;
        }
        if let Some(manifest) = self.sqlx_migrations {
            manifest.validate()?;
        }
        if let Some(policy) = self.schema {
            policy.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlx_manifest_rejects_duplicates_and_oversized_checksums() {
        let ledger = QualifiedName::new("public", "_sqlx_migrations").unwrap();
        assert_eq!(
            SqlxMigrationManifest::new(
                ledger.clone(),
                SqlxLedgerMode::Exact,
                [
                    MigrationExpectation::new(1, [1]),
                    MigrationExpectation::new(1, [2]),
                ],
            ),
            Err(PolicyError::DuplicateMigrationVersion)
        );
        assert_eq!(
            SqlxMigrationManifest::new(
                ledger,
                SqlxLedgerMode::Exact,
                [MigrationExpectation::new(
                    1,
                    vec![0; MAX_MIGRATION_CHECKSUM_BYTES + 1],
                )],
            ),
            Err(PolicyError::MigrationChecksumTooLarge)
        );
    }

    #[test]
    fn schema_policy_is_normalized_and_exact() {
        let policy = SchemaInspectionPolicy::canonical([
            Identifier::new("service").unwrap(),
            Identifier::new("audit").unwrap(),
            Identifier::new("service").unwrap(),
        ])
        .unwrap();
        assert_eq!(
            policy
                .schemas()
                .iter()
                .map(Identifier::as_str)
                .collect::<Vec<_>>(),
            ["audit", "service"]
        );
        assert_eq!(
            policy.expected_search_path(),
            "search_path=pg_catalog, pg_temp"
        );
        assert_eq!(
            SchemaInspectionPolicy::new([Identifier::new("service").unwrap()], "work_mem=1MB",),
            Err(PolicyError::InvalidSearchPathSetting)
        );
        assert_eq!(
            SchemaInspectionPolicy::canonical(Vec::<Identifier>::new()),
            Err(PolicyError::EmptySchemaInspection)
        );
    }

    #[test]
    fn constructors_stop_consuming_inputs_at_the_capacity_boundary() {
        let ledger = QualifiedName::new("public", "_sqlx_migrations").unwrap();
        let mut produced = 0usize;
        let expectations = std::iter::from_fn(|| {
            produced += 1;
            assert!(produced <= MAX_MIGRATION_LEDGER_ROWS + 1);
            Some(MigrationExpectation::new(
                i64::try_from(produced).unwrap(),
                [0],
            ))
        });
        assert_eq!(
            SqlxMigrationManifest::new(ledger, SqlxLedgerMode::Exact, expectations),
            Err(PolicyError::AuthorityCapacity)
        );
        assert_eq!(produced, MAX_MIGRATION_LEDGER_ROWS + 1);

        let schema = Identifier::new("service").unwrap();
        assert_eq!(
            SchemaInspectionPolicy::canonical(std::iter::repeat_n(
                schema,
                MAX_MIGRATION_LEDGER_ROWS + 1
            )),
            Err(PolicyError::AuthorityCapacity)
        );
    }
}
