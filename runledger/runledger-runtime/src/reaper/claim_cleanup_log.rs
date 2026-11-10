//! Diagnostics for the coordination-claim cleanup that follows a lease batch.

use tracing::{info, warn};

pub(super) fn log_claim_cleanup(
    result: &runledger_postgres::jobs::ReapExpiredLeasesDetailedResult,
    batch_size: i64,
) {
    log_released_coordination_claims(result);
    log_coordination_claim_cleanup_errors(result);
    log_coordination_claim_cleanup_saturation(result, batch_size);
}

fn log_released_coordination_claims(
    result: &runledger_postgres::jobs::ReapExpiredLeasesDetailedResult,
) {
    if result.workflow_active_claims_released > 0 || result.execution_resource_claims_released > 0 {
        info!(
            workflow_active_claims_released = result.workflow_active_claims_released,
            execution_resource_claims_released = result.execution_resource_claims_released,
            "reaper released quiesced coordination claims"
        );
    }
}

fn log_coordination_claim_cleanup_errors(
    result: &runledger_postgres::jobs::ReapExpiredLeasesDetailedResult,
) {
    for error in &result.cleanup_errors {
        warn!(
            operation = error.operation.as_str(),
            error = %error.error,
            "reaper coordination-claim cleanup failed after the lease batch committed"
        );
    }
}

fn log_coordination_claim_cleanup_saturation(
    result: &runledger_postgres::jobs::ReapExpiredLeasesDetailedResult,
    batch_size: i64,
) {
    let Ok(batch_size) = u64::try_from(batch_size) else {
        return;
    };
    if result.workflow_active_claims_released >= batch_size
        || result.execution_resource_claims_released >= batch_size
    {
        warn!(
            workflow_active_claims_released = result.workflow_active_claims_released,
            execution_resource_claims_released = result.execution_resource_claims_released,
            cleanup_batch_size = batch_size,
            "reaper coordination-claim cleanup saturated its batch limit"
        );
    }
}
