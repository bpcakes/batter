use super::super::required::tests::snapshot;
use super::super::{AclEntry, RoutineObject, TypeObject};
use super::*;
use crate::verification::report::{FindingKind, VerificationReport, VerificationStatus};

fn report(snapshot: &CatalogSnapshot, policy: &AuthorityPolicy) -> VerificationReport {
    crate::verification::authority::evaluation::tests::run(super::super::evaluate_snapshot(
        snapshot.clone(),
        policy,
        Vec::new(),
        false,
    ))
    .unwrap()
}

#[test]
fn thousands_of_discovered_public_defaults_keep_complete_verdict_and_detect_one_excess() {
    // This tests real full-collection evaluation, not just HashMap lookups.
    // The watchdog catches nonyielding regressions; verdicts and the injected
    // excess are the behavioral oracle, independent of elapsed wall time.
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut snapshot = snapshot();
        snapshot.relations.clear();
        for ordinal in 0..4096 {
            snapshot.types.push(TypeObject {
                schema: "service".to_owned(),
                name: format!("kind_{ordinal}"),
                owner: 99,
                relation_oid: 0,
                acl: vec![AclEntry {
                    grantee: 0,
                    privilege: ObjectPrivilege::Usage,
                    grant_option: false,
                }]
                .into(),
            });
            snapshot.routines.push(RoutineObject {
                schema: "service".to_owned(),
                name: format!("read_{ordinal}"),
                owner: 99,
                security_definer: false,

                argument_types: Vec::new(),
                acl: vec![AclEntry {
                    grantee: 0,
                    privilege: ObjectPrivilege::Execute,
                    grant_option: false,
                }],
            });
        }
        let mut policy = AuthorityPolicy {
            discovery: DiscoveryScope::UserSchemas,
            ..AuthorityPolicy::default()
        };
        policy.defaults.types.public_privileges =
            vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)];
        policy.defaults.invoker_routines.public_privileges =
            vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)];
        let clean = report(&snapshot, &policy);
        assert_eq!(clean.status(), VerificationStatus::WithinDeclaredPolicy);
        assert!(clean.findings().is_empty());
        snapshot.routines[3071].security_definer = true;
        let excess = report(&snapshot, &policy);
        assert_eq!(excess.status(), VerificationStatus::Violations);
        assert_eq!(excess.findings().len(), 1);
        let finding = &excess.findings()[0];
        assert_eq!(finding.kind, FindingKind::Privilege);
        assert_eq!(
            finding.object.as_deref(),
            Some("\"service\".\"read_3071\"()")
        );
        assert_eq!(finding.subject.as_deref(), Some("public"));
        assert_eq!(finding.privilege, Some(ObjectPrivilege::Execute));
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("bounded catalog evaluation exceeded watchdog or worker failed");
    worker.join().unwrap();
}
