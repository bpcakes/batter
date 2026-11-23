//! Cadence between lease-reaper passes.

use runledger_postgres::jobs::ReapExpiredLeasesDetailedResult;

/// What the reaper loop does after one pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReaperNextPass {
    /// Shutdown was observed while notifying a committed batch's side effects.
    Shutdown,
    /// Yield, check shutdown, then reap the next batch.
    Immediately,
    /// Wait for shutdown or the configured reaper interval.
    AfterInterval,
}

/// Chooses the cadence after a committed batch whose side effects were notified.
///
/// Only a whole batch that reaped at least one lease continues at once: every
/// reaped row left the expired-lease set, so each immediate pass makes durable
/// progress. A batch of only deferred rows waits, because a short
/// `reaper_retry_delay_ms` can make those rows expire again before the next
/// pass. A saturated reaped-lease observer owner also waits, so back-to-back
/// batches do not drop best-effort callbacks that the interval gives time to
/// finish.
pub(super) fn after_committed_batch(
    result: &ReapExpiredLeasesDetailedResult,
    limit: i64,
    observers_saturated: bool,
) -> ReaperNextPass {
    next_pass(
        result.summary.processed,
        result.deferred_row_error_count,
        limit,
        observers_saturated,
    )
}

fn next_pass(
    processed: i64,
    deferred_row_errors: usize,
    limit: i64,
    observers_saturated: bool,
) -> ReaperNextPass {
    let deferred_row_errors = i64::try_from(deferred_row_errors).unwrap_or(i64::MAX);
    let full_with_progress =
        processed > 0 && processed.saturating_add(deferred_row_errors) >= limit;
    if full_with_progress && !observers_saturated {
        ReaperNextPass::Immediately
    } else {
        ReaperNextPass::AfterInterval
    }
}

#[cfg(test)]
mod tests {
    use super::ReaperNextPass::{AfterInterval, Immediately};
    use super::next_pass;

    #[test]
    fn whole_batch_with_reaped_rows_continues() {
        assert_eq!(next_pass(16, 0, 16, false), Immediately);
        assert_eq!(next_pass(1, 15, 16, false), Immediately);
        assert_eq!(next_pass(1, 0, 1, false), Immediately);
    }

    #[test]
    fn whole_batch_of_only_deferred_rows_waits() {
        assert_eq!(
            next_pass(0, 16, 16, false),
            AfterInterval,
            "deferred rows can expire again at once, so they must not drive re-polling"
        );
    }

    #[test]
    fn partial_and_empty_batches_wait() {
        assert_eq!(next_pass(15, 0, 16, false), AfterInterval);
        assert_eq!(next_pass(7, 8, 16, false), AfterInterval);
        assert_eq!(next_pass(0, 0, 16, false), AfterInterval);
    }

    #[test]
    fn saturated_observers_wait_even_after_a_whole_batch() {
        assert_eq!(
            next_pass(16, 0, 16, true),
            AfterInterval,
            "an immediate pass would drop the next batch's observer callbacks"
        );
    }
}
