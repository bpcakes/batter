# ADR-003: One total deadline; replay must be authorized

Status: accepted. Date: 2026-09-07.

Context: independent timeout resets can inflate request latency. Nested retries
can multiply attempts. A network failure or caller deadline does not establish
whether a remote mutation committed.

Decision: OperationContext carries one monotonic total deadline. Children can
only shorten it. Retry uses a fresh future factory per attempt, counts the initial
attempt, includes backoff in the budget, and requires a caller replay assertion
plus application error classification. Provider delay is a lower bound. Timeout,
cancellation, and panic are not automatically retried. Preserve the last returned
application error when interruption happens later.

Consequences: callers must reason about idempotency and choose a retry owner.
Some potentially transient failures will not be retried automatically; that is
preferable to duplicating uncertain side effects. The default entrypoint retains
deterministic capped backoff. An explicit sampler supports reproducible equal
jitter without overriding provider lower bounds or the total deadline.
There is no universal is_retryable trait and no automatic transaction replay.

Amendment, 2026-09-08: `reserve_finalization` produces sibling phase contexts:
shortened work deadline and original finalization deadline. Work cancellation
does not cancel finalization; parent cancellation still reaches both. This
reserves time without shielding execution or extending the total allowance.

Amendment, 2026-09-14: `RetryOptions::with_attempt_maximum` adds an opt-in cap
derived again when each attempt starts. The cap is clamped to the unchanged
input context, including a shortened work phase, and its typed expiration is
terminal rather than replay authorization. Absolute deadline ordering selects
attempt versus total expiration; delayed runtime observation does not promise
unused total time. Options are single-execution values so stateful jitter
samplers are not accidentally cloned across concurrent calls. Retry-token
policy remains separate future work.
