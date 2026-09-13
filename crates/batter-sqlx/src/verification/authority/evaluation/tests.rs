use super::*;
use crate::verification::authority::{AclEntry, Membership, TypeObject, evaluate_snapshot};
use crate::verification::{
    AllowedPrivilege, AuthorityPolicy, DiscoveryScope, FindingKind, ObjectPrivilege,
};
use batter::operation::{Interruption, OperationContext, OperationError};
use std::future::Future;
use std::task::{Context, Waker};
use std::time::Duration;

/// Pure snapshot oracles poll the production future without database IO.
pub(crate) fn run<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return result;
        }
    }
}

fn large_snapshot() -> (
    crate::verification::authority::CatalogSnapshot,
    AuthorityPolicy,
) {
    let mut snapshot = crate::verification::authority::required::tests::snapshot();
    snapshot.relations.clear();
    for oid in 10_000..12_048 {
        let mut role = snapshot.roles[0].clone();
        role.oid = oid;
        role.name = format!("role_{oid}");
        snapshot.roles.push(role);
        snapshot.memberships.push(Membership {
            role: oid,
            member: 1,
            admin: false,
            inherit: true,
            set: false,
        });
    }
    for ordinal in 0..4096 {
        snapshot.types.push(TypeObject {
            schema: "service".into(),
            name: format!("kind_{ordinal}"),
            owner: 99,
            relation_oid: 0,
            acl: vec![AclEntry {
                grantee: 1,
                privilege: ObjectPrivilege::Usage,
                grant_option: false,
            }]
            .into(),
        });
    }
    let mut policy = AuthorityPolicy {
        discovery: DiscoveryScope::UserSchemas,
        ..AuthorityPolicy::default()
    };
    policy
        .defaults
        .types
        .privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Usage, false));
    (snapshot, policy)
}

#[tokio::test]
async fn loaded_evaluation_yields_and_cancellation_wins_before_a_report() {
    let (snapshot, policy) = large_snapshot();
    let context = OperationContext::new(Duration::from_secs(30)).unwrap();
    let operation = context.run("test.evaluation", |_| {
        evaluate_snapshot(snapshot, &policy, Vec::new(), false)
    });
    tokio::pin!(operation);
    // The first poll enters the real post-load evaluator and must yield before
    // completion. Removing its checkpoints makes this assertion fail.
    assert!(futures_util::poll!(&mut operation).is_pending());
    context.cancel();
    assert!(matches!(
        operation.await,
        Err(OperationError::Interrupted(Interruption::Cancelled))
    ));
}

#[tokio::test(start_paused = true)]
async fn loaded_evaluation_observes_deadline_after_its_first_poll() {
    let (snapshot, policy) = large_snapshot();
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let operation = context.run("test.evaluation", |_| {
        evaluate_snapshot(snapshot, &policy, Vec::new(), false)
    });
    tokio::pin!(operation);
    assert!(futures_util::poll!(&mut operation).is_pending());
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        operation.await,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
    ));
}

#[test]
fn many_roles_and_objects_finish_and_retain_a_late_grant_option() {
    let (mut snapshot, policy) = large_snapshot();
    // More than eight million role/object combinations must fit the one-million
    // visit budget by considering actual ACL sources, without skipping a late one.
    let last = snapshot.types.last_mut().unwrap();
    last.acl = vec![AclEntry {
        grantee: 12_047,
        privilege: ObjectPrivilege::Usage,
        grant_option: true,
    }]
    .into();
    let report = run(evaluate_snapshot(snapshot, &policy, Vec::new(), false)).unwrap();
    assert_eq!(report.findings().len(), 1);
    let finding = &report.findings()[0];
    assert_eq!(finding.kind, FindingKind::GrantOption);
    assert_eq!(finding.object.as_deref(), Some("\"service\".\"kind_4095\""));
    assert_eq!(finding.subject.as_deref(), Some("role_12047"));
}

#[tokio::test]
async fn evaluation_capacity_rejects_excess_work_and_report_bytes() {
    let mut evaluation = Evaluation::new();
    for _ in 0..MAX_VISITS {
        evaluation.checkpoint(&[]).await.unwrap();
    }
    assert!(matches!(
        evaluation.checkpoint(&[]).await,
        Err(VerificationError::EvaluationCapacity)
    ));
    let mut evaluation = Evaluation::new();
    let finding = Finding::new(
        FindingKind::MissingObject,
        Some("x".repeat(MAX_FINDING_BYTES)),
        None::<String>,
        None,
    );
    assert!(matches!(
        evaluation.checkpoint(&[finding]).await,
        Err(VerificationError::EvaluationCapacity)
    ));
}

#[test]
fn findings_preserve_distinct_dotted_and_quoted_catalog_identities() {
    let mut snapshot = crate::verification::authority::required::tests::snapshot();
    snapshot.relations.clear();
    for (schema, name) in [("a.b", "c"), ("a", "b.c"), ("a\"b", "c")] {
        snapshot.types.push(TypeObject {
            schema: schema.into(),
            name: name.into(),
            owner: 99,
            relation_oid: 0,
            acl: vec![AclEntry {
                grantee: 1,
                privilege: ObjectPrivilege::Usage,
                grant_option: false,
            }]
            .into(),
        });
    }
    let policy = AuthorityPolicy {
        discovery: DiscoveryScope::UserSchemas,
        ..AuthorityPolicy::default()
    };
    let report = run(evaluate_snapshot(snapshot, &policy, Vec::new(), false)).unwrap();
    let names: std::collections::HashSet<_> = report
        .findings()
        .iter()
        .map(|finding| finding.object.as_deref().unwrap())
        .collect();
    assert_eq!(
        names,
        std::collections::HashSet::from(["\"a.b\".\"c\"", "\"a\".\"b.c\"", "\"a\"\"b\".\"c\""])
    );
}

#[tokio::test]
async fn excessive_real_evaluation_returns_an_error_instead_of_a_partial_report() {
    use crate::verification::authority::ParameterObject;
    use crate::verification::{ParameterName, ParameterPolicy};
    let (mut snapshot, mut policy) = large_snapshot();
    snapshot.types.clear();
    // Active roles have implicit SET on user-context parameters even without
    // ACL rows. This is real work that sparse object ACL indexing cannot omit.
    for edge in &mut snapshot.memberships {
        edge.set = true;
    }
    for ordinal in 0..600 {
        let name = format!("service.parameter_{ordinal}");
        snapshot.parameters.push(ParameterObject {
            name: name.clone(),
            observable: true,
            context: Some("user".into()),
            exists: true,
            custom: false,
            acl: Vec::new(),
        });
        policy.parameters.push(ParameterPolicy {
            parameter: ParameterName::new(name).unwrap(),
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
        });
    }
    policy.validate().unwrap();
    assert!(matches!(
        evaluate_snapshot(snapshot, &policy, Vec::new(), false).await,
        Err(VerificationError::EvaluationCapacity)
    ));
}

#[test]
fn policy_limits_bound_nested_input_and_repeated_default_expansion() {
    let mut policy = AuthorityPolicy::default();
    policy.defaults.types.privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false); 33];
    assert!(matches!(
        policy.validate(),
        Err(crate::verification::PolicyError::AuthorityCapacity)
    ));
    policy.defaults.types.privileges.clear();
    policy.roles.allowed_admin_roles =
        vec![crate::verification::Identifier::new("role").unwrap(); 10_001];
    assert!(matches!(
        policy.validate(),
        Err(crate::verification::PolicyError::AuthorityCapacity)
    ));
}
