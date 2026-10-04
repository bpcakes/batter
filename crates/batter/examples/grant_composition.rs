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

/// One consumer-shaped role this example renders.
///
/// The three roles are defined once here so the tests below assert on the same
/// values `main` prints. A selection removed from a role is therefore a failing
/// test, not a silently narrower example.
struct ConsumerRole {
    role: &'static str,
    public: PublicPolicy,
    application: ApplicationObjects,
    runledger: &'static [RunledgerOperation],
    runlimit: &'static [RunlimitOperation],
}

/// Consumer shape A submits enqueue intents with narrow column grants, runs
/// fixed-window quotas and authentication attempts, and permits selected PUBLIC
/// delivery. It deliberately does not select the privileged full-schema
/// snapshot: that selection needs relation-level `SELECT` on five relations, so
/// it belongs to the separate inspection login below.
const INTENT_WRITER: ConsumerRole = ConsumerRole {
    role: "intent_writer",
    public: PublicPolicy::PermitSelectedDelivery,
    application: ApplicationObjects::DeliveryNotes,
    runledger: &[RunledgerOperation::IntentSubmission],
    runlimit: &[
        RunlimitOperation::FixedWindowAdmission,
        RunlimitOperation::FixedWindowExpiryCleanup,
        RunlimitOperation::AuthenticationAttempts,
    ],
};

/// Shape A's startup inspection login. Selecting the snapshot alone keeps the
/// relation-wide reads it needs away from every serving login.
const SCHEMA_INSPECTOR: ConsumerRole = ConsumerRole {
    role: "schema_inspector",
    public: PublicPolicy::Deny,
    application: ApplicationObjects::None,
    runledger: &[RunledgerOperation::SchemaSnapshot],
    runlimit: &[],
};

/// Consumer shape B submits intents and runs direct-job workers under the native
/// supervisor's default loops, uses GCRA quotas and attempts, and denies PUBLIC
/// delivery. The default supervisor enables the scheduler loop, so scheduled
/// dispatch is part of this role; a deployment that calls
/// `SupervisorBuilder::disable_scheduler` drops that selection instead.
const WORKER_SERVICE: ConsumerRole = ConsumerRole {
    role: "worker_service",
    public: PublicPolicy::Deny,
    application: ApplicationObjects::DeliveryNotes,
    runledger: &[
        RunledgerOperation::DirectJobExecution,
        RunledgerOperation::IntentPromotion,
        RunledgerOperation::IntentSubmission,
        RunledgerOperation::ScheduledDispatch,
    ],
    runlimit: &[
        RunlimitOperation::GcraAdmission,
        RunlimitOperation::GcraExpiryCleanup,
        RunlimitOperation::AuthenticationAttempts,
    ],
};

/// Every role this example renders, in output order.
const CONSUMER_ROLES: [&ConsumerRole; 3] = [&INTENT_WRITER, &SCHEMA_INSPECTOR, &WORKER_SERVICE];

fn main() -> ExitCode {
    let mut rendered = String::new();
    for consumer in CONSUMER_ROLES {
        match compose(consumer) {
            Ok(role) => rendered.push_str(&role),
            Err(error) => {
                eprintln!("the composed exact role is invalid: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    print!("{rendered}");
    ExitCode::SUCCESS
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

fn compose(consumer: &ConsumerRole) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let ConsumerRole {
        role,
        public,
        application: application_objects,
        runledger,
        runlimit,
    } = *consumer;
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

    if owns_application {
        manifest = declare_application_objects(manifest, &application, public)?;
    }
    render(role, &manifest.compile()?)
}

/// The application's own schema, relation, columns and routine. These are
/// ordinary manifest inputs beside the fragments, declared only for a role that
/// selected them.
fn declare_application_objects(
    mut manifest: ExactRoleManifest,
    application: &Identifier,
    public: PublicPolicy,
) -> Result<ExactRoleManifest, Box<dyn std::error::Error + Send + Sync>> {
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
    Ok(manifest)
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
    use super::{
        ApplicationObjects, CONSUMER_ROLES, ConsumerRole, INTENT_WRITER, PublicPolicy,
        SCHEMA_INSPECTOR, WORKER_SERVICE, compose,
    };
    use batter::runledger::grants::RunledgerOperation;

    fn render(consumer: &ConsumerRole) -> String {
        compose(consumer).expect("the composed exact role is valid")
    }

    #[test]
    fn every_rendered_role_is_deterministic_and_grants_no_options() {
        for consumer in CONSUMER_ROLES {
            let rendered = render(consumer);
            assert_eq!(rendered, render(consumer));
            assert!(!rendered.contains("WITH GRANT OPTION"));
            assert!(!rendered.contains("ALL PRIVILEGES"));
            assert!(rendered.contains("GRANT CONNECT ON DATABASE \"service_database\""));
        }
    }

    #[test]
    fn the_serving_shapes_compose_both_adapters_with_their_own_objects() {
        for consumer in [&INTENT_WRITER, &WORKER_SERVICE] {
            let rendered = render(consumer);
            assert!(rendered.contains("ON TABLE \"jobs\".\"job_enqueue_intents\""));
            assert!(rendered.contains("ON TABLE \"service\".\"delivery_notes\""));
            assert!(rendered.contains(
                "GRANT EXECUTE ON ROUTINE \"service\".\"normalize_note\"(\"pg_catalog\".\"text\")",
            ));
            // Intent submission is column scoped, so the relation-wide reads the
            // full-schema snapshot needs cannot appear in a serving role.
            assert!(!rendered.contains("GRANT SELECT ON TABLE"));
        }
        assert!(render(&INTENT_WRITER).contains("ON TABLE \"quotas\".\"runlimit_fixed_windows\""));
        assert!(render(&WORKER_SERVICE).contains("ON TABLE \"quotas\".\"runlimit_gcra\""));
    }

    #[test]
    fn the_inspection_role_selects_only_the_snapshot() {
        let rendered = render(&SCHEMA_INSPECTOR);
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

    #[test]
    fn the_default_loop_worker_role_carries_its_scheduler_authority() {
        // The native supervisor enables the scheduler by default, so the role
        // this example actually renders for shape B needs the schedule claim's
        // table lock and its fire-recording writes. Dropping the selection from
        // `WORKER_SERVICE` fails here rather than quietly narrowing the example.
        let rendered = render(&WORKER_SERVICE);
        assert!(rendered.contains("GRANT MAINTAIN ON TABLE \"jobs\".\"job_schedules\""));
        assert!(rendered.contains("UPDATE (\"last_fired_at\")"));
        assert!(rendered.contains("UPDATE (\"next_fire_at\")"));

        // A deployment that calls `SupervisorBuilder::disable_scheduler` drops
        // the selection, and with it every schedule grant.
        let scheduler_disabled = ConsumerRole {
            runledger: &[
                RunledgerOperation::DirectJobExecution,
                RunledgerOperation::IntentPromotion,
                RunledgerOperation::IntentSubmission,
            ],
            ..WORKER_SERVICE
        };
        assert!(!render(&scheduler_disabled).contains("job_schedules"));
    }

    #[test]
    fn a_permitted_public_delivery_does_not_change_the_rendered_grants() {
        // PUBLIC delivery changes what verification accepts, never the
        // role-targeted plan, so the two policies render identically.
        let denied = ConsumerRole {
            public: PublicPolicy::Deny,
            ..INTENT_WRITER
        };
        assert_eq!(render(&INTENT_WRITER), render(&denied));
        // The application-object choice does change the plan.
        let without_application = ConsumerRole {
            application: ApplicationObjects::None,
            ..INTENT_WRITER
        };
        assert_ne!(render(&INTENT_WRITER), render(&without_application));
    }
}
