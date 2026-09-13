use super::super::super::policy::{
    AllowedPrivilege, AuthorityPolicy, ObjectPrivilege, PublicGrant, PublicObject, QualifiedName,
};
use std::collections::{HashMap, HashSet};

/// Index one already-expanded PUBLIC policy, keeping empty exact targets.
/// Built once per inspection phase, never once per inspected object or grant.
pub(in super::super) struct PublicPolicy {
    grants: HashMap<PublicObject, HashMap<ObjectPrivilege, bool>>,
    relations: HashSet<QualifiedName>,
    parameters: HashMap<String, HashMap<ObjectPrivilege, bool>>,
}

impl PublicPolicy {
    pub(in super::super) fn new(policy: &AuthorityPolicy) -> Self {
        let mut indexed = Self::from_grants(&policy.public_grants);
        for entry in &policy.public_overrides {
            indexed.select(&entry.object);
        }
        indexed
    }

    pub(in super::super) fn from_grants(grants: &[PublicGrant]) -> Self {
        let mut indexed = Self {
            grants: HashMap::new(),
            relations: HashSet::new(),
            parameters: HashMap::new(),
        };
        for grant in grants {
            indexed.add(grant.object.clone(), grant.privilege);
        }
        indexed
    }

    fn select(&mut self, object: &PublicObject) {
        if let PublicObject::Relation(name)
        | PublicObject::Column(name, _)
        | PublicObject::Sequence(name) = object
        {
            self.relations.insert(name.clone());
        }
        self.grants.entry(object.clone()).or_default();
    }

    pub(in super::super) fn add(&mut self, object: PublicObject, privilege: AllowedPrivilege) {
        self.select(&object);
        if let PublicObject::Parameter(name) = &object {
            self.parameters
                .entry(name.as_str().to_owned())
                .or_default()
                .entry(privilege.privilege)
                .and_modify(|option| *option |= privilege.grant_option)
                .or_insert(privilege.grant_option);
        }
        self.grants
            .entry(object)
            .or_default()
            .entry(privilege.privilege)
            .and_modify(|grant_option| *grant_option |= privilege.grant_option)
            .or_insert(privilege.grant_option);
    }

    pub(in super::super) fn allows_parameter(
        &self,
        name: &str,
        privilege: ObjectPrivilege,
        grant_option: bool,
    ) -> bool {
        self.parameters
            .get(name)
            .and_then(|grants| grants.get(&privilege))
            .is_some_and(|option| !grant_option || *option)
    }

    pub(in super::super) fn contains(&self, object: &PublicObject) -> bool {
        self.grants.contains_key(object)
    }

    pub(in super::super) fn selects_relation(&self, name: &QualifiedName) -> bool {
        self.relations.contains(name)
    }

    pub(in super::super) fn allows(
        &self,
        object: &PublicObject,
        privilege: ObjectPrivilege,
        grant_option: bool,
    ) -> bool {
        let allowed = |object| {
            self.grants
                .get(object)
                .and_then(|grants| grants.get(&privilege))
                .is_some_and(|allowed_option| !grant_option || *allowed_option)
        };
        allowed(object)
            || match object {
                PublicObject::Column(name, _) => allowed(&PublicObject::Relation(name.clone())),
                _ => false,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::policy::Identifier;

    #[test]
    fn indexed_public_policy_preserves_parent_column_allowance_and_grant_options() {
        let relation = QualifiedName::new("service", "records").unwrap();
        let column = PublicObject::Column(relation.clone(), Identifier::new("id").unwrap());
        let policy = AuthorityPolicy {
            public_grants: vec![
                PublicGrant {
                    object: PublicObject::Relation(relation.clone()),
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Select, false),
                },
                PublicGrant {
                    object: column.clone(),
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Update, true),
                },
                PublicGrant {
                    object: column.clone(),
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Update, false),
                },
            ],
            ..AuthorityPolicy::default()
        };
        let indexed = PublicPolicy::new(&policy);
        assert!(indexed.selects_relation(&relation));
        assert!(indexed.contains(&column));
        assert!(indexed.allows(&column, ObjectPrivilege::Select, false));
        assert!(!indexed.allows(&column, ObjectPrivilege::Select, true));
        assert!(indexed.allows(&column, ObjectPrivilege::Update, true));
        assert!(!indexed.allows(
            &PublicObject::Relation(relation),
            ObjectPrivilege::Update,
            false
        ));
    }
}
