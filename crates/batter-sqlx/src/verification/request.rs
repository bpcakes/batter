use super::{
    AuthorityPolicy, CompiledExactRole, Identifier, MigrationExpectation, MigrationPolicy,
    PolicyError, QualifiedName,
    policy::{MAX_MIGRATION_CHECKSUM_BYTES, MAX_MIGRATION_LEDGER_ROWS},
};
use std::collections::HashSet;
use std::fmt;

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

#[derive(Clone, Copy, Debug)]
pub(crate) enum AuthorityInspection<'a> {
    Policy(&'a AuthorityPolicy),
    ExactRole(&'a CompiledExactRole),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MigrationInspection<'a> {
    Generic(&'a MigrationPolicy),
    Sqlx(&'a SqlxMigrationManifest),
}

impl<'a> AuthorityInspection<'a> {
    fn policy(self) -> &'a AuthorityPolicy {
        match self {
            Self::Policy(policy) => policy,
            Self::ExactRole(role) => role.authority_policy(),
        }
    }
}

impl<'a> MigrationInspection<'a> {
    fn ledger(self) -> &'a QualifiedName {
        match self {
            Self::Generic(policy) => policy.ledger(),
            Self::Sqlx(manifest) => manifest.ledger(),
        }
    }
}

/// An invalid semantic composition in a [`VerificationPlan`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// A generic authority policy or compiled exact role was already selected.
    AuthorityAlreadySelected,
    /// A generic or SQLx-specific migration policy was already selected.
    MigrationAlreadySelected,
    /// A schema-inspection policy was already selected.
    SchemaInspectionAlreadySelected,
    /// Authority requires the migration ledger's schema-qualified identity to
    /// be a sequence, while migration verification requires a relation.
    ConflictingRelationKind,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AuthorityAlreadySelected => "verification authority is already selected",
            Self::MigrationAlreadySelected => "verification migration ledger is already selected",
            Self::SchemaInspectionAlreadySelected => {
                "verification schema inspection is already selected"
            }
            Self::ConflictingRelationKind => {
                "migration ledger identity conflicts with authority object kind"
            }
        })
    }
}

impl std::error::Error for PlanError {}

/// A non-empty selection of already validated verification components.
///
/// Every constructor selects the first component, so an empty plan cannot be
/// represented. Composition rejects a second authority, migration, or schema
/// component instead of silently replacing the earlier selection. The three
/// semantic axes are independently optional: either authority form may be
/// combined with either migration form and with schema inspection in one
/// snapshot. Composition rejects an authority policy that declares the
/// migration ledger's identity as a sequence because PostgreSQL relations and
/// sequences share one schema namespace.
///
/// ```compile_fail
/// use batter_sqlx::verification::VerificationPlan;
///
/// // There is deliberately no empty or default verification plan.
/// let _: VerificationPlan<'static> = Default::default();
/// ```
#[derive(Clone, Copy, Debug)]
pub struct VerificationPlan<'a> {
    pub(crate) authority: Option<AuthorityInspection<'a>>,
    pub(crate) migration: Option<MigrationInspection<'a>>,
    pub(crate) schema: Option<&'a SchemaInspectionPolicy>,
}

impl<'a> VerificationPlan<'a> {
    /// Verify one compiled generic authority policy.
    pub const fn authority(policy: &'a AuthorityPolicy) -> Self {
        Self {
            authority: Some(AuthorityInspection::Policy(policy)),
            migration: None,
            schema: None,
        }
    }

    /// Verify one compiled generic migration policy.
    pub const fn migrations(policy: &'a MigrationPolicy) -> Self {
        Self {
            authority: None,
            migration: Some(MigrationInspection::Generic(policy)),
            schema: None,
        }
    }

    /// Verify one compiled exact role, retaining all of its safeguards.
    pub const fn exact_role(role: &'a CompiledExactRole) -> Self {
        Self {
            authority: Some(AuthorityInspection::ExactRole(role)),
            migration: None,
            schema: None,
        }
    }

    /// Verify one validated SQLx 0.9 migration manifest.
    pub const fn sqlx_migrations(manifest: &'a SqlxMigrationManifest) -> Self {
        Self {
            authority: None,
            migration: Some(MigrationInspection::Sqlx(manifest)),
            schema: None,
        }
    }

    /// Verify SECURITY DEFINER configuration in an explicit schema scope.
    pub const fn schema_inspection(policy: &'a SchemaInspectionPolicy) -> Self {
        Self {
            authority: None,
            migration: None,
            schema: Some(policy),
        }
    }

    /// Add a compiled generic authority policy.
    pub fn with_authority(mut self, policy: &'a AuthorityPolicy) -> Result<Self, PlanError> {
        self.select_authority(AuthorityInspection::Policy(policy))?;
        Ok(self)
    }

    /// Add a compiled exact role and all of its safeguards.
    pub fn with_exact_role(mut self, role: &'a CompiledExactRole) -> Result<Self, PlanError> {
        self.select_authority(AuthorityInspection::ExactRole(role))?;
        Ok(self)
    }

    /// Add a compiled generic migration policy.
    pub fn with_migrations(mut self, policy: &'a MigrationPolicy) -> Result<Self, PlanError> {
        self.select_migration(MigrationInspection::Generic(policy))?;
        Ok(self)
    }

    /// Add a validated SQLx 0.9 migration manifest.
    pub fn with_sqlx_migrations(
        mut self,
        manifest: &'a SqlxMigrationManifest,
    ) -> Result<Self, PlanError> {
        self.select_migration(MigrationInspection::Sqlx(manifest))?;
        Ok(self)
    }

    /// Add schema-only SECURITY DEFINER configuration verification.
    pub fn with_schema_inspection(
        mut self,
        policy: &'a SchemaInspectionPolicy,
    ) -> Result<Self, PlanError> {
        if self.schema.is_some() {
            return Err(PlanError::SchemaInspectionAlreadySelected);
        }
        self.schema = Some(policy);
        Ok(self)
    }

    fn select_authority(&mut self, authority: AuthorityInspection<'a>) -> Result<(), PlanError> {
        if self.authority.is_some() {
            return Err(PlanError::AuthorityAlreadySelected);
        }
        if self.migration.is_some_and(|migration| {
            authority
                .policy()
                .requires_sequence_identity(migration.ledger())
        }) {
            return Err(PlanError::ConflictingRelationKind);
        }
        self.authority = Some(authority);
        Ok(())
    }

    fn select_migration(&mut self, migration: MigrationInspection<'a>) -> Result<(), PlanError> {
        if self.migration.is_some() {
            return Err(PlanError::MigrationAlreadySelected);
        }
        if self.authority.is_some_and(|authority| {
            authority
                .policy()
                .requires_sequence_identity(migration.ledger())
        }) {
            return Err(PlanError::ConflictingRelationKind);
        }
        self.migration = Some(migration);
        Ok(())
    }

    pub(crate) fn authority_policy(self) -> Option<&'a AuthorityPolicy> {
        match self.authority {
            Some(AuthorityInspection::Policy(policy)) => Some(policy),
            Some(AuthorityInspection::ExactRole(role)) => Some(role.authority_policy()),
            None => None,
        }
    }

    pub(crate) fn selected_exact_role(self) -> Option<&'a CompiledExactRole> {
        match self.authority {
            Some(AuthorityInspection::ExactRole(role)) => Some(role),
            Some(AuthorityInspection::Policy(_)) | None => None,
        }
    }

    pub(crate) fn migration_policy(self) -> Option<&'a MigrationPolicy> {
        match self.migration {
            Some(MigrationInspection::Generic(policy)) => Some(policy),
            Some(MigrationInspection::Sqlx(_)) | None => None,
        }
    }

    pub(crate) fn sqlx_migration(self) -> Option<&'a SqlxMigrationManifest> {
        match self.migration {
            Some(MigrationInspection::Sqlx(manifest)) => Some(manifest),
            Some(MigrationInspection::Generic(_)) | None => None,
        }
    }

    pub(crate) const fn schema(self) -> Option<&'a SchemaInspectionPolicy> {
        self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::{
        AllowedPrivilege, AuthorityPolicyBuilder, DiscoveryDefaults, DiscoveryScope,
        ObjectDefaults, ObjectPrivilege, PublicAllowance, PublicGrant, PublicObject,
        RelationPolicy, RequiredPrivilege, SequencePolicy,
    };

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
    fn plan_rejects_duplicate_semantic_components() {
        let authority = AuthorityPolicy::default();
        let migration =
            MigrationPolicy::new(QualifiedName::new("service", "ledger").unwrap(), []).unwrap();
        let sqlx = SqlxMigrationManifest::new(
            QualifiedName::new("service", "_sqlx_migrations").unwrap(),
            SqlxLedgerMode::Exact,
            [],
        )
        .unwrap();
        let schema =
            SchemaInspectionPolicy::canonical([Identifier::new("service").unwrap()]).unwrap();
        let role = crate::verification::ExactRoleManifest::new(
            Identifier::new("service").unwrap(),
            crate::verification::DiscoveryScope::Declared,
        )
        .unwrap()
        .compile()
        .unwrap();

        assert!(matches!(
            VerificationPlan::authority(&authority).with_authority(&authority),
            Err(PlanError::AuthorityAlreadySelected)
        ));
        assert!(matches!(
            VerificationPlan::authority(&authority).with_exact_role(&role),
            Err(PlanError::AuthorityAlreadySelected)
        ));
        assert!(matches!(
            VerificationPlan::exact_role(&role).with_authority(&authority),
            Err(PlanError::AuthorityAlreadySelected)
        ));
        assert!(matches!(
            VerificationPlan::migrations(&migration).with_migrations(&migration),
            Err(PlanError::MigrationAlreadySelected)
        ));
        assert!(matches!(
            VerificationPlan::migrations(&migration).with_sqlx_migrations(&sqlx),
            Err(PlanError::MigrationAlreadySelected)
        ));
        assert!(matches!(
            VerificationPlan::sqlx_migrations(&sqlx).with_migrations(&migration),
            Err(PlanError::MigrationAlreadySelected)
        ));
        assert!(matches!(
            VerificationPlan::schema_inspection(&schema).with_schema_inspection(&schema),
            Err(PlanError::SchemaInspectionAlreadySelected)
        ));

        assert_supported_plan_combinations(&authority, &migration, &sqlx, &schema, &role);
    }

    #[test]
    fn plan_rejects_a_migration_ledger_declared_as_a_sequence() {
        let ledger = QualifiedName::new("service", "ledger").unwrap();
        let generic = MigrationPolicy::new(ledger.clone(), []).unwrap();
        let sqlx = SqlxMigrationManifest::new(ledger.clone(), SqlxLedgerMode::Exact, []).unwrap();
        let usage = AllowedPrivilege::new(ObjectPrivilege::Usage, false);
        let policies = [
            AuthorityPolicyBuilder {
                sequences: vec![SequencePolicy {
                    sequence: ledger.clone(),
                    privileges: Vec::new(),
                    allow_owner: false,
                }],
                ..AuthorityPolicyBuilder::default()
            },
            AuthorityPolicyBuilder {
                public_grants: vec![PublicGrant {
                    object: PublicObject::Sequence(ledger.clone()),
                    privilege: usage,
                }],
                ..AuthorityPolicyBuilder::default()
            },
            AuthorityPolicyBuilder {
                public_overrides: vec![PublicAllowance {
                    object: PublicObject::Sequence(ledger.clone()),
                    privileges: Vec::new(),
                }],
                ..AuthorityPolicyBuilder::default()
            },
            AuthorityPolicyBuilder {
                discovery: DiscoveryScope::UserSchemas,
                defaults: DiscoveryDefaults {
                    sequences: ObjectDefaults {
                        privileges: vec![usage],
                        ..ObjectDefaults::default()
                    },
                    ..DiscoveryDefaults::default()
                },
                required_privileges: vec![RequiredPrivilege {
                    object: PublicObject::Sequence(ledger.clone()),
                    privilege: ObjectPrivilege::Usage,
                }],
                ..AuthorityPolicyBuilder::default()
            },
        ]
        .map(|policy| policy.build().unwrap());

        for authority in &policies {
            assert!(matches!(
                VerificationPlan::authority(authority).with_migrations(&generic),
                Err(PlanError::ConflictingRelationKind)
            ));
            assert!(matches!(
                VerificationPlan::migrations(&generic).with_authority(authority),
                Err(PlanError::ConflictingRelationKind)
            ));
        }
        assert!(matches!(
            VerificationPlan::authority(&policies[0]).with_sqlx_migrations(&sqlx),
            Err(PlanError::ConflictingRelationKind)
        ));
        assert!(matches!(
            VerificationPlan::sqlx_migrations(&sqlx).with_authority(&policies[0]),
            Err(PlanError::ConflictingRelationKind)
        ));

        let relation = AuthorityPolicyBuilder {
            relations: vec![RelationPolicy {
                relation: ledger,
                privileges: Vec::new(),
                columns: Vec::new(),
                allow_owner: false,
                allow_row_type_public_usage: false,
            }],
            ..AuthorityPolicyBuilder::default()
        }
        .build()
        .unwrap();
        assert!(
            VerificationPlan::authority(&relation)
                .with_migrations(&generic)
                .is_ok()
        );
    }

    fn assert_supported_plan_combinations<'a>(
        authority: &'a AuthorityPolicy,
        migration: &'a MigrationPolicy,
        sqlx: &'a SqlxMigrationManifest,
        schema: &'a SchemaInspectionPolicy,
        role: &'a crate::verification::CompiledExactRole,
    ) {
        let exact_generic = VerificationPlan::exact_role(role)
            .with_migrations(migration)
            .unwrap()
            .with_schema_inspection(schema)
            .unwrap();
        assert!(exact_generic.selected_exact_role().is_some());
        assert!(exact_generic.migration_policy().is_some());
        assert!(exact_generic.schema().is_some());

        let generic_sqlx = VerificationPlan::authority(authority)
            .with_sqlx_migrations(sqlx)
            .unwrap()
            .with_schema_inspection(schema)
            .unwrap();
        assert!(generic_sqlx.authority_policy().is_some());
        assert!(generic_sqlx.sqlx_migration().is_some());
        assert!(generic_sqlx.schema().is_some());
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

        let mut generic_produced = 0usize;
        let generic_expectations = std::iter::from_fn(|| {
            generic_produced += 1;
            assert!(generic_produced <= MAX_MIGRATION_LEDGER_ROWS + 1);
            Some(MigrationExpectation::new(generic_produced as i64, [1]))
        });
        assert_eq!(
            MigrationPolicy::new(
                QualifiedName::new("public", "ledger").unwrap(),
                generic_expectations,
            ),
            Err(PolicyError::AuthorityCapacity)
        );
        assert_eq!(generic_produced, MAX_MIGRATION_LEDGER_ROWS + 1);

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
