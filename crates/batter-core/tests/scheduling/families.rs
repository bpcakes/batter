use super::{
    admission, closure_races, escalation, failures, operations, ownership, readiness,
    support::{Case, Choices, yields},
    transitions,
};

pub async fn capacity(case: Case) {
    admission::capacity_one(case, false).await;
    admission::capacity_one(Case { index: 1, ..case }, true).await;
    transitions::queued_capacity(Case { index: 2, ..case }).await;
}

pub async fn admission(case: Case, choices: &mut Choices) {
    transitions::root_drain(case, choices.next()).await;
    transitions::descendant_outlives_parent(Case { index: 1, ..case }).await;
    transitions::scope_expiry(Case { index: 2, ..case }, choices.next()).await;
    transitions::forced_descendant(Case { index: 3, ..case }).await;
    for (index, failure) in [false, true].into_iter().enumerate() {
        let verified_report = closure_races::descendant_closure(
            Case {
                index: index + 4,
                ..case
            },
            failure,
            choices.next(),
            std::future::ready(()),
        )
        .await;
        // The helper reconciles every receipt, task outcome and cleanup record.
        drop(verified_report);
    }
}

pub async fn operations(case: Case, choices: &mut Choices) {
    operations::hierarchy_and_drop(case).await;
    for (index, (cancel, expire)) in [(false, false), (true, false), (false, true), (true, true)]
        .into_iter()
        .enumerate()
    {
        operations::branch_priority(
            Case {
                index: index + 1,
                ..case
            },
            cancel,
            expire,
        )
        .await;
    }
    operations::completion_race(Case { index: 5, ..case }, choices.next()).await;
    operations::bulkhead_preflight(Case { index: 6, ..case }, choices.next()).await;
}

pub async fn readiness(case: Case, choices: &mut Choices) {
    for order in 0..3 {
        readiness::approvals(
            Case {
                index: order,
                ..case
            },
            order,
            choices.next(),
        )
        .await;
    }
    readiness::acknowledgement_race(Case { index: 3, ..case }, choices.next()).await;
    readiness::critical_exit(Case { index: 4, ..case }).await;
}

pub async fn failures(case: Case, choices: &mut Choices) {
    failures::retained_errors(case, choices.next()).await;
}

pub async fn ownership(case: Case, choices: &mut Choices) {
    ownership::receipt_waiter(case, choices.next()).await;
    ownership::cleanup_waiter(Case { index: 1, ..case }, choices.next()).await;
    ownership::last_owner(Case { index: 2, ..case }, choices.next()).await;
    ownership::caller_owned(Case { index: 3, ..case }, false).await;
    ownership::caller_owned(Case { index: 4, ..case }, true).await;
    escalation::coordinator_failure(Case { index: 5, ..case }).await;
}

pub async fn escalation(case: Case, choices: &mut Choices) {
    for mode in 0..4 {
        let verified_report = escalation::outcomes(
            Case {
                index: mode,
                ..case
            },
            mode,
            yields(choices.next()),
        )
        .await;
        // The helper has asserted the report against the observed exit path.
        drop(verified_report);
    }
}
