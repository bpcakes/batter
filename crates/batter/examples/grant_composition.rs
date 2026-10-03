//! Compose one exact role from the native Runledger and Runlimit grant
//! requirements plus an application's own objects, using only a `batter`
//! dependency.
//!
//! # Required setup
//!
//! The rendered statements are inert text. A migration or DBA role applies them
//! to an already provisioned schema, and the role must already exist:
//!
//! ```text
//! cargo run --example grant_composition --features runledger,runlimit-postgres
//! ```
//!
//! # Limitations
//!
//! * Rendering grants cannot remove privileges a role already holds. Adopting
//!   these requirements means reviewing and reconciling the provisioned role,
//!   including any privilege it reaches through PUBLIC.
//! * The requirements describe this source version's operations. They are not
//!   evidence that the native schema is installed, that a remote effect
//!   occurred, or that the application's authorization policy is correct.
//! * `MAINTAIN` on the catalog relations admits the table locks catalog
//!   synchronization and scheduled dispatch take. It also permits `VACUUM`,
//!   `ANALYZE`, `REINDEX` and `CLUSTER` on them.
//! * Row-locking `UPDATE` privileges, including `UPDATE(id)` and
//!   `UPDATE(capacity_shard)`, are ordinary mutation authority usable by
//!   arbitrary SQL.

use batter::runledger::grants::RunledgerOperation;
use batter::runlimit::grants::RunlimitOperation;
use batter::sqlx::verification::{
    ColumnGrantGroup, CompiledExactRole, DatabaseGrantSpec, DeclarationPurpose, DiscoveryDefaults,
    DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier, ObjectDefaults,
    ObjectPrivilege, PublicDelivery, QualifiedName, RelationGrantGroup, RolePolicy,
    SchemaGrantSpec,
};
use std::process::ExitCode;

/// Whether the application's provisioned roles use PostgreSQL's PUBLIC role.
#[derive(Clone, Copy)]
enum PublicPolicy {
    /// PUBLIC delivers nothing on the composed objects.
    Deny,
    /// PUBLIC may deliver `SELECT` on the composed relations and columns.
    PermitSelectedDelivery,
}

fn main() -> ExitCode {
    // Consumer shape A submits enqueue intents with narrow column grants, runs
    // fixed-window quotas and authentication attempts, and permits selected
    // PUBLIC delivery. It deliberately does not select the privileged
    // full-schema snapshot: that selection needs relation-level SELECT on five
    // relations, so it belongs to a separate login below.
    let shape_a = compose(
        "intent_writer",
        PublicPolicy::PermitSelectedDelivery,
        &[RunledgerOperation::IntentSubmission],
        &[
            RunlimitOperation::FixedWindowAdmission,
            RunlimitOperation::FixedWindowExpiryCleanup,
            RunlimitOperation::AuthenticationAttempts,
        ],
    );
    // Shape A's startup inspection login. Selecting the snapshot alone keeps the
    // relation-wide reads it needs away from the serving login.
    let inspector = compose(
        "schema_inspector",
        PublicPolicy::Deny,
        &[RunledgerOperation::SchemaSnapshot],
        &[],
    );
    // Consumer shape B submits intents, runs direct-job workers with durable
    // promotion, uses GCRA quotas and attempts, and denies PUBLIC delivery.
    let shape_b = compose(
        "worker_service",
        PublicPolicy::Deny,
        &[
            RunledgerOperation::DirectJobExecution,
            RunledgerOperation::IntentPromotion,
            RunledgerOperation::IntentSubmission,
        ],
        &[
            RunlimitOperation::GcraAdmission,
            RunlimitOperation::GcraExpiryCleanup,
            RunlimitOperation::AuthenticationAttempts,
        ],
    );
    match (shape_a, inspector, shape_b) {
        (Ok(first), Ok(second), Ok(third)) => {
            print!("{first}\n{second}\n{third}");
            ExitCode::SUCCESS
        }
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            eprintln!("the composed exact role is invalid: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Compile one role and render its inert grant plan.
fn render(
    role: &str,
    compiled: &CompiledExactRole,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(format!(
        "-- exact role {role}\n{}",
        compiled.grant_plan().render(
            &Identifier::new(role)?,
            Some(&Identifier::new("service_database")?),
        )?
    ))
}

fn compose(
    role: &str,
    public: PublicPolicy,
    runledger: &[RunledgerOperation],
    runlimit: &[RunlimitOperation],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let jobs = Identifier::new("jobs")?;
    let quotas = Identifier::new("quotas")?;
    let application = Identifier::new("service")?;

    // Discovery covers exactly the schemas this role declares objects in, so an
    // unselected native store is neither audited nor reachable.
    let mut scope = vec![application.clone()];
    if !runledger.is_empty() {
        scope.push(jobs.clone());
    }
    if !runlimit.is_empty() {
        scope.push(quotas.clone());
    }

    // The application owns every policy a fragment cannot carry.
    let mut manifest = ExactRoleManifest::new(application.clone(), DiscoveryScope::Schemas(scope))?;
    manifest.set_role_policy(RolePolicy::default())?;
    manifest.deny_current_database_ownership(true);
    manifest.set_discovery_defaults(defaults())?;
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

    // The native requirements arrive as fragments; no native relation, column or
    // privilege list is written here. An empty selection is deliberately not
    // composed: the producers reject one rather than contributing nothing.
    if !runledger.is_empty() {
        manifest = manifest.with_fragment(batter::runledger::grants::grant_fragment(
            &objects(jobs, public)?,
            runledger.iter().copied(),
        )?)?;
    }
    if !runlimit.is_empty() {
        manifest = manifest.with_fragment(batter::runlimit::grants::grant_fragment(
            &objects(quotas, public)?,
            runlimit.iter().copied(),
        )?)?;
    }

    // The application's own relation, columns and routine stay ordinary manifest
    // inputs beside them.
    manifest.add_schema(SchemaGrantSpec::new(
        application.clone(),
        [ObjectPrivilege::Usage],
        DeclarationPurpose::RequiredAndProvisioned,
    )?)?;
    let deliveries = match public {
        PublicPolicy::Deny => PublicDelivery::Deny,
        PublicPolicy::PermitSelectedDelivery => PublicDelivery::AllowDeclared,
    };
    let notes = QualifiedName::new(application.as_str(), "delivery_notes")?;
    manifest.add_relations(
        RelationGrantGroup::new(
            [notes.clone()],
            [],
            DeclarationPurpose::RequiredAndProvisioned,
        )?
        .allow_row_type_public_usage(true),
    )?;
    manifest.add_columns(
        ColumnGrantGroup::new(
            notes.clone(),
            [Identifier::new("id")?, Identifier::new("note")?],
            [ObjectPrivilege::Select],
            DeclarationPurpose::RequiredAndProvisioned,
        )?
        .public_delivery(deliveries),
    )?;
    manifest.add_columns(ColumnGrantGroup::new(
        notes,
        [Identifier::new("note")?],
        [ObjectPrivilege::Insert],
        DeclarationPurpose::RequiredAndProvisioned,
    )?)?;
    render(role, &manifest.compile()?)
}

/// The application's choices for the native objects a fragment declares.
fn objects(
    schema: Identifier,
    public: PublicPolicy,
) -> Result<FragmentObjectPolicy, Box<dyn std::error::Error + Send + Sync>> {
    // PostgreSQL's default PUBLIC USAGE on composite row types is retained here
    // rather than reset, because both audited consumer shapes keep it.
    let policy = FragmentObjectPolicy::new(schema).allow_row_type_public_usage(true);
    Ok(match public {
        PublicPolicy::Deny => policy,
        PublicPolicy::PermitSelectedDelivery => {
            policy.with_public_delivered_privileges([ObjectPrivilege::Select])?
        }
    })
}

/// Discovery defaults for objects in scope that no declaration names.
fn defaults() -> DiscoveryDefaults {
    DiscoveryDefaults {
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

#[cfg(test)]
mod tests {
    use super::{PublicPolicy, compose};
    use batter::runledger::grants::RunledgerOperation;
    use batter::runlimit::grants::RunlimitOperation;

    #[test]
    fn both_consumer_shapes_render_deterministically_without_grant_options() {
        for public in [PublicPolicy::Deny, PublicPolicy::PermitSelectedDelivery] {
            let rendered = compose(
                "service_login",
                public,
                &[
                    RunledgerOperation::DirectJobExecution,
                    RunledgerOperation::IntentSubmission,
                ],
                &[RunlimitOperation::GcraAdmission],
            )
            .expect("the composed exact role is valid");
            assert_eq!(
                rendered,
                compose(
                    "service_login",
                    public,
                    &[
                        RunledgerOperation::IntentSubmission,
                        RunledgerOperation::DirectJobExecution,
                    ],
                    &[RunlimitOperation::GcraAdmission],
                )
                .expect("the composed exact role is valid"),
            );
            assert!(!rendered.contains("WITH GRANT OPTION"));
            assert!(!rendered.contains("ALL PRIVILEGES"));
            assert!(rendered.contains("GRANT CONNECT ON DATABASE \"service_database\""));
            assert!(rendered.contains("ON TABLE \"jobs\".\"job_enqueue_intents\""));
            assert!(rendered.contains("ON TABLE \"quotas\".\"runlimit_gcra\""));
            assert!(rendered.contains("ON TABLE \"service\".\"delivery_notes\""));
            // Intent submission is column scoped, so the relation-wide reads the
            // full-schema snapshot needs cannot appear in a serving role.
            assert!(!rendered.contains("GRANT SELECT ON TABLE"));
        }
    }

    #[test]
    fn the_inspection_role_selects_only_the_snapshot() {
        let rendered = compose(
            "schema_inspector",
            PublicPolicy::Deny,
            &[RunledgerOperation::SchemaSnapshot],
            &[],
        )
        .expect("the composed exact role is valid");
        // The snapshot is relation wide, which is exactly why it is a separate
        // login rather than part of a serving role.
        assert_eq!(rendered.matches("GRANT SELECT ON TABLE").count(), 5);
        assert!(!rendered.contains("\"quotas\""));
        assert!(!rendered.contains("job_enqueue_intents"));
    }
}
