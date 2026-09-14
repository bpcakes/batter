//! PostgreSQL 18 acceptance controls for the generic verification contract.
//!
//! These cases intentionally provision their own uniquely named roles and
//! schemas through an explicitly supplied administrative connection.  They
//! are ignored by ordinary workspace gates and selected by
//! `scripts/test_sqlx_live.sh`.

#[allow(dead_code)]
mod support;
mod verification_fixture;
mod verification_live_adoption;
mod verification_live_capacity;
mod verification_live_cases;
mod verification_live_discovery_scale;
mod verification_live_identity;
mod verification_live_inheritance;
mod verification_live_notice;
mod verification_live_ownership;
mod verification_live_parameters;
mod verification_live_protected;
mod verification_live_public;
mod verification_live_recovery;
mod verification_live_scope;
mod verification_live_visibility;

pub(crate) use verification_fixture::{AuthorityFixture, Names, exec, quote};

use batter_sqlx::verification::{
    AdditionalMigrations, AllowedPrivilege, AuthorityPolicyBuilder, ColumnPolicy, DatabasePolicy,
    FindingKind, Identifier, MigrationExpectation, MigrationPolicy, ObjectPrivilege, ParameterName,
    ParameterPolicy, PublicGrant, PublicObject, QualifiedName, RelationPolicy, RolePolicy,
    RoutinePolicy, RoutineSignature, RoutineType, SchemaPolicy, SequencePolicy, TypePolicy,
    VerificationPlan,
};
use support::Result;

#[derive(Clone)]
pub(crate) struct VerificationPolicy {
    pub(crate) migration: MigrationPolicy,
    pub(crate) authority: AuthorityPolicyBuilder,
}

impl VerificationPolicy {
    fn new(migration: MigrationPolicy, authority: AuthorityPolicyBuilder) -> Self {
        Self {
            migration,
            authority,
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn names_policy(
    names: &Names,
    schema: &str,
    ledger: &str,
    table: &str,
    role: RolePolicy,
    table_privileges: Vec<AllowedPrivilege>,
    allow_parameter_set: bool,
    include_public_table_a: bool,
    allow_sequence_privileges: bool,
    allow_table_owner: bool,
    allow_routine_execute: bool,
) -> Result<VerificationPolicy> {
    let schema_name = Identifier::new(schema)?;
    let ledger_name = QualifiedName::new(schema, ledger)?;
    let table_name = QualifiedName::new(schema, table)?;
    let sequence_name = QualifiedName::new(&names.schema_a, &names.sequence_a)?;
    let type_name = QualifiedName::new(&names.schema_a, &names.type_a)?;
    let routine_name = RoutineSignature::new(
        &names.schema_a,
        &names.routine_a,
        [RoutineType::new("pg_catalog", "int4")?],
    )?;
    let column_policies = if table == names.table_a {
        vec![
            // `visible` deliberately has no column ACL in the fixture. Its
            // presence proves requested columns are catalog identities, not
            // only rows with an explicit attacl value.
            ColumnPolicy {
                column: Identifier::new("visible")?,
                privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
            },
            ColumnPolicy {
                column: Identifier::new("secret")?,
                privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, true)],
            },
        ]
    } else {
        Vec::new()
    };
    let mut public_grants = vec![
        PublicGrant {
            object: PublicObject::Database,
            privilege: AllowedPrivilege::new(ObjectPrivilege::Connect, false),
        },
        PublicGrant {
            object: PublicObject::Database,
            privilege: AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
        },
        PublicGrant {
            object: PublicObject::Schema(Identifier::new("public")?),
            privilege: AllowedPrivilege::new(ObjectPrivilege::Usage, false),
        },
        PublicGrant {
            object: PublicObject::Type(type_name.clone()),
            privilege: AllowedPrivilege::new(ObjectPrivilege::Usage, false),
        },
        PublicGrant {
            object: PublicObject::Routine(routine_name.clone()),
            privilege: AllowedPrivilege::new(ObjectPrivilege::Execute, false),
        },
        PublicGrant {
            object: PublicObject::Relation(QualifiedName::new(&names.schema_b, &names.table_b)?),
            privilege: AllowedPrivilege::new(ObjectPrivilege::Select, false),
        },
    ];
    if include_public_table_a {
        public_grants.push(PublicGrant {
            object: PublicObject::Relation(QualifiedName::new(&names.schema_a, &names.table_a)?),
            privilege: AllowedPrivilege::new(ObjectPrivilege::Select, false),
        });
    }
    Ok(VerificationPolicy::new(
        MigrationPolicy::new(ledger_name, [MigrationExpectation::new(1, [1])])?
            .with_additional(AdditionalMigrations::Reject)?,
        AuthorityPolicyBuilder {
            roles: role,
            relations: vec![
                RelationPolicy {
                    relation: table_name.clone(),
                    privileges: table_privileges,
                    columns: column_policies,
                    allow_owner: allow_table_owner,
                    allow_row_type_public_usage: true,
                },
                RelationPolicy {
                    relation: QualifiedName::new(schema, ledger)?,
                    privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
                    columns: Vec::new(),
                    allow_owner: false,
                    allow_row_type_public_usage: true,
                },
            ],
            sequences: vec![
                SequencePolicy {
                    sequence: sequence_name,
                    privileges: if allow_sequence_privileges {
                        vec![
                            AllowedPrivilege::new(ObjectPrivilege::Usage, true),
                            AllowedPrivilege::new(ObjectPrivilege::Select, true),
                            AllowedPrivilege::new(ObjectPrivilege::Update, true),
                        ]
                    } else {
                        Vec::new()
                    },
                    allow_owner: false,
                },
                SequencePolicy {
                    sequence: QualifiedName::new(&names.schema_a, &names.sequence_default)?,
                    privileges: Vec::new(),
                    allow_owner: false,
                },
            ],
            schemas: vec![
                SchemaPolicy {
                    schema: schema_name,
                    privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
                    allow_owner: false,
                },
                SchemaPolicy {
                    schema: Identifier::new("public")?,
                    privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
                    allow_owner: false,
                },
            ],
            routines: vec![RoutinePolicy {
                routine: routine_name,
                privileges: if allow_routine_execute {
                    vec![AllowedPrivilege::new(ObjectPrivilege::Execute, true)]
                } else {
                    Vec::new()
                },
                allow_owner: false,
                allow_security_definer: false,
            }],
            // Composite row types owned by declared relations inherit the
            // relation's owner allowance and an implicit USAGE allowance;
            // only this standalone enum remains an explicit TypePolicy.
            types: [type_name]
                .into_iter()
                .map(|type_name| TypePolicy {
                    type_name,
                    privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
                    allow_owner: false,
                })
                .collect(),
            parameters: vec![
                ParameterPolicy {
                    parameter: ParameterName::new("work_mem")?,
                    privileges: if allow_parameter_set {
                        vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)]
                    } else {
                        Vec::new()
                    },
                },
                ParameterPolicy {
                    parameter: ParameterName::new("ignore_system_indexes")?,
                    privileges: if allow_parameter_set {
                        vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)]
                    } else {
                        Vec::new()
                    },
                },
            ],
            database: DatabasePolicy {
                privileges: vec![
                    AllowedPrivilege::new(ObjectPrivilege::Connect, false),
                    AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
                ],
                allow_owner: false,
            },
            public_grants,
            required_surfaces: Vec::new(),
            ..AuthorityPolicyBuilder::default()
        },
    ))
}

fn finding(report: &batter_sqlx::verification::VerificationReport, kind: FindingKind) -> bool {
    report.findings().iter().any(|item| item.kind == kind)
}

fn exact_finding(
    report: &batter_sqlx::verification::VerificationReport,
    kind: FindingKind,
    object: Option<&str>,
    subject: Option<&str>,
    privilege: Option<ObjectPrivilege>,
) -> bool {
    report.findings().iter().any(|item| {
        item.kind == kind
            && item.object.as_deref() == object
            && item.subject.as_deref() == subject
            && item.privilege == privilege
    })
}

/// Each call exercises the public pool-owned boundary with an independent budget.
async fn verify_policy(
    pool: &sqlx::PgPool,
    policy: &VerificationPolicy,
) -> Result<batter_sqlx::verification::VerificationReport> {
    let context = batter::operation::OperationContext::new(std::time::Duration::from_secs(10))?;
    let authority = policy.authority.clone().build()?;
    let plan = VerificationPlan::migrations(&policy.migration).with_authority(&authority)?;
    Ok(batter_sqlx::verification::verify(pool, &context, plan).await?)
}

async fn verify_combined(
    pool: &sqlx::PgPool,
    context: &batter::operation::OperationContext,
    policy: &VerificationPolicy,
) -> std::result::Result<
    batter_sqlx::verification::VerificationReport,
    batter::operation::OperationError<batter_sqlx::verification::VerificationError>,
> {
    let authority = policy
        .authority
        .clone()
        .build()
        .expect("live-test authority draft must compile before execution");
    let plan = VerificationPlan::migrations(&policy.migration)
        .with_authority(&authority)
        .expect("live-test plan selects one authority component");
    batter_sqlx::verification::verify(pool, context, plan).await
}
