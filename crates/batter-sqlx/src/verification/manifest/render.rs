use super::*;
use std::fmt::{self, Write};

/// Invalid context for pure grant-plan rendering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantRenderError {
    /// The role is a PostgreSQL special or reserved role target rather than an
    /// application-owned exact role. Matching is ASCII case-insensitive so a
    /// quoted case variant cannot bypass this high-level boundary.
    InvalidRoleTarget,
    /// The plan contains a database grant but no database identifier was given.
    MissingDatabaseContext,
    /// A database identifier was supplied to a plan with no database grant.
    UnexpectedDatabaseContext,
}

impl fmt::Display for GrantRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRoleTarget => "invalid exact-role grant target",
            Self::MissingDatabaseContext => "database grant requires explicit rendering context",
            Self::UnexpectedDatabaseContext => {
                "database rendering context supplied without a database grant"
            }
        })
    }
}

impl std::error::Error for GrantRenderError {}

pub(super) fn render(
    plan: &GrantPlan,
    role: &Identifier,
    database: Option<&Identifier>,
) -> Result<String, GrantRenderError> {
    if invalid_role_target(role) {
        return Err(GrantRenderError::InvalidRoleTarget);
    }
    match (plan.requires_database_context(), database) {
        (true, None) => return Err(GrantRenderError::MissingDatabaseContext),
        (false, Some(_)) => return Err(GrantRenderError::UnexpectedDatabaseContext),
        _ => {}
    }
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < plan.atoms.len() {
        let next = target_end(&plan.atoms, cursor);
        render_target(&mut output, &plan.atoms[cursor..next], role, database);
        cursor = next;
    }
    Ok(output)
}

fn target_end(atoms: &[GrantAtom], start: usize) -> usize {
    let mut end = start + 1;
    while end < atoms.len() && atoms[end].target == atoms[start].target {
        end += 1;
    }
    end
}

fn render_target(
    output: &mut String,
    atoms: &[GrantAtom],
    role: &Identifier,
    database: Option<&Identifier>,
) {
    let privileges = atoms
        .iter()
        .map(|atom| privilege_sql(atom.privilege))
        .collect::<Vec<_>>()
        .join(", ");
    let target = match &atoms[0].target {
        GrantTarget::Database => format!(
            "DATABASE {}",
            database.expect("render context was prevalidated").quoted()
        ),
        GrantTarget::Schema(schema) => format!("SCHEMA {}", schema.quoted()),
        GrantTarget::Relation(relation) => format!("TABLE {}", relation.quoted()),
        GrantTarget::Column(relation, column) => {
            let column_privileges = atoms
                .iter()
                .map(|atom| format!("{} ({})", privilege_sql(atom.privilege), column.quoted()))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                output,
                "GRANT {column_privileges} ON TABLE {} TO {};",
                relation.quoted(),
                role.quoted()
            )
            .expect("writing to a String cannot fail");
            return;
        }
        GrantTarget::Routine(routine) => format!("ROUTINE {}", routine.quoted()),
    };
    writeln!(
        output,
        "GRANT {privileges} ON {target} TO {};",
        role.quoted()
    )
    .expect("writing to a String cannot fail");
}

fn invalid_role_target(role: &Identifier) -> bool {
    let normalized = role.as_str().to_ascii_lowercase();
    matches!(normalized.as_str(), "public" | "none") || normalized.starts_with("pg_")
}

const fn privilege_sql(privilege: ObjectPrivilege) -> &'static str {
    match privilege {
        ObjectPrivilege::Select => "SELECT",
        ObjectPrivilege::Insert => "INSERT",
        ObjectPrivilege::Update => "UPDATE",
        ObjectPrivilege::Delete => "DELETE",
        ObjectPrivilege::Truncate => "TRUNCATE",
        ObjectPrivilege::References => "REFERENCES",
        ObjectPrivilege::Trigger => "TRIGGER",
        ObjectPrivilege::Maintain => "MAINTAIN",
        ObjectPrivilege::Usage => "USAGE",
        ObjectPrivilege::Create => "CREATE",
        ObjectPrivilege::Connect => "CONNECT",
        ObjectPrivilege::Temporary => "TEMPORARY",
        ObjectPrivilege::Execute => "EXECUTE",
        ObjectPrivilege::Set => "SET",
        ObjectPrivilege::AlterSystem => "ALTER SYSTEM",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_privilege_has_its_exact_keyword() {
        use ObjectPrivilege::*;
        assert_eq!(
            [
                Select,
                Insert,
                Update,
                Delete,
                Truncate,
                References,
                Trigger,
                Maintain,
                Usage,
                Create,
                Connect,
                Temporary,
                Execute,
                Set,
                AlterSystem,
            ]
            .map(privilege_sql),
            [
                "SELECT",
                "INSERT",
                "UPDATE",
                "DELETE",
                "TRUNCATE",
                "REFERENCES",
                "TRIGGER",
                "MAINTAIN",
                "USAGE",
                "CREATE",
                "CONNECT",
                "TEMPORARY",
                "EXECUTE",
                "SET",
                "ALTER SYSTEM",
            ]
        );
    }
}
