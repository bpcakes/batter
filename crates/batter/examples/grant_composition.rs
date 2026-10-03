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
    RoutineGrantSpec, RoutineSignature, RoutineType, SchemaGrantSpec,
};
use std::process::ExitCode;

/// Whether a role touches the application's own objects at all.
///
/// This is as explicit as the native selections. A login that exists only to run
/// one native operation must not receive application read or write authority
/// just because the same `compose` helper builds it.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ApplicationObjects {
    /// The role reads and writes the application's delivery notes.
    DeliveryNotes,
    /// The role declares no application object.
    None,
}

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
        ApplicationObjects::DeliveryNotes,
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
        ApplicationObjects::None,
        &[RunledgerOperation::SchemaSnapshot],
        &[],
    );
    // Consumer shape B submits intents, runs direct-job workers under the native
    // supervisor's default loops, uses GCRA quotas and attempts, and denies
    // PUBLIC delivery. The default supervisor enables the scheduler loop, so
    // scheduled dispatch is part of this role; a deployment that calls
    // `SupervisorBuilder::disable_scheduler` drops that selection instead.
    let shape_b = compose(
        "worker_service",
        PublicPolicy::Deny,
        ApplicationObjects::DeliveryNotes,
        &[
            RunledgerOperation::DirectJobExecution,
            RunledgerOperation::IntentPromotion,
            RunledgerOperation::IntentSubmission,
            RunledgerOperation::ScheduledDispatch,
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
    application_objects: ApplicationObjects,
    runledger: &[RunledgerOperation],
    runlimit: &[RunlimitOperation],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let jobs = Identifier::new("jobs")?;
    let quotas = Identifier::new("quotas")?;
    let application = Identifier::new("service")?;
    let owns_application = application_objects == ApplicationObjects::DeliveryNotes;

    // Discovery covers exactly the schemas this role declares objects in, so an
    // unselected native store or application schema is neither audited nor
    // reachable.
    let mut scope = Vec::new();
    if owns_application {
        scope.push(application.clone());
    }
    if !runledger.is_empty() {
        scope.push(jobs.clone());
    }
    if !runlimit.is_empty() {
        scope.push(quotas.clone());
    }
    let primary = scope
        .first()
        .ok_or("a composed role needs at least one schema")?
        .clone();

    // The application owns every policy a fragment cannot carry.
    let mut manifest = ExactRoleManifest::new(primary, DiscoveryScope::Schemas(scope))?;
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

    if !owns_application {
        return render(role, &manifest.compile()?);
    }

    // The application's own schema, relation and columns stay ordinary manifest
    // inputs beside the fragments, and only for a role that declares them.
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
    // The application's own routine carries the same explicit PUBLIC choice as
    // its columns. Its overload identity is structural, so no SQL type
    // expression is parsed.
    manifest.add_routine(
        RoutineGrantSpec::new(
            RoutineSignature::new(
                application.as_str(),
                "normalize_note",
                [RoutineType::new("pg_catalog", "text")?],
            )?,
            [ObjectPrivilege::Execute],
            DeclarationPurpose::RequiredAndProvisioned,
        )?
        .public_delivery(deliveries),
    )?;
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
    use super::{ApplicationObjects, PublicPolicy, compose};
    use batter::runledger::grants::RunledgerOperation;
    use batter::runlimit::grants::RunlimitOperation;

    #[test]
    fn both_consumer_shapes_render_deterministically_without_grant_options() {
        for public in [PublicPolicy::Deny, PublicPolicy::PermitSelectedDelivery] {
            let rendered = compose(
                "service_login",
                public,
                ApplicationObjects::DeliveryNotes,
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
                    ApplicationObjects::DeliveryNotes,
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
            // The application's own routine is part of the composition, so the
            // inspector's exclusion of it below is a real check.
            assert!(rendered.contains(
                "GRANT EXECUTE ON ROUTINE \"service\".\"normalize_note\"(\"pg_catalog\".\"text\")",
            ));
            // Intent submission is column scoped, so the relation-wide reads the
            // full-schema snapshot needs cannot appear in a serving role.
            assert!(!rendered.contains("GRANT SELECT ON TABLE"));
        }
    }

    #[test]
    fn the_default_loop_worker_role_carries_its_scheduler_authority() {
        let default_loops = compose(
            "worker_service",
            PublicPolicy::Deny,
            ApplicationObjects::None,
            &[
                RunledgerOperation::DirectJobExecution,
                RunledgerOperation::ScheduledDispatch,
            ],
            &[],
        )
        .expect("the composed exact role is valid");
        // The native supervisor enables the scheduler by default, so a role for
        // it needs the schedule claim's table lock and its fire-recording writes.
        assert!(default_loops.contains("GRANT MAINTAIN ON TABLE \"jobs\".\"job_schedules\""));
        assert!(default_loops.contains("UPDATE (\"last_fired_at\")"));
        assert!(default_loops.contains("UPDATE (\"next_fire_at\")"));

        // A deployment that calls `SupervisorBuilder::disable_scheduler` drops
        // the selection, and with it every schedule grant.
        let scheduler_disabled = compose(
            "worker_service",
            PublicPolicy::Deny,
            ApplicationObjects::None,
            &[RunledgerOperation::DirectJobExecution],
            &[],
        )
        .expect("the composed exact role is valid");
        assert!(!scheduler_disabled.contains("job_schedules"));
    }

    #[test]
    fn the_inspection_role_selects_only_the_snapshot() {
        let rendered = compose(
            "schema_inspector",
            PublicPolicy::Deny,
            ApplicationObjects::None,
            &[RunledgerOperation::SchemaSnapshot],
            &[],
        )
        .expect("the composed exact role is valid");
        // The snapshot is relation wide, which is exactly why it is a separate
        // login rather than part of a serving role.
        assert_eq!(rendered.matches("GRANT SELECT ON TABLE").count(), 5);
        // An inspection login declares no quota store and no application object,
        // so it receives neither.
        for absent in [
            "\"quotas\"",
            "\"service\"",
            "job_enqueue_intents",
            "delivery_notes",
            "normalize_note",
        ] {
            assert!(!rendered.contains(absent), "unexpected grant for {absent}");
        }
    }
}
