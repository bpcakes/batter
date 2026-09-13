use super::{AllowedPrivilege, AuthorityPolicy, ObjectPrivilege, PublicObject};
use std::collections::HashMap;

/// Exact PUBLIC declarations, including empty denies. An exact relation policy
/// replaces defaults for its columns; explicitly declared columns remain valid
/// exceptions. A whole-relation allowance still supplies column privileges.
pub(crate) struct PublicDeclarations<'a> {
    grants: HashMap<&'a PublicObject, Vec<AllowedPrivilege>>,
}

impl<'a> PublicDeclarations<'a> {
    pub(crate) fn new(policy: &'a AuthorityPolicy) -> Self {
        let mut grants: HashMap<&PublicObject, Vec<AllowedPrivilege>> = HashMap::new();
        for entry in &policy.public_grants {
            grants
                .entry(&entry.object)
                .or_default()
                .push(entry.privilege);
        }
        for entry in &policy.public_overrides {
            grants.insert(&entry.object, entry.privileges.clone());
        }
        Self { grants }
    }

    pub(crate) fn allows_exact(&self, object: &PublicObject, privilege: ObjectPrivilege) -> bool {
        self.grants
            .get(object)
            .is_some_and(|entries| entries.iter().any(|entry| entry.privilege == privilege))
    }

    pub(crate) fn defaults_apply(&self, object: &PublicObject) -> bool {
        !self.grants.contains_key(object)
            && match object {
                PublicObject::Column(relation, _) => !self
                    .grants
                    .contains_key(&PublicObject::Relation(relation.clone())),
                _ => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::{
        DiscoveryScope, Identifier, PolicyError, PublicAllowance, PublicGrant, QualifiedName,
        RequiredPrivilege,
    };

    #[test]
    fn required_policy_uses_relation_override_precedence_and_explicit_column_exceptions() {
        let relation = PublicObject::Relation(QualifiedName::new("service", "records").unwrap());
        let column = PublicObject::Column(
            QualifiedName::new("service", "records").unwrap(),
            Identifier::new("id").unwrap(),
        );
        let select = AllowedPrivilege::new(ObjectPrivilege::Select, false);
        let mut policy = AuthorityPolicy {
            discovery: DiscoveryScope::Schemas(vec![Identifier::new("service").unwrap()]),
            public_overrides: vec![PublicAllowance {
                object: relation.clone(),
                privileges: Vec::new(),
            }],
            required_privileges: vec![RequiredPrivilege {
                object: column.clone(),
                privilege: ObjectPrivilege::Select,
            }],
            ..AuthorityPolicy::default()
        };
        policy.defaults.columns.public_privileges = vec![select];
        assert_eq!(
            policy.validate(),
            Err(PolicyError::ContradictoryRequiredPrivilege)
        );
        policy.public_overrides.push(PublicAllowance {
            object: column.clone(),
            privileges: vec![select],
        });
        assert_eq!(policy.validate(), Ok(()));
        // A column deny cannot undo a deliberately permitted whole-table grant.
        policy.public_overrides[0].privileges = vec![select];
        policy.public_overrides[1].privileges.clear();
        assert_eq!(policy.validate(), Ok(()));
        // The older PublicGrant form has the same exact-parent precedence.
        policy.public_overrides.clear();
        policy.public_grants.push(PublicGrant {
            object: relation,
            privilege: AllowedPrivilege::new(ObjectPrivilege::Update, false),
        });
        assert_eq!(
            policy.validate(),
            Err(PolicyError::ContradictoryRequiredPrivilege)
        );
    }
}
