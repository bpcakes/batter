use super::*;
use crate::verification::authority::{AclEntry, Membership, TypeObject, evaluate_snapshot};
use crate::verification::{
    AllowedPrivilege, AuthorityPolicy, DiscoveryScope, FindingKind, ObjectPrivilege, QualifiedName,
    RelationPolicy,
};
use batter::operation::{Interruption, OperationContext, OperationError};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Waker};
use std::time::Duration;

mod parameter_paths;

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

#[test]
fn near_capacity_parameter_inventory_indexes_missing_checks_linearly() {
    use crate::verification::authority::ParameterObject;
    use crate::verification::authority::database::ParameterCatalog;
    use crate::verification::authority::parameter_index::ParameterWork;
    use crate::verification::{ParameterName, ParameterPolicy, PublicObject, RequiredPrivilege};

    const POLICY_PARAMETER_COUNT: usize = 9_998;
    let mut snapshot = crate::verification::authority::required::tests::snapshot();
    snapshot.relations.clear();
    let mut policy = AuthorityPolicy::default();
    let mut captured_parameters = Vec::with_capacity(POLICY_PARAMETER_COUNT - 1);
    for ordinal in 0..POLICY_PARAMETER_COUNT {
        let name = if ordinal == 1 {
            "extension.placeholder".to_owned()
        } else {
            format!("parameter_{ordinal}")
        };
        let mut privileges = Vec::new();
        if ordinal == 1 {
            privileges.push(AllowedPrivilege::new(ObjectPrivilege::Set, false));
        }
        policy.parameters.push(ParameterPolicy {
            parameter: ParameterName::new(name.clone()).unwrap(),
            privileges,
        });
        if ordinal + 1 < POLICY_PARAMETER_COUNT {
            captured_parameters.push(ParameterObject {
                name,
                observable: ordinal != 0,
                context: (ordinal > 1).then(|| "sighup".to_owned()),
                exists: true,
                custom: ordinal == 1,
                acl: if ordinal + 2 == POLICY_PARAMETER_COUNT {
                    vec![AclEntry {
                        grantee: 1,
                        privilege: ObjectPrivilege::Set,
                        grant_option: true,
                    }]
                } else {
                    Vec::new()
                },
            });
        }
    }
    let operations = Arc::new(AtomicUsize::new(0));
    snapshot.parameters = ParameterCatalog::try_from_objects(captured_parameters)
        .unwrap()
        .with_work(ParameterWork::counted(Arc::clone(&operations)));
    policy.required_privileges.push(RequiredPrivilege {
        object: PublicObject::Parameter(policy.parameters[1].parameter.clone()),
        privilege: ObjectPrivilege::Set,
    });
    policy.validate().unwrap();

    let captured_count = snapshot.parameters.len();
    let report = run(evaluate_snapshot(snapshot, &policy, Vec::new(), false)).unwrap();

    // The full evaluator visits captured objects once to index and once to
    // inspect, probes each declaration and checks the one custom requirement.
    // A repeated build or per-declaration catalog scan records extra visits.
    assert_eq!(
        operations.load(Ordering::Relaxed),
        2 * captured_count + policy.parameters.len() + 1
    );
    let findings = report.findings();
    assert_eq!(findings.len(), 5);
    for kind in [FindingKind::ProtectedParameter, FindingKind::GrantOption] {
        assert!(findings.iter().any(|finding| {
            finding.kind == kind
                && finding.object.as_deref() == Some("parameter_9996")
                && finding.subject.as_deref() == Some("login")
                && finding.privilege == Some(ObjectPrivilege::Set)
        }));
    }
    assert!(findings.iter().any(|finding| {
        finding.kind == FindingKind::ParameterUnobservable
            && finding.object.as_deref() == Some("parameter_0")
    }));
    assert!(findings.iter().any(|finding| {
        finding.kind == FindingKind::ParameterUnobservable
            && finding.object.as_deref() == Some("extension.placeholder")
    }));
    assert!(findings.iter().any(|finding| {
        finding.kind == FindingKind::MissingObject
            && finding.object.as_deref() == Some("parameter_9997")
    }));
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

#[test]
fn relation_ownership_is_reported_once_without_column_amplification() {
    use crate::verification::authority::{ColumnObject, RelationObject};

    let mut snapshot = crate::verification::authority::required::tests::snapshot();
    snapshot.relations = (0..1_000)
        .map(|ordinal| RelationObject {
            oid: 10_000 + ordinal,
            schema: "service".to_owned(),
            name: format!("records_{ordinal}"),
            kind: "r".to_owned(),
            owner: 1,
            acl: Vec::new(),
            columns: (0..10)
                .map(|column| ColumnObject {
                    name: format!("value_{column}"),
                    acl: Vec::new(),
                })
                .collect(),
        })
        .collect();
    snapshot.types = snapshot
        .relations
        .iter()
        .map(|relation| TypeObject {
            schema: relation.schema.clone(),
            name: relation.name.clone(),
            owner: relation.owner,
            relation_oid: relation.oid,
            acl: Vec::new().into(),
        })
        .collect();
    let policy = AuthorityPolicy {
        discovery: DiscoveryScope::UserSchemas,
        ..AuthorityPolicy::default()
    };

    let report = run(evaluate_snapshot(snapshot, &policy, Vec::new(), false)).unwrap();
    assert_eq!(
        report
            .findings()
            .iter()
            .filter(|finding| finding.kind == FindingKind::Ownership)
            .count(),
        1_000
    );
    assert!(!report.findings().iter().any(|finding| {
        finding.kind == FindingKind::Ownership
            && finding
                .object
                .as_deref()
                .is_some_and(|object| object.matches('.').count() == 2)
    }));
}

#[test]
fn allowed_relation_owner_may_hold_an_explicit_column_grant() {
    use crate::verification::authority::{ColumnObject, RelationObject};

    let mut snapshot = crate::verification::authority::required::tests::snapshot();
    snapshot.relations = vec![RelationObject {
        oid: 20,
        schema: "service".to_owned(),
        name: "records".to_owned(),
        kind: "r".to_owned(),
        owner: 1,
        acl: Vec::new(),
        columns: vec![ColumnObject {
            name: "status".to_owned(),
            acl: vec![AclEntry {
                grantee: 1,
                privilege: ObjectPrivilege::Update,
                grant_option: true,
            }],
        }],
    }];
    let policy = AuthorityPolicy {
        relations: vec![RelationPolicy {
            relation: QualifiedName::new("service", "records").unwrap(),
            privileges: Vec::new(),
            columns: Vec::new(),
            allow_owner: true,
            allow_row_type_public_usage: false,
        }],
        ..AuthorityPolicy::default()
    };

    let report = run(evaluate_snapshot(snapshot, &policy, Vec::new(), false)).unwrap();
    assert!(!report.findings().iter().any(|finding| {
        finding.object.as_deref() == Some("\"service\".\"records\".\"status\"")
    }));
}

#[tokio::test]
async fn excessive_real_evaluation_returns_an_error_instead_of_a_partial_report() {
    use crate::verification::authority::ParameterObject;
    use crate::verification::authority::database::ParameterCatalog;
    use crate::verification::{ParameterName, ParameterPolicy};
    let (mut snapshot, mut policy) = large_snapshot();
    snapshot.types.clear();
    // Active roles have implicit SET on user-context parameters even without
    // ACL rows. This is real work that sparse object ACL indexing cannot omit.
    for edge in &mut snapshot.memberships {
        edge.set = true;
    }
    let mut captured_parameters = Vec::with_capacity(600);
    for ordinal in 0..600 {
        let name = format!("service.parameter_{ordinal}");
        captured_parameters.push(ParameterObject {
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
    snapshot.parameters = ParameterCatalog::try_from_objects(captured_parameters).unwrap();
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
