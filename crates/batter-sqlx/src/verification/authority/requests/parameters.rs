use super::*;

pub(in super::super) async fn inspect_parameters(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    parameters: &ParameterIndex<'_>,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    allow_superuser: SuperuserAllowance,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let declared: HashMap<&str, &crate::verification::policy::ParameterPolicy> = policy
        .parameters
        .iter()
        .map(|parameter| (parameter.parameter.as_str(), parameter))
        .collect();
    let requested: HashSet<&str> = declared
        .keys()
        .copied()
        .chain(
            policy
                .required_privileges
                .iter()
                .map(|entry| &entry.object)
                .chain(policy.public_grants.iter().map(|entry| &entry.object))
                .chain(policy.public_overrides.iter().map(|entry| &entry.object))
                .filter_map(|object| match object {
                    PublicObject::Parameter(name) => Some(name.as_str()),
                    _ => None,
                }),
        )
        .collect();
    let public = PublicPolicy::new(policy);
    let required_parameters: HashSet<&str> = policy
        .required_privileges
        .iter()
        .filter_map(|required| match &required.object {
            PublicObject::Parameter(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    for parameter in snapshot.parameters.iter() {
        evaluation.checkpoint(findings).await?;
        // An unknown custom name is conservative potential authority, not
        // proof of a currently assignable parameter. Reserved prefixes and
        // hidden extension definitions cannot satisfy a positive requirement.
        if (!parameter.observable && requested.contains(parameter.name.as_str()))
            || (parameter.custom && required_parameters.contains(parameter.name.as_str()))
        {
            findings.push(Finding::new(
                FindingKind::ParameterUnobservable,
                Some(parameter.name.clone()),
                Some(snapshot.current_user.clone()),
                None,
            ));
        }
        let configured = declared.get(parameter.name.as_str()).copied();
        if configured.is_none() && parameter.acl.is_empty() {
            continue;
        }
        inspect_parameter(
            evaluation,
            parameter,
            configured,
            graph,
            allow_superuser,
            &public,
            findings,
        )
        .await?;
    }
    for parameter in &policy.parameters {
        evaluation.checkpoint(findings).await?;
        // Hidden metadata cannot prove absence, so only an observable known-
        // absent entry is a missing object.
        if !parameters
            .get(parameter.parameter.as_str())
            .is_some_and(|parameter| parameter.exists || !parameter.observable)
        {
            findings.push(Finding::new(
                FindingKind::MissingObject,
                Some(parameter.parameter.as_str().to_owned()),
                None::<String>,
                None,
            ));
        }
    }
    Ok(())
}

async fn inspect_parameter(
    evaluation: &mut Evaluation,
    parameter: &ParameterObject,
    configured: Option<&crate::verification::policy::ParameterPolicy>,
    graph: &RoleGraph<'_>,
    allow_superuser: SuperuserAllowance,
    public: &PublicPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let allow_superuser = allow_superuser == SuperuserAllowance::Allow;
    let allowed = configured.map_or(&[][..], |entry| entry.privileges.as_slice());
    let indexed_acl = IndexedAcl::new(&[parameter.acl.as_slice()]);
    // An explicit pg_parameter_acl row augments, rather than replaces,
    // PostgreSQL's context-wide capability.
    // Context defaults apply to active roles only. Compute them directly for
    // each source instead of rebuilding and rescanning a role-sized defaults list.
    for role in graph.capability_roles() {
        evaluation.checkpoint(findings).await?;
        if role.superuser && graph.is_active_role(role.oid) && allow_superuser {
            continue;
        }
        let mut actual = indexed_acl.actual_for(
            role.oid,
            None,
            graph,
            &[ObjectPrivilege::Set, ObjectPrivilege::AlterSystem],
        );
        if configured.is_some() && graph.is_active_role(role.oid) {
            let default_set = parameter.custom
                || matches!(parameter.context.as_deref(), Some("user" | "backend"))
                || (matches!(
                    parameter.context.as_deref(),
                    Some("superuser-backend" | "superuser")
                ) && role.superuser);
            for (privilege, granted, _) in &mut actual {
                *granted |= match privilege {
                    ObjectPrivilege::Set => default_set,
                    ObjectPrivilege::AlterSystem => role.superuser,
                    _ => false,
                };
            }
        }
        for (privilege, granted, grant_option) in actual {
            evaluation.checkpoint(findings).await?;
            if granted && !allows(allowed, privilege, GrantAuthority::PrivilegeOnly) {
                findings.push(Finding::new(
                    FindingKind::ProtectedParameter,
                    Some(parameter.name.clone()),
                    Some(role.name.clone()),
                    Some(privilege),
                ));
            }
            if grant_option && !allows(allowed, privilege, GrantAuthority::Grantable) {
                findings.push(Finding::new(
                    FindingKind::GrantOption,
                    Some(parameter.name.clone()),
                    Some(role.name.clone()),
                    Some(privilege),
                ));
            }
        }
    }
    inspect_public_parameter(parameter, &indexed_acl, public, findings);

    Ok(())
}

fn inspect_public_parameter(
    parameter: &ParameterObject,
    indexed_acl: &IndexedAcl,
    public: &PublicPolicy,
    findings: &mut Vec<Finding>,
) {
    for (privilege, granted, grant_option) in
        indexed_acl.public(&[ObjectPrivilege::Set, ObjectPrivilege::AlterSystem])
    {
        if granted
            && !public.allows_parameter(&parameter.name, privilege, GrantAuthority::PrivilegeOnly)
        {
            findings.push(Finding::new(
                FindingKind::ProtectedParameter,
                Some(parameter.name.clone()),
                Some("public".to_owned()),
                Some(privilege),
            ));
        }
        if grant_option
            && !public.allows_parameter(&parameter.name, privilege, GrantAuthority::Grantable)
        {
            findings.push(Finding::new(
                FindingKind::GrantOption,
                Some(parameter.name.clone()),
                Some("public".to_owned()),
                Some(privilege),
            ));
        }
    }
}
