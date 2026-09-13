use super::*;
use crate::verification::{
    AllowedPrivilege, ColumnPolicy, DatabasePolicy, PublicAllowance, PublicObject, RelationPolicy,
    RequiredPrivilege, RoutinePolicy, SchemaPolicy,
};
use std::{cmp::Ordering, collections::HashMap};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetOptions {
    Database {
        allow_owner: bool,
    },
    Schema {
        allow_owner: bool,
    },
    Relation {
        allow_owner: bool,
        allow_row_type_public_usage: bool,
    },
    Column,
    Routine {
        allow_owner: bool,
        allow_security_definer: bool,
    },
}

struct Declaration {
    options: TargetOptions,
    privileges: HashMap<ObjectPrivilege, DeclarationOptions>,
}

pub(super) fn compile(mut manifest: ExactRoleManifest) -> Result<CompiledExactRole, ManifestError> {
    manifest.ensure_input_capacity(0)?;
    normalize_discovery(&mut manifest.discovery);
    normalize_defaults(&mut manifest.defaults)?;
    normalize_roles(&mut manifest.roles);
    ensure_expansion_capacity(&manifest)?;

    let mut declarations = HashMap::new();
    collect_database_declarations(&mut declarations, std::mem::take(&mut manifest.databases))?;
    collect_schema_declarations(&mut declarations, std::mem::take(&mut manifest.schemas))?;
    collect_relation_declarations(&mut declarations, std::mem::take(&mut manifest.relations))?;
    collect_column_declarations(&mut declarations, std::mem::take(&mut manifest.columns))?;
    collect_routine_declarations(&mut declarations, std::mem::take(&mut manifest.routines))?;

    let mut entries = declarations.into_iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| compare_targets(left, right));
    ensure_generated_policy_capacity(&manifest, &entries)?;
    build_compiled(manifest, entries)
}

fn collect_database_declarations(
    declarations: &mut HashMap<GrantTarget, Declaration>,
    specs: Vec<DatabaseGrantSpec>,
) -> Result<(), ManifestError> {
    for spec in specs {
        insert_declaration(
            declarations,
            GrantTarget::Database,
            TargetOptions::Database {
                allow_owner: spec.allow_owner,
            },
            spec.privileges,
            spec.options,
        )?;
    }
    Ok(())
}

fn collect_schema_declarations(
    declarations: &mut HashMap<GrantTarget, Declaration>,
    specs: Vec<SchemaGrantSpec>,
) -> Result<(), ManifestError> {
    for spec in specs {
        insert_declaration(
            declarations,
            GrantTarget::Schema(spec.schema),
            TargetOptions::Schema {
                allow_owner: spec.allow_owner,
            },
            spec.privileges,
            spec.options,
        )?;
    }
    Ok(())
}

fn collect_relation_declarations(
    declarations: &mut HashMap<GrantTarget, Declaration>,
    groups: Vec<RelationGrantGroup>,
) -> Result<(), ManifestError> {
    for group in groups {
        for relation in group.relations {
            insert_declaration(
                declarations,
                GrantTarget::Relation(relation),
                TargetOptions::Relation {
                    allow_owner: group.allow_owner,
                    allow_row_type_public_usage: group.allow_row_type_public_usage,
                },
                group.privileges.clone(),
                group.options,
            )?;
        }
    }
    Ok(())
}

fn collect_column_declarations(
    declarations: &mut HashMap<GrantTarget, Declaration>,
    groups: Vec<ColumnGrantGroup>,
) -> Result<(), ManifestError> {
    for group in groups {
        let parent = declarations.get(&GrantTarget::Relation(group.relation.clone()));
        let Some(parent) = parent else {
            return Err(ManifestError::MissingRelationDeclaration);
        };
        if group.options.public_delivery == PublicDelivery::Deny
            && group.privileges.iter().any(|privilege| {
                parent
                    .privileges
                    .get(privilege)
                    .is_some_and(|options| options.public_delivery == PublicDelivery::AllowDeclared)
            })
        {
            return Err(ManifestError::ContradictoryDeclaration);
        }
        for column in group.columns {
            insert_declaration(
                declarations,
                GrantTarget::Column(group.relation.clone(), column),
                TargetOptions::Column,
                group.privileges.clone(),
                group.options,
            )?;
        }
    }
    Ok(())
}

fn collect_routine_declarations(
    declarations: &mut HashMap<GrantTarget, Declaration>,
    specs: Vec<RoutineGrantSpec>,
) -> Result<(), ManifestError> {
    for spec in specs {
        insert_declaration(
            declarations,
            GrantTarget::Routine(spec.routine),
            TargetOptions::Routine {
                allow_owner: spec.allow_owner,
                allow_security_definer: spec.allow_security_definer,
            },
            spec.privileges,
            spec.options,
        )?;
    }
    Ok(())
}

fn insert_declaration(
    declarations: &mut HashMap<GrantTarget, Declaration>,
    target: GrantTarget,
    target_options: TargetOptions,
    privileges: Vec<ObjectPrivilege>,
    options: DeclarationOptions,
) -> Result<(), ManifestError> {
    let declaration = declarations
        .entry(target.clone())
        .or_insert_with(|| Declaration {
            options: target_options,
            privileges: HashMap::new(),
        });
    if declaration.options != target_options {
        return Err(ManifestError::ContradictoryDeclaration);
    }
    for privilege in privileges {
        if !valid_privilege(&target, privilege) {
            return Err(ManifestError::InvalidObjectPrivilege);
        }
        if let Some(previous) = declaration.privileges.insert(privilege, options)
            && previous != options
        {
            return Err(ManifestError::ContradictoryDeclaration);
        }
    }
    Ok(())
}

fn build_compiled(
    manifest: ExactRoleManifest,
    entries: Vec<(GrantTarget, Declaration)>,
) -> Result<CompiledExactRole, ManifestError> {
    let mut policy = AuthorityPolicy {
        discovery: manifest.discovery,
        defaults: manifest.defaults,
        roles: manifest.roles,
        ..AuthorityPolicy::default()
    };
    let mut grants = Vec::new();
    let mut relation_indices: HashMap<QualifiedName, usize> = HashMap::new();

    for (target, declaration) in entries {
        let privileges = normalized_privileges(&declaration);
        add_public_overrides(&mut policy, &target, &declaration);
        add_requirements_and_grants(&mut policy, &mut grants, &target, &declaration);
        add_object_policy(
            &mut policy,
            &mut relation_indices,
            target,
            declaration.options,
            privileges,
        );
    }
    sort_relation_columns(&mut policy.relations);
    policy.validate()?;
    Ok(CompiledExactRole {
        primary_schema: manifest.primary_schema,
        authority_policy: policy,
        grant_plan: GrantPlan { atoms: grants },
    })
}

fn add_public_overrides(
    policy: &mut AuthorityPolicy,
    target: &GrantTarget,
    declaration: &Declaration,
) {
    let privileges = sorted_atoms(declaration)
        .filter(|(_, options)| options.public_delivery == PublicDelivery::AllowDeclared)
        .map(|(privilege, _)| AllowedPrivilege::new(privilege, false))
        .collect();
    policy.public_overrides.push(PublicAllowance {
        object: public_object(target),
        privileges,
    });
    if let (
        GrantTarget::Relation(relation),
        TargetOptions::Relation {
            allow_row_type_public_usage,
            ..
        },
    ) = (target, declaration.options)
    {
        policy.public_overrides.push(PublicAllowance {
            object: PublicObject::Type(relation.clone()),
            privileges: if allow_row_type_public_usage {
                vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)]
            } else {
                Vec::new()
            },
        });
    }
}

fn add_requirements_and_grants(
    policy: &mut AuthorityPolicy,
    grants: &mut Vec<GrantAtom>,
    target: &GrantTarget,
    declaration: &Declaration,
) {
    for (privilege, options) in sorted_atoms(declaration) {
        if options.purpose == DeclarationPurpose::RequiredAndProvisioned {
            policy.required_privileges.push(RequiredPrivilege {
                object: public_object(target),
                privilege,
            });
            grants.push(GrantAtom {
                target: target.clone(),
                privilege,
            });
        }
    }
}

fn add_object_policy(
    policy: &mut AuthorityPolicy,
    relation_indices: &mut HashMap<QualifiedName, usize>,
    target: GrantTarget,
    options: TargetOptions,
    privileges: Vec<AllowedPrivilege>,
) {
    match (target, options) {
        (GrantTarget::Database, TargetOptions::Database { allow_owner }) => {
            policy.database = DatabasePolicy {
                privileges,
                allow_owner,
            };
        }
        (GrantTarget::Schema(schema), TargetOptions::Schema { allow_owner }) => {
            policy.schemas.push(SchemaPolicy {
                schema,
                privileges,
                allow_owner,
            });
        }
        (
            GrantTarget::Relation(relation),
            TargetOptions::Relation {
                allow_owner,
                allow_row_type_public_usage,
            },
        ) => {
            let index = relation_entry(policy, relation_indices, relation);
            let entry = &mut policy.relations[index];
            entry.privileges = privileges;
            entry.allow_owner = allow_owner;
            entry.allow_row_type_public_usage = allow_row_type_public_usage;
        }
        (GrantTarget::Column(relation, column), TargetOptions::Column) => {
            let index = relation_entry(policy, relation_indices, relation);
            policy.relations[index]
                .columns
                .push(ColumnPolicy { column, privileges });
        }
        (
            GrantTarget::Routine(routine),
            TargetOptions::Routine {
                allow_owner,
                allow_security_definer,
            },
        ) => policy.routines.push(RoutinePolicy {
            routine,
            privileges,
            allow_owner,
            allow_security_definer,
        }),
        _ => unreachable!("grant target and options are constructed together"),
    }
}

fn relation_entry(
    policy: &mut AuthorityPolicy,
    indices: &mut HashMap<QualifiedName, usize>,
    relation: QualifiedName,
) -> usize {
    if let Some(index) = indices.get(&relation) {
        return *index;
    }
    let index = policy.relations.len();
    indices.insert(relation.clone(), index);
    policy.relations.push(RelationPolicy {
        relation,
        privileges: Vec::new(),
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    });
    index
}

fn sort_relation_columns(relations: &mut [RelationPolicy]) {
    relations.sort_by(|left, right| compare_names(&left.relation, &right.relation));
    for relation in relations {
        relation
            .columns
            .sort_by(|left, right| left.column.as_str().cmp(right.column.as_str()));
    }
}

fn normalized_privileges(declaration: &Declaration) -> Vec<AllowedPrivilege> {
    sorted_atoms(declaration)
        .map(|(privilege, options)| AllowedPrivilege::new(privilege, options.allow_grant_option))
        .collect()
}

fn sorted_atoms(
    declaration: &Declaration,
) -> impl Iterator<Item = (ObjectPrivilege, DeclarationOptions)> + '_ {
    let mut atoms = declaration
        .privileges
        .iter()
        .map(|(privilege, options)| (*privilege, *options))
        .collect::<Vec<_>>();
    atoms.sort_by_key(|(privilege, _)| privilege_rank(*privilege));
    atoms.into_iter()
}

fn public_object(target: &GrantTarget) -> PublicObject {
    match target {
        GrantTarget::Database => PublicObject::Database,
        GrantTarget::Schema(schema) => PublicObject::Schema(schema.clone()),
        GrantTarget::Relation(relation) => PublicObject::Relation(relation.clone()),
        GrantTarget::Column(relation, column) => {
            PublicObject::Column(relation.clone(), column.clone())
        }
        GrantTarget::Routine(routine) => PublicObject::Routine(routine.clone()),
    }
}

fn normalize_discovery(discovery: &mut DiscoveryScope) {
    if let DiscoveryScope::Schemas(schemas) = discovery {
        schemas.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        schemas.dedup();
    }
}

fn normalize_roles(roles: &mut RolePolicy) {
    for names in [
        &mut roles.allowed_admin_roles,
        &mut roles.allowed_predefined_roles,
    ] {
        names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        names.dedup();
    }
}

fn normalize_defaults(defaults: &mut DiscoveryDefaults) -> Result<(), ManifestError> {
    for (kind, entry) in [
        (ObjectKind::Schema, &mut defaults.schemas),
        (ObjectKind::Relation, &mut defaults.relations),
        (ObjectKind::Column, &mut defaults.columns),
        (ObjectKind::Sequence, &mut defaults.sequences),
        (ObjectKind::Type, &mut defaults.types),
        (ObjectKind::Routine, &mut defaults.invoker_routines),
        (ObjectKind::Routine, &mut defaults.definer_routines),
    ] {
        normalize_allowances(kind, &mut entry.privileges)?;
        normalize_allowances(kind, &mut entry.public_privileges)?;
    }
    Ok(())
}

fn normalize_allowances(
    kind: ObjectKind,
    privileges: &mut Vec<AllowedPrivilege>,
) -> Result<(), ManifestError> {
    privileges.sort_by_key(|entry| privilege_rank(entry.privilege));
    let mut normalized: Vec<AllowedPrivilege> = Vec::with_capacity(privileges.len());
    for entry in privileges.drain(..) {
        if !valid_kind_privilege(kind, entry.privilege) {
            return Err(ManifestError::InvalidObjectPrivilege);
        }
        if let Some(previous) = normalized.last()
            && previous.privilege == entry.privilege
        {
            if *previous != entry {
                return Err(ManifestError::ContradictoryDeclaration);
            }
            continue;
        }
        normalized.push(entry);
    }
    *privileges = normalized;
    Ok(())
}

#[derive(Clone, Copy)]
enum ObjectKind {
    Database,
    Schema,
    Relation,
    Column,
    Sequence,
    Type,
    Routine,
}

fn valid_privilege(target: &GrantTarget, privilege: ObjectPrivilege) -> bool {
    let kind = match target {
        GrantTarget::Database => ObjectKind::Database,
        GrantTarget::Schema(_) => ObjectKind::Schema,
        GrantTarget::Relation(_) => ObjectKind::Relation,
        GrantTarget::Column(..) => ObjectKind::Column,
        GrantTarget::Routine(_) => ObjectKind::Routine,
    };
    valid_kind_privilege(kind, privilege)
}

fn valid_kind_privilege(kind: ObjectKind, privilege: ObjectPrivilege) -> bool {
    use ObjectPrivilege::{
        Connect, Create, Delete, Execute, Insert, Maintain, References, Select, Temporary, Trigger,
        Truncate, Update, Usage,
    };
    match kind {
        ObjectKind::Database => matches!(privilege, Connect | Create | Temporary),
        ObjectKind::Schema => matches!(privilege, Usage | Create),
        ObjectKind::Relation => matches!(
            privilege,
            Select | Insert | Update | Delete | Truncate | References | Trigger | Maintain
        ),
        ObjectKind::Column => matches!(privilege, Select | Insert | Update | References),
        ObjectKind::Sequence => matches!(privilege, Usage | Select | Update),
        ObjectKind::Type => privilege == Usage,
        ObjectKind::Routine => privilege == Execute,
    }
}

fn ensure_expansion_capacity(manifest: &ExactRoleManifest) -> Result<(), ManifestError> {
    let mut expanded = 0usize;
    for count in manifest
        .databases
        .iter()
        .map(|spec| spec.privileges.len().max(1))
        .chain(
            manifest
                .schemas
                .iter()
                .map(|spec| spec.privileges.len().max(1)),
        )
        .chain(manifest.relations.iter().map(|group| {
            group
                .relations
                .len()
                .saturating_mul(group.privileges.len().max(1))
        }))
        .chain(manifest.columns.iter().map(|group| {
            group
                .columns
                .len()
                .saturating_mul(group.privileges.len().max(1))
        }))
        .chain(
            manifest
                .routines
                .iter()
                .map(|spec| spec.privileges.len().max(1)),
        )
    {
        expanded = checked_add(expanded, count)?;
    }
    Ok(())
}

fn ensure_generated_policy_capacity(
    manifest: &ExactRoleManifest,
    entries: &[(GrantTarget, Declaration)],
) -> Result<(), ManifestError> {
    let discovery = match &manifest.discovery {
        DiscoveryScope::Schemas(schemas) => schemas.len(),
        DiscoveryScope::Declared | DiscoveryScope::UserSchemas => 0,
    };
    let mut total = defaults_input_size(&manifest.defaults)?;
    for count in [
        discovery,
        manifest.roles.allowed_admin_roles.len(),
        manifest.roles.allowed_predefined_roles.len(),
    ] {
        total = checked_add(total, count)?;
    }
    for (target, declaration) in entries {
        let privileges = declaration.privileges.len();
        let required = declaration
            .privileges
            .values()
            .filter(|options| options.purpose == DeclarationPurpose::RequiredAndProvisioned)
            .count();
        let public = declaration
            .privileges
            .values()
            .filter(|options| options.public_delivery == PublicDelivery::AllowDeclared)
            .count();
        let object = usize::from(!matches!(target, GrantTarget::Database));
        let (row_type_override, row_type_public) = match (target, declaration.options) {
            (
                GrantTarget::Relation(_),
                TargetOptions::Relation {
                    allow_row_type_public_usage,
                    ..
                },
            ) => (1, usize::from(allow_row_type_public_usage)),
            _ => (0, 0),
        };
        for count in [
            object,
            1,
            privileges,
            required,
            public,
            row_type_override,
            row_type_public,
        ] {
            total = checked_add(total, count)?;
        }
        if let GrantTarget::Routine(routine) = target {
            let copies = required
                .checked_add(2)
                .ok_or(ManifestError::Policy(PolicyError::AuthorityCapacity))?;
            let arguments = routine
                .arguments()
                .len()
                .checked_mul(copies)
                .ok_or(ManifestError::Policy(PolicyError::AuthorityCapacity))?;
            total = checked_add(total, arguments)?;
        }
    }
    Ok(())
}

pub(super) fn compare_targets(left: &GrantTarget, right: &GrantTarget) -> Ordering {
    target_rank(left)
        .cmp(&target_rank(right))
        .then_with(|| match (left, right) {
            (GrantTarget::Database, GrantTarget::Database) => Ordering::Equal,
            (GrantTarget::Schema(left), GrantTarget::Schema(right)) => {
                left.as_str().cmp(right.as_str())
            }
            (GrantTarget::Relation(left), GrantTarget::Relation(right)) => {
                compare_names(left, right)
            }
            (
                GrantTarget::Column(left_name, left_column),
                GrantTarget::Column(right_name, right_column),
            ) => compare_names(left_name, right_name)
                .then_with(|| left_column.as_str().cmp(right_column.as_str())),
            (GrantTarget::Routine(left), GrantTarget::Routine(right)) => {
                compare_routines(left, right)
            }
            _ => Ordering::Equal,
        })
}

fn target_rank(target: &GrantTarget) -> u8 {
    match target {
        GrantTarget::Database => 0,
        GrantTarget::Schema(_) => 1,
        GrantTarget::Relation(_) => 2,
        GrantTarget::Column(..) => 3,
        GrantTarget::Routine(_) => 4,
    }
}

fn compare_names(left: &QualifiedName, right: &QualifiedName) -> Ordering {
    left.schema()
        .cmp(right.schema())
        .then_with(|| left.name().cmp(right.name()))
}

fn compare_routines(left: &RoutineSignature, right: &RoutineSignature) -> Ordering {
    left.schema()
        .cmp(right.schema())
        .then_with(|| left.name().cmp(right.name()))
        .then_with(|| compare_arguments(left, right))
}

fn compare_arguments(left: &RoutineSignature, right: &RoutineSignature) -> Ordering {
    for (left, right) in left.arguments().iter().zip(right.arguments()) {
        let ordering = left
            .schema()
            .cmp(right.schema())
            .then_with(|| left.name().cmp(right.name()))
            .then_with(|| left.is_array().cmp(&right.is_array()));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.arguments().len().cmp(&right.arguments().len())
}

pub(super) const fn privilege_rank(privilege: ObjectPrivilege) -> u8 {
    match privilege {
        ObjectPrivilege::Select => 0,
        ObjectPrivilege::Insert => 1,
        ObjectPrivilege::Update => 2,
        ObjectPrivilege::Delete => 3,
        ObjectPrivilege::Truncate => 4,
        ObjectPrivilege::References => 5,
        ObjectPrivilege::Trigger => 6,
        ObjectPrivilege::Maintain => 7,
        ObjectPrivilege::Usage => 8,
        ObjectPrivilege::Create => 9,
        ObjectPrivilege::Connect => 10,
        ObjectPrivilege::Temporary => 11,
        ObjectPrivilege::Execute => 12,
        ObjectPrivilege::Set => 13,
        ObjectPrivilege::AlterSystem => 14,
    }
}
