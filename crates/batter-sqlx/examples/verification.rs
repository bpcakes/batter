//! Run the generic read-only PostgreSQL verifier against an externally
//! provisioned database. The migration and grant policy stays application-owned.

use batter::operation::{Interruption, OperationContext, OperationError};
use batter_sqlx::verification::VerificationError;
use batter_sqlx::verification::{
    AdditionalMigrations, AllowedPrivilege, AuthorityPolicy, DatabasePolicy, Identifier,
    MigrationExpectation, MigrationPolicy, ObjectPrivilege, PublicGrant, PublicObject,
    QualifiedName, RelationPolicy, RolePolicy, SchemaPolicy, VerificationPolicy,
    VerificationStatus, verify,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{process::ExitCode, str::FromStr, time::Duration};

#[tokio::main(flavor = "current_thread")]
#[allow(clippy::too_many_lines)]
async fn main() -> ExitCode {
    let Some(url) = std::env::var_os("DATABASE_URL") else {
        eprintln!("DATABASE_URL is required");
        return ExitCode::FAILURE;
    };
    let Some(url) = url.to_str() else {
        eprintln!("DATABASE_URL must be valid Unicode");
        return ExitCode::FAILURE;
    };
    let options = match PgConnectOptions::from_str(url) {
        Ok(options) => options,
        Err(_) => {
            eprintln!("DATABASE_URL is not a valid PostgreSQL connection string");
            return ExitCode::FAILURE;
        }
    };
    let schema =
        std::env::var("BATTER_VERIFY_MIGRATION_SCHEMA").unwrap_or_else(|_| "public".to_owned());
    let table = std::env::var("BATTER_VERIFY_MIGRATION_TABLE")
        .unwrap_or_else(|_| "_sqlx_migrations".to_owned());
    let version = match std::env::var("BATTER_VERIFY_REQUIRED_VERSION") {
        Ok(version) => match version.parse::<i64>() {
            Ok(version) => version,
            Err(_) => {
                eprintln!("BATTER_VERIFY_REQUIRED_VERSION must be an integer");
                return ExitCode::FAILURE;
            }
        },
        Err(_) => {
            eprintln!(
                "BATTER_VERIFY_REQUIRED_VERSION and BATTER_VERIFY_REQUIRED_CHECKSUM_HEX are required"
            );
            return ExitCode::FAILURE;
        }
    };
    let checksum = match std::env::var("BATTER_VERIFY_REQUIRED_CHECKSUM_HEX") {
        Ok(value) => match parse_hex(&value) {
            Some(checksum) => checksum,
            None => {
                eprintln!("BATTER_VERIFY_REQUIRED_CHECKSUM_HEX must contain an even hex string");
                return ExitCode::FAILURE;
            }
        },
        Err(_) => {
            eprintln!(
                "BATTER_VERIFY_REQUIRED_VERSION and BATTER_VERIFY_REQUIRED_CHECKSUM_HEX are required"
            );
            return ExitCode::FAILURE;
        }
    };
    let ledger = match QualifiedName::new(schema, table) {
        Ok(ledger) => ledger,
        Err(_) => {
            eprintln!("migration ledger identifiers are invalid");
            return ExitCode::FAILURE;
        }
    };
    let allow_owner = flag("BATTER_VERIFY_ALLOW_OWNER");
    let deliberate_violation = flag("BATTER_VERIFY_DELIBERATE_VIOLATION");
    let allow_later = flag("BATTER_VERIFY_ALLOW_LATER");
    let allow_superuser = flag("BATTER_VERIFY_ALLOW_SUPERUSER");
    let schema = Identifier::new(ledger.schema()).expect("validated ledger schema");
    let public_schema = PublicObject::Schema(schema.clone());
    let relation_privileges = if deliberate_violation {
        Vec::new()
    } else {
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)]
    };
    let migration = MigrationPolicy {
        ledger: ledger.clone(),
        required: vec![MigrationExpectation::new(
            version,
            if deliberate_violation {
                vec![0]
            } else {
                checksum
            },
        )],
        additional: if allow_later {
            AdditionalMigrations::AllowSuccessful
        } else {
            AdditionalMigrations::Reject
        },
    };
    let authority = AuthorityPolicy {
        roles: RolePolicy {
            allow_superuser,
            allow_create_database: allow_superuser,
            allow_create_role: allow_superuser,
            allow_replication: allow_superuser,
            allow_bypass_rls: allow_superuser,
            allowed_admin_roles: Vec::new(),
            allowed_predefined_roles: if allow_superuser {
                [
                    "pg_checkpoint",
                    "pg_create_subscription",
                    "pg_database_owner",
                    "pg_execute_server_program",
                    "pg_maintain",
                    "pg_monitor",
                    "pg_read_all_data",
                    "pg_read_all_settings",
                    "pg_read_all_stats",
                    "pg_read_server_files",
                    "pg_signal_autovacuum_worker",
                    "pg_signal_backend",
                    "pg_stat_scan_tables",
                    "pg_use_reserved_connections",
                    "pg_write_all_data",
                    "pg_write_server_files",
                ]
                .into_iter()
                .map(|name| Identifier::new(name).expect("catalog predefined role identifier"))
                .collect()
            } else if allow_owner {
                vec![Identifier::new("pg_database_owner").expect("catalog role identifier")]
            } else {
                Vec::new()
            },
        },
        relations: vec![RelationPolicy {
            relation: ledger,
            privileges: relation_privileges,
            columns: Vec::new(),
            allow_owner,
            allow_row_type_public_usage: true,
        }],
        schemas: vec![SchemaPolicy {
            schema,
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)],
            allow_owner,
        }],
        database: DatabasePolicy {
            privileges: vec![
                AllowedPrivilege::new(ObjectPrivilege::Connect, false),
                AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
            ],
            allow_owner,
        },
        public_grants: vec![
            PublicGrant {
                object: PublicObject::Database,
                privilege: AllowedPrivilege::new(ObjectPrivilege::Connect, false),
            },
            PublicGrant {
                object: PublicObject::Database,
                privilege: AllowedPrivilege::new(ObjectPrivilege::Temporary, false),
            },
            PublicGrant {
                object: public_schema,
                privilege: AllowedPrivilege::new(ObjectPrivilege::Usage, false),
            },
        ],
        ..AuthorityPolicy::default()
    };
    let policy = VerificationPolicy::new(migration, authority);
    let context = OperationContext::new(Duration::from_secs(30))
        .expect("constant verification budget is valid");
    // Lazy pool construction performs no connection work outside the operation.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy_with(options);
    let result = verify(&pool, &context, &policy).await;
    pool.close().await;
    let report = match result {
        Ok(report) => report,
        Err(OperationError::Interrupted(Interruption::Cancelled)) => {
            eprintln!("PostgreSQL verification was cancelled");
            return ExitCode::FAILURE;
        }
        Err(OperationError::Interrupted(_)) => {
            eprintln!("PostgreSQL verification exceeded its operation budget");
            return ExitCode::FAILURE;
        }
        Err(OperationError::Failed(VerificationError::EvaluationCapacity)) => {
            eprintln!(
                "PostgreSQL verification exceeded evaluation capacity; no report is available"
            );
            return ExitCode::FAILURE;
        }
        Err(_) => {
            eprintln!("PostgreSQL verification failed; no report is available");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "verification status={:?} findings={} unsupported={} deliberate_violation={deliberate_violation}",
        report.status(),
        report.findings().len(),
        report.unsupported().len()
    );
    if flag("BATTER_VERIFY_PRINT_FINDINGS") {
        for finding in report.findings() {
            eprintln!(
                "finding kind={:?} object={:?} subject={:?}",
                finding.kind, finding.object, finding.subject
            );
        }
    }
    if report.status() == VerificationStatus::WithinDeclaredPolicy {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

fn parse_hex(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        bytes.push((high * 16 + low) as u8);
    }
    Some(bytes)
}
