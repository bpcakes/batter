use super::{AuthorityPolicy, DiscoveryScope, PolicyError, PublicObject};

/// Bound validation and default expansion before either walks caller data.
/// The entry budget includes nested columns, arguments and allowance entries.
pub(super) fn validate(policy: &AuthorityPolicy) -> Result<(), PolicyError> {
    let mut remaining = 10_000usize;
    let mut charge = |count: usize| {
        remaining = remaining
            .checked_sub(count)
            .ok_or(PolicyError::AuthorityCapacity)?;
        Ok::<_, PolicyError>(())
    };
    for count in [
        policy.relations.len(),
        policy.sequences.len(),
        policy.schemas.len(),
        policy.types.len(),
        policy.routines.len(),
        policy.parameters.len(),
        policy.public_grants.len(),
        policy.public_overrides.len(),
        policy.required_privileges.len(),
        policy.required_surfaces.len(),
        policy.roles.allowed_admin_roles.len(),
        policy.roles.allowed_predefined_roles.len(),
    ] {
        charge(count)?;
    }
    if let DiscoveryScope::Schemas(names) = &policy.discovery {
        charge(names.len())?;
    }
    for relation in &policy.relations {
        charge(relation.columns.len())?;
    }
    for routine in policy.routines.iter().map(|entry| &entry.routine).chain(
        policy
            .public_grants
            .iter()
            .map(|entry| &entry.object)
            .chain(policy.public_overrides.iter().map(|entry| &entry.object))
            .chain(policy.required_privileges.iter().map(|entry| &entry.object))
            .filter_map(|object| {
                if let PublicObject::Routine(routine) = object {
                    Some(routine)
                } else {
                    None
                }
            }),
    ) {
        charge(routine.arguments().len())?;
    }
    let defaults = &policy.defaults;
    let default_lists = [
        &defaults.relations,
        &defaults.columns,
        &defaults.sequences,
        &defaults.schemas,
        &defaults.types,
        &defaults.invoker_routines,
        &defaults.definer_routines,
    ];
    for privileges in policy
        .relations
        .iter()
        .map(|entry| &entry.privileges)
        .chain(
            policy
                .relations
                .iter()
                .flat_map(|entry| entry.columns.iter().map(|column| &column.privileges)),
        )
        .chain(policy.sequences.iter().map(|entry| &entry.privileges))
        .chain(policy.schemas.iter().map(|entry| &entry.privileges))
        .chain(policy.types.iter().map(|entry| &entry.privileges))
        .chain(policy.routines.iter().map(|entry| &entry.privileges))
        .chain(policy.parameters.iter().map(|entry| &entry.privileges))
        .chain(
            policy
                .public_overrides
                .iter()
                .map(|entry| &entry.privileges),
        )
        .chain(std::iter::once(&policy.database.privileges))
        .chain(
            default_lists
                .into_iter()
                .flat_map(|entry| [&entry.privileges, &entry.public_privileges]),
        )
    {
        // There are only fourteen privilege kinds. This also prevents a repeated
        // allowance list from multiplying into every discovered object's policy.
        if privileges.len() > 32 {
            return Err(PolicyError::AuthorityCapacity);
        }
        charge(privileges.len())?;
    }
    Ok(())
}
