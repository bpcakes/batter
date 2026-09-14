use super::run;
use crate::verification::authority::database::ParameterCatalog;
use crate::verification::authority::parameter_index::ParameterWork;
use crate::verification::authority::{AclEntry, ParameterObject, evaluate_snapshot};
use crate::verification::{
    AllowedPrivilege, AuthorityPolicy, AuthorityPolicyBuilder, Finding, FindingKind,
    ObjectPrivilege, ParameterName, ParameterPolicy, PolicyError, PublicAllowance, PublicGrant,
    PublicObject, RequiredPrivilege, VerificationReport, VerificationStatus,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const CAPTURED_PARAMETERS: usize = 10_000;

fn captured_parameters() -> Vec<ParameterObject> {
    (0..CAPTURED_PARAMETERS)
        .map(|ordinal| ParameterObject {
            name: format!("parameter_{ordinal}"),
            observable: true,
            context: Some("sighup".to_owned()),
            exists: true,
            custom: false,
            acl: Vec::new(),
        })
        .collect()
}

fn parameter_target(name: &str) -> PublicObject {
    PublicObject::Parameter(ParameterName::new(name).unwrap())
}

fn set_acl(grantee: i64) -> Vec<AclEntry> {
    vec![AclEntry {
        grantee,
        privilege: ObjectPrivilege::Set,
        grant_option: false,
    }]
}

fn evaluate(
    objects: Vec<ParameterObject>,
    policy: &AuthorityPolicy,
) -> (VerificationReport, usize) {
    assert_eq!(objects.len(), CAPTURED_PARAMETERS);
    policy.validate().unwrap();
    let mut overflow = policy.to_builder();
    overflow.public_overrides.push(PublicAllowance {
        object: parameter_target("overflow"),
        privileges: Vec::new(),
    });
    assert_eq!(overflow.build(), Err(PolicyError::AuthorityCapacity));

    let mut snapshot = crate::verification::authority::required::tests::snapshot();
    snapshot.relations.clear();
    let operations = Arc::new(AtomicUsize::new(0));
    snapshot.parameters = ParameterCatalog::try_from_objects(objects)
        .unwrap()
        .with_work(ParameterWork::counted(Arc::clone(&operations)));
    let report = run(evaluate_snapshot(snapshot, policy, Vec::new(), false)).unwrap();
    (report, operations.load(Ordering::Relaxed))
}

fn assert_findings(report: &VerificationReport, expected: &[Finding]) {
    assert_eq!(report.status(), VerificationStatus::Incomplete);
    assert_eq!(
        report.findings().len(),
        expected.len(),
        "{:?}",
        report.findings()
    );
    for finding in expected {
        assert!(report.findings().contains(finding), "missing {finding:?}");
    }
}

fn required_fixture() -> (Vec<ParameterObject>, AuthorityPolicy) {
    let mut objects = captured_parameters();
    objects[7_002].name = "extension.placeholder".to_owned();
    let mut policy = AuthorityPolicyBuilder::default();
    for (ordinal, object) in objects.iter_mut().enumerate().skip(7_000) {
        // The effective current role is 2, with role 3 inherited. Neither is
        // reachable from the login root, so these are required-authority checks.
        object.acl = set_acl(if ordinal % 2 == 0 { 2 } else { 3 });
        policy.parameters.push(ParameterPolicy {
            parameter: ParameterName::new(&object.name).unwrap(),
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
        });
        policy.required_privileges.push(RequiredPrivilege {
            object: parameter_target(&object.name),
            privilege: ObjectPrivilege::Set,
        });
    }
    policy
        .parameters
        .extend(objects[..1_000].iter().map(|object| ParameterPolicy {
            parameter: ParameterName::new(&object.name).unwrap(),
            privileges: Vec::new(),
        }));
    objects[7_000].exists = false;
    objects[7_000].acl.clear();
    objects[7_001].observable = false;
    objects[7_001].exists = false;
    objects[7_001].context = None;
    objects[7_001].acl.clear();
    objects[7_002].custom = true;
    objects[7_002].context = None;
    // Preserve the inventory size while making a selected identity absent.
    objects[9_997].name = "unselected_replacement".to_owned();
    objects[9_997].acl.clear();
    objects[9_999].acl.clear();
    (objects, policy.build().unwrap())
}

#[test]
fn near_capacity_required_parameter_callers_remain_indexed() {
    let (objects, policy) = required_fixture();
    let (report, operations) = evaluate(objects, &policy);
    // 10,000 captured objects visited twice, 4,000 declared probes and 3,000
    // requirement classification probes. Only hidden/custom requirements skip
    // the second lookup in has_privilege (2,998 ordinary privilege lookups).
    assert_eq!(operations, 20_000 + 4_000 + 3_000 + 2_998);
    let mut expected = Vec::new();
    for object in ["parameter_7000", "parameter_9997", "parameter_9999"] {
        expected.push(Finding::new(
            FindingKind::MissingPrivilege,
            Some(object.to_owned()),
            Some("current".to_owned()),
            Some(ObjectPrivilege::Set),
        ));
    }
    for object in ["parameter_7000", "parameter_9997"] {
        expected.push(Finding::new(
            FindingKind::MissingObject,
            Some(object.to_owned()),
            None::<String>,
            None,
        ));
    }
    for object in ["parameter_7001", "extension.placeholder"] {
        expected.push(Finding::new(
            FindingKind::ParameterUnobservable,
            Some(object.to_owned()),
            Some("current".to_owned()),
            None,
        ));
    }
    assert_findings(&report, &expected);
}

fn public_fixture() -> (Vec<ParameterObject>, AuthorityPolicy) {
    let mut objects = captured_parameters();
    let mut policy = AuthorityPolicyBuilder::default();
    for (ordinal, object) in objects.iter_mut().enumerate().take(7_000) {
        object.acl = set_acl(0);
        let target = parameter_target(&object.name);
        let set = AllowedPrivilege::new(ObjectPrivilege::Set, false);
        if ordinal < 4_000 {
            policy.public_grants.push(PublicGrant {
                object: target,
                privilege: set,
            });
        } else {
            policy.public_overrides.push(PublicAllowance {
                object: target,
                privileges: if ordinal == 6_999 {
                    Vec::new()
                } else {
                    vec![set]
                },
            });
        }
    }
    policy.public_overrides.push(PublicAllowance {
        object: parameter_target(&objects[7_000].name),
        privileges: Vec::new(),
    });
    for ordinal in [3_996, 6_998] {
        objects[ordinal].observable = false;
        objects[ordinal].exists = false;
        objects[ordinal].context = None;
        objects[ordinal].acl.clear();
    }
    for ordinal in [3_998, 6_997] {
        objects[ordinal].exists = false;
        objects[ordinal].acl.clear();
    }
    for ordinal in [3_997, 6_996] {
        objects[ordinal].name = format!("unselected_replacement_{ordinal}");
        objects[ordinal].acl.clear();
    }
    for ordinal in [3_999, 6_999] {
        objects[ordinal].acl[0].grant_option = true;
    }
    (objects, policy.build().unwrap())
}

#[test]
fn near_capacity_public_parameter_callers_remain_indexed() {
    let (objects, policy) = public_fixture();
    let (report, operations) = evaluate(objects, &policy);
    // 4,000 grants + 3,001 overrides + 2,999 override allowances exactly fill
    // the policy budget. Each PUBLIC target checks observability; all except
    // two hidden targets also check existence. No role declarations/requirements.
    assert_eq!(operations, 20_000 + 7_001 + 6_999);
    let mut expected = Vec::new();
    for object in [
        "parameter_3997",
        "parameter_3998",
        "parameter_6996",
        "parameter_6997",
    ] {
        expected.push(Finding::new(
            FindingKind::MissingObject,
            Some(object.to_owned()),
            None::<String>,
            None,
        ));
    }
    for object in ["parameter_3996", "parameter_6998"] {
        expected.push(Finding::new(
            FindingKind::ParameterUnobservable,
            Some(object.to_owned()),
            Some("current".to_owned()),
            None,
        ));
    }
    for object in ["parameter_3999", "parameter_6999"] {
        expected.push(Finding::new(
            FindingKind::GrantOption,
            Some(object.to_owned()),
            Some("public".to_owned()),
            Some(ObjectPrivilege::Set),
        ));
    }
    expected.push(Finding::new(
        FindingKind::ProtectedParameter,
        Some("parameter_6999".to_owned()),
        Some("public".to_owned()),
        Some(ObjectPrivilege::Set),
    ));
    assert_findings(&report, &expected);
}
