//! The application-owned side of one composed exact role.
//!
//! Only this module decides database policy, discovery scope and defaults, role
//! ceilings and the ownership guard. The native fragments contribute object
//! declarations and nothing else, exactly as a consumer would compose them.

use super::support::Result;
use batter::runledger::grants::RunledgerOperation;
use batter::runlimit::grants::RunlimitOperation;
use batter::sqlx::verification::{
    ColumnGrantGroup, CompiledExactRole, DatabaseGrantSpec, DeclarationPurpose, DiscoveryDefaults,
    DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier, ObjectDefaults,
    ObjectPrivilege, PublicDelivery, QualifiedName, RelationGrantGroup, RolePolicy,
    RoutineGrantSpec, RoutineSignature, RoutineType, SchemaGrantSpec,
};

/// How the application's provisioned roles use PostgreSQL's PUBLIC role.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum PublicPolicy {
    /// PUBLIC delivers nothing on the composed objects. The suite revokes the
    /// default PUBLIC routine `EXECUTE` so this holds for application routines
    /// too.
    Deny,
    /// PUBLIC delivers selected privileges: `SELECT` on the composed relations
    /// and columns, and `EXECUTE` on the application routine.
    PermitSelectedDelivery,
}

/// One consumer-shaped composition request.
pub(crate) struct Composition<'a> {
    pub(crate) jobs: Option<(&'a str, &'a [RunledgerOperation])>,
    pub(crate) quotas: Option<(&'a str, &'a [RunlimitOperation])>,
    pub(crate) application: Option<&'a str>,
    pub(crate) public: PublicPolicy,
}

impl<'a> Composition<'a> {
    pub(crate) const fn new(public: PublicPolicy) -> Self {
        Self {
            jobs: None,
            quotas: None,
            application: None,
            public,
        }
    }

    pub(crate) const fn with_jobs(
        mut self,
        schema: &'a str,
        operations: &'a [RunledgerOperation],
    ) -> Self {
        self.jobs = Some((schema, operations));
        self
    }

    pub(crate) const fn with_quotas(
        mut self,
        schema: &'a str,
        operations: &'a [RunlimitOperation],
    ) -> Self {
        self.quotas = Some((schema, operations));
        self
    }

    pub(crate) const fn with_application(mut self, schema: &'a str) -> Self {
        self.application = Some(schema);
        self
    }

    /// Compile one exact role for exact-role verification and `GRANT` rendering.
    pub(crate) fn compile(&self) -> Result<CompiledExactRole> {
        let primary = self
            .application
            .or(self.jobs.map(|(schema, _)| schema))
            .or(self.quotas.map(|(schema, _)| schema))
            .ok_or_else(|| super::support::fail("a composition needs at least one schema"))?;
        let mut scope = Vec::new();
        for schema in [
            self.jobs.map(|(schema, _)| schema),
            self.quotas.map(|(schema, _)| schema),
            self.application,
        ]
        .into_iter()
        .flatten()
        {
            scope.push(Identifier::new(schema)?);
        }
        let mut manifest = ExactRoleManifest::new(
            Identifier::new(primary)?,
            DiscoveryScope::Schemas(scope.clone()),
        )?;
        manifest.set_discovery_defaults(self.defaults())?;
        manifest.set_role_policy(RolePolicy::default())?;
        manifest.deny_current_database_ownership(true);
        // The cluster retains PostgreSQL's default PUBLIC database privileges.
        // Declaring them keeps that an explicit application choice instead of an
        // unexplained excess-authority finding.
        manifest.add_database(
            DatabaseGrantSpec::new(
                [ObjectPrivilege::Connect],
                DeclarationPurpose::RequiredAndProvisioned,
            )?
            .public_delivery(PublicDelivery::AllowDeclared),
        )?;
        manifest.add_database(
            DatabaseGrantSpec::new(
                [ObjectPrivilege::Temporary],
                DeclarationPurpose::AllowedOnly,
            )?
            .public_delivery(PublicDelivery::AllowDeclared),
        )?;

        if let Some((schema, operations)) = self.jobs {
            let fragment = batter::runledger::grants::grant_fragment(
                &self.objects(schema)?,
                operations.iter().copied(),
            )?;
            manifest = manifest.with_fragment(fragment)?;
        }
        if let Some((schema, operations)) = self.quotas {
            let fragment = batter::runlimit::grants::grant_fragment(
                &self.objects(schema)?,
                operations.iter().copied(),
            )?;
            manifest = manifest.with_fragment(fragment)?;
        }
        if let Some(schema) = self.application {
            manifest = self.declare_application_objects(manifest, schema)?;
        }
        Ok(manifest.compile()?)
    }

    /// The explicit policy the application applies to native objects.
    fn objects(&self, schema: &str) -> Result<FragmentObjectPolicy> {
        let policy = FragmentObjectPolicy::new(Identifier::new(schema)?)
            // Both audited consumer shapes retain PostgreSQL's default PUBLIC
            // USAGE on table row types.
            .allow_row_type_public_usage(true);
        Ok(match self.public {
            PublicPolicy::Deny => policy,
            PublicPolicy::PermitSelectedDelivery => {
                policy.with_public_delivered_privileges([ObjectPrivilege::Select])?
            }
        })
    }

    fn defaults(&self) -> DiscoveryDefaults {
        DiscoveryDefaults {
            // Composite row types and the native enum types keep PostgreSQL's
            // default PUBLIC USAGE.
            types: ObjectDefaults {
                public_privileges: vec![batter::sqlx::verification::AllowedPrivilege::new(
                    ObjectPrivilege::Usage,
                    false,
                )],
                ..ObjectDefaults::default()
            },
            allow_row_type_public_usage: true,
            ..DiscoveryDefaults::default()
        }
    }

    /// The application's own relation, column and routine declarations. These
    /// are ordinary manifest inputs; no fragment contributes them.
    fn declare_application_objects(
        &self,
        mut manifest: ExactRoleManifest,
        schema: &str,
    ) -> Result<ExactRoleManifest> {
        let identifier = Identifier::new(schema)?;
        let deliveries = match self.public {
            PublicPolicy::Deny => (PublicDelivery::Deny, PublicDelivery::Deny),
            PublicPolicy::PermitSelectedDelivery => {
                (PublicDelivery::AllowDeclared, PublicDelivery::AllowDeclared)
            }
        };
        manifest.add_schema(SchemaGrantSpec::new(
            identifier.clone(),
            [ObjectPrivilege::Usage],
            DeclarationPurpose::RequiredAndProvisioned,
        )?)?;
        let deliveries_relation = deliveries.0;
        let ledger = QualifiedName::new(schema, APPLICATION_RELATION)?;
        manifest.add_relations(
            RelationGrantGroup::new(
                [ledger.clone()],
                [],
                DeclarationPurpose::RequiredAndProvisioned,
            )?
            .allow_row_type_public_usage(true),
        )?;
        manifest.add_columns(
            ColumnGrantGroup::new(
                ledger.clone(),
                [Identifier::new("id")?, Identifier::new("note")?],
                [ObjectPrivilege::Select],
                DeclarationPurpose::RequiredAndProvisioned,
            )?
            .public_delivery(deliveries_relation),
        )?;
        manifest.add_columns(ColumnGrantGroup::new(
            ledger,
            [Identifier::new("id")?, Identifier::new("note")?],
            [ObjectPrivilege::Insert],
            DeclarationPurpose::RequiredAndProvisioned,
        )?)?;
        manifest.add_routine(
            RoutineGrantSpec::new(
                RoutineSignature::new(
                    schema,
                    APPLICATION_ROUTINE,
                    [RoutineType::new("pg_catalog", "text")?],
                )?,
                [ObjectPrivilege::Execute],
                DeclarationPurpose::RequiredAndProvisioned,
            )?
            .public_delivery(deliveries.1),
        )?;
        Ok(manifest)
    }
}

/// The application relation the composed login reads and writes.
pub(crate) const APPLICATION_RELATION: &str = "delivery_notes";
/// The application routine the composed login calls.
pub(crate) const APPLICATION_ROUTINE: &str = "normalize_note";
