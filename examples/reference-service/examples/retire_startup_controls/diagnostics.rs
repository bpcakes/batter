use batter::command::{CommandCause, CommandFailure, CommandOutcome, CommandReport, check_command};
use batter_example_reference_service::retirement::{
    DatabaseIdentity, DefinitionState, RetirementError, RetirementReport,
};
use serde_json::{Value, json};

#[cfg(test)]
#[path = "diagnostics/tests.rs"]
mod tests;

pub(super) fn setup_failure() -> Value {
    json!({"status": "incomplete", "stage": "setup", "failure": {"kind": "invalid_setup"},
        "usage": "retire_startup_controls SYSTEM_ID DATABASE_OID [settings-file]",
        "deployment_state_inspected": false, "retry_authorized": false})
}

pub(super) fn outcome(value: CommandOutcome<RetirementReport, RetirementError>) -> (Value, bool) {
    match check_command(value) {
        Ok(value) => (report(&value), true),
        Err(CommandFailure::Report(value)) => (report(&value), false),
        Err(CommandFailure::Coordinator(_)) => (
            json!({
                "status": "incomplete", "failure": {"kind": "command_coordinator"},
                "cleanup": {"report_available": false, "succeeded": false},
                "deployment_state_inspected": false, "retry_authorized": false
            }),
            false,
        ),
    }
}

fn report(value: &CommandReport<RetirementReport, RetirementError>) -> Value {
    let work = match &value.work {
        Ok(work) => json!({"kind": "completed", "identity": identity(work.identity),
        "cancelled": work.cancelled, "definition": match work.definition {
            DefinitionState::Absent => "absent", DefinitionState::Disabled => "disabled"
        }}),
        Err(CommandCause::Failed(error)) => failure(error),
        Err(CommandCause::Interrupted(reason)) => {
            json!({"kind": "interrupted", "reason": interruption(*reason)})
        }
        Err(CommandCause::Panicked(_)) => json!({"kind": "panicked"}),
    };
    let cleanup = match &value.cleanup {
        Ok(cleanup) => json!({"report_available": true, "succeeded": cleanup.is_success(),
            "attempted": cleanup.records.len(), "skipped": cleanup.skipped.len()}),
        Err(_) => json!({"report_available": false, "succeeded": false}),
    };
    json!({"status": if value.is_success() {"complete"} else {"incomplete"},
        "stage": value.stage, "work": work, "cleanup": cleanup,
        "interruption_after_work": value.interruption_after_work.map(interruption),
        "destruction_panic": value.destruction_panic.is_some(),
        "deployment_state_inspected": false, "retry_authorized": false})
}

fn interruption(reason: batter::operation::Interruption) -> &'static str {
    match reason {
        batter::operation::Interruption::Cancelled => "cancelled",
        batter::operation::Interruption::DeadlineExceeded => "deadline_exceeded",
    }
}

fn identity(value: DatabaseIdentity) -> Value {
    json!({"system_identifier": value.system_identifier(), "database_oid": value.database_oid()})
}

fn failure(error: &RetirementError) -> Value {
    match error {
        RetirementError::InvalidIdentity => json!({"kind": "invalid_identity"}),
        RetirementError::WrongDatabase { actual } => {
            json!({"kind": "wrong_database", "actual": identity(*actual)})
        }
        RetirementError::NotOffline {
            other_backends,
            prepared_transactions,
            cancelled,
        } => json!({
            "kind": "not_offline", "other_backends": other_backends,
            "prepared_transactions": prepared_transactions, "cancelled": cancelled}),
        RetirementError::DefinitionEnabled => json!({"kind": "definition_enabled"}),
        RetirementError::SessionReplaced { original } => json!({"kind": "session_replaced",
            "original": original.as_deref().map(failure)}),
        RetirementError::Registration(_) => json!({"kind": "registration"}),
        RetirementError::Database(_) => json!({"kind": "database"}),
        RetirementError::Native(error) => {
            json!({"kind": "native_definition", "native": native_failure(error)})
        }
        RetirementError::Cancellation(error) => json!({"kind": "cancellation",
            "job_id": error.job_id, "cancelled_before_failure": error.cancelled_before_failure,
            "readback_attempted": false, "native": native_failure(&error.native)}),
    }
}

fn native_failure(error: &runledger_postgres::Error) -> Value {
    match error {
        runledger_postgres::Error::QueryError(query) => {
            json!({"kind": "query", "code": query.code(),
            "category": match query.category() {
                runledger_postgres::QueryErrorCategory::Conflict => "conflict",
                runledger_postgres::QueryErrorCategory::Validation => "validation",
                runledger_postgres::QueryErrorCategory::Forbidden => "forbidden",
                runledger_postgres::QueryErrorCategory::Internal => "internal",
            }})
        }
        runledger_postgres::Error::RollbackFailure(pair) => json!({"kind": "rollback_failure",
            "operation": native_failure(&pair.operation), "rollback_failed": true}),
        runledger_postgres::Error::ConfigError(_) => json!({"kind": "configuration"}),
        runledger_postgres::Error::ConnectionError(_) => json!({"kind": "connection"}),
        runledger_postgres::Error::MigrationError(_) => json!({"kind": "migration"}),
    }
}
