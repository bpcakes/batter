use super::{Names, Result, exec, exec_secret, grant, grant_column, literal, object_kind, quote};
use sqlx::PgConnection;

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub(super) async fn provision(
    connection: &mut PgConnection,
    names: &Names,
    password: &str,
) -> Result {
    for role in [
        &names.login_a,
        &names.login_b,
        &names.login_noinherit,
        &names.inherited,
        &names.nested,
        &names.settable,
        &names.admin_target,
        &names.unrelated_creator,
        &names.owner,
    ] {
        exec(
            &mut *connection,
            format!(
                "CREATE ROLE {} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS",
                quote(role)
            ),
        )
        .await?;
    }
    exec_secret(
        &mut *connection,
        format!(
            "ALTER ROLE {} LOGIN INHERIT PASSWORD {} VALID UNTIL 'tomorrow'",
            quote(&names.login_a),
            literal(password),
        ),
        password,
    )
    .await?;
    exec_secret(
        &mut *connection,
        format!(
            "ALTER ROLE {} LOGIN NOINHERIT PASSWORD {} VALID UNTIL 'tomorrow'",
            quote(&names.login_noinherit),
            literal(password),
        ),
        password,
    )
    .await?;
    exec_secret(
        &mut *connection,
        format!(
            "ALTER ROLE {} LOGIN NOINHERIT CREATEROLE PASSWORD {} VALID UNTIL 'tomorrow'",
            quote(&names.unrelated_creator),
            literal(password),
        ),
        password,
    )
    .await?;
    exec_secret(
        &mut *connection,
        format!(
            "ALTER ROLE {} LOGIN INHERIT PASSWORD {} VALID UNTIL 'tomorrow'",
            quote(&names.login_b),
            literal(password),
        ),
        password,
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT TRUE, SET FALSE, ADMIN FALSE",
            quote(&names.inherited),
            quote(&names.login_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT FALSE, SET TRUE, ADMIN FALSE",
            quote(&names.settable),
            quote(&names.login_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT FALSE, SET TRUE, ADMIN TRUE",
            quote(&names.admin_target),
            quote(&names.login_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT TRUE, SET FALSE, ADMIN FALSE",
            quote(&names.nested),
            quote(&names.login_noinherit),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT FALSE, SET FALSE, ADMIN FALSE",
            quote(&names.settable),
            quote(&names.login_noinherit),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT FALSE, SET FALSE, ADMIN TRUE",
            quote(&names.admin_target),
            quote(&names.settable),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT {} TO {} WITH INHERIT FALSE, SET FALSE, ADMIN FALSE",
            quote(&names.settable),
            quote(&names.unrelated_creator),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT pg_write_all_data TO {} WITH INHERIT TRUE, SET FALSE, ADMIN FALSE",
            quote(&names.nested),
        ),
    )
    .await?;
    for schema in [&names.schema_a, &names.schema_b] {
        exec(
            &mut *connection,
            format!(
                "CREATE SCHEMA {} AUTHORIZATION {}",
                quote(schema),
                quote(&names.owner),
            ),
        )
        .await?;
        exec(
            &mut *connection,
            format!("REVOKE ALL ON SCHEMA {} FROM PUBLIC", quote(schema)),
        )
        .await?;
    }

    exec(
        &mut *connection,
        format!(
            "CREATE TABLE {}.{} (version BIGINT NOT NULL PRIMARY KEY, checksum BYTEA NOT NULL, success BOOLEAN NOT NULL)",
            quote(&names.schema_a),
            quote(&names.ledger_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE TABLE {}.{} (version BIGINT NOT NULL PRIMARY KEY, checksum BYTEA NOT NULL, success BOOLEAN NOT NULL)",
            quote(&names.schema_b),
            quote(&names.ledger_b),
        ),
    )
    .await?;
    for (schema, ledger) in [
        (&names.schema_a, &names.ledger_a),
        (&names.schema_b, &names.ledger_b),
    ] {
        exec(
            &mut *connection,
            format!(
                "INSERT INTO {}.{} VALUES (1, '\\x01', true)",
                quote(schema),
                quote(ledger),
            ),
        )
        .await?;
    }
    exec(
        &mut *connection,
        format!(
            "CREATE TABLE {}.{} (id BIGINT NOT NULL, visible TEXT NOT NULL, secret TEXT NOT NULL)",
            quote(&names.schema_a),
            quote(&names.table_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE TABLE {}.{} (id BIGINT NOT NULL, visible TEXT NOT NULL)",
            quote(&names.schema_b),
            quote(&names.table_b),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE SEQUENCE {}.{}",
            quote(&names.schema_a),
            quote(&names.sequence_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE SEQUENCE {}.{}",
            quote(&names.schema_a),
            quote(&names.sequence_default),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE TYPE {}.{} AS ENUM ('one', 'two')",
            quote(&names.schema_a),
            quote(&names.type_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE FUNCTION {}.{}(id integer) RETURNS integer LANGUAGE SQL IMMUTABLE AS 'SELECT $1'",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE FUNCTION {}.{}(items integer[][]) RETURNS integer LANGUAGE SQL IMMUTABLE AS 'SELECT COALESCE(array_length($1, 1), 0)'",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "CREATE FUNCTION {}.{}(value name) RETURNS text LANGUAGE SQL IMMUTABLE AS 'SELECT $1::text'",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    for argument in ["oidvector", "oid[]"] {
        exec(
            &mut *connection,
            format!(
                "CREATE FUNCTION {}.{}(value {argument}) RETURNS integer LANGUAGE SQL IMMUTABLE AS 'SELECT 0'",
                quote(&names.schema_a),
                quote(&names.routine_a),
            ),
        )
        .await?;
    }
    exec(
        &mut *connection,
        format!(
            "ALTER FUNCTION {}.{}(integer) OWNER TO {}",
            quote(&names.schema_a),
            quote(&names.routine_a),
            quote(&names.owner),
        ),
    )
    .await?;
    for argument in ["integer[]", "name", "oidvector", "oid[]"] {
        exec(
            &mut *connection,
            format!(
                "ALTER FUNCTION {}.{}({argument}) OWNER TO {}",
                quote(&names.schema_a),
                quote(&names.routine_a),
                quote(&names.owner),
            ),
        )
        .await?;
    }
    exec(
        &mut *connection,
        format!(
            "REVOKE ALL ON FUNCTION {}.{}(integer) FROM PUBLIC",
            quote(&names.schema_a),
            quote(&names.routine_a),
        ),
    )
    .await?;
    for argument in ["integer[]", "name", "oidvector", "oid[]"] {
        exec(
            &mut *connection,
            format!(
                "REVOKE ALL ON FUNCTION {}.{}({argument}) FROM PUBLIC",
                quote(&names.schema_a),
                quote(&names.routine_a),
            ),
        )
        .await?;
    }
    for object in [
        format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a)),
        format!("{}.{}", quote(&names.schema_a), quote(&names.table_a)),
        format!("{}.{}", quote(&names.schema_a), quote(&names.sequence_a)),
        format!(
            "{}.{}",
            quote(&names.schema_a),
            quote(&names.sequence_default)
        ),
        format!("{}.{}", quote(&names.schema_a), quote(&names.type_a)),
        format!("{}.{}", quote(&names.schema_b), quote(&names.ledger_b)),
        format!("{}.{}", quote(&names.schema_b), quote(&names.table_b)),
    ] {
        exec(
            &mut *connection,
            format!(
                "ALTER {} {} OWNER TO {}",
                object_kind(&object),
                object,
                quote(&names.owner)
            ),
        )
        .await?;
    }

    // Direct, inherited and SET-reachable privileges deliberately differ so
    // the checker must inspect each reachable role rather than only
    // `current_user`.
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a)),
        "SELECT",
        &names.login_a,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a)),
        "SELECT",
        &names.settable,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a)),
        "SELECT",
        &names.nested,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.ledger_a)),
        "SELECT",
        &names.unrelated_creator,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.table_a)),
        "SELECT",
        &names.login_a,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.table_a)),
        "INSERT",
        &names.inherited,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.table_a)),
        "UPDATE",
        &names.settable,
        true,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.table_a)),
        "DELETE",
        &names.admin_target,
        false,
    )
    .await?;
    grant_column(
        &mut *connection,
        &names.schema_a,
        &names.table_a,
        "secret",
        "SELECT",
        &names.login_a,
        true,
    )
    .await?;
    grant_column(
        &mut *connection,
        &names.schema_a,
        &names.table_a,
        "secret",
        "UPDATE",
        &names.login_a,
        true,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.sequence_a)),
        "USAGE",
        &names.inherited,
        false,
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT EXECUTE ON FUNCTION {}.{}(integer) TO {}",
            quote(&names.schema_a),
            quote(&names.routine_a),
            quote(&names.login_a),
        ),
    )
    .await?;
    for argument in ["integer[]", "name", "oidvector", "oid[]"] {
        exec(
            &mut *connection,
            format!(
                "GRANT EXECUTE ON FUNCTION {}.{}({argument}) TO {}",
                quote(&names.schema_a),
                quote(&names.routine_a),
                quote(&names.login_a),
            ),
        )
        .await?;
    }
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_a), quote(&names.sequence_a)),
        "UPDATE",
        &names.settable,
        true,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_b), quote(&names.ledger_b)),
        "SELECT",
        &names.login_b,
        false,
    )
    .await?;
    grant(
        &mut *connection,
        &format!("{}.{}", quote(&names.schema_b), quote(&names.table_b)),
        "SELECT",
        &names.login_b,
        false,
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            quote(&names.schema_a),
            quote(&names.login_a)
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            quote(&names.schema_a),
            quote(&names.settable)
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            quote(&names.schema_a),
            quote(&names.nested)
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            quote(&names.schema_a),
            quote(&names.admin_target)
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            quote(&names.schema_a),
            quote(&names.unrelated_creator)
        ),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT USAGE ON SCHEMA {} TO {}",
            quote(&names.schema_b),
            quote(&names.login_b)
        ),
    )
    .await?;
    // PostgreSQL stores parameter ACLs as one cluster-wide row per parameter;
    // serialize the two independent fixture producers so concurrent live
    // tests do not race that unique row creation.
    exec(
        &mut *connection,
        "SELECT pg_catalog.pg_advisory_lock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT SET ON PARAMETER work_mem TO {}",
            quote(&names.login_b)
        ),
    )
    .await?;
    exec(
        &mut *connection,
        "SELECT pg_catalog.pg_advisory_unlock(hashtext('batter-verification-work-mem'))".to_owned(),
    )
    .await?;
    exec(
        &mut *connection,
        format!(
            "GRANT SELECT ON {}.{} TO PUBLIC",
            quote(&names.schema_a),
            quote(&names.table_a)
        ),
    )
    .await?;
    Ok(())
}
