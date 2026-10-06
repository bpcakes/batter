SELECT session_user::text AS login, current_user::text AS role,
       pg_catalog.pg_has_role(session_user, current_user, 'SET') AS can_set_role,
       pg_catalog.current_setting('search_path') AS path,
       ARRAY(SELECT COALESCE((SELECT pg_catalog.has_schema_privilege(n.oid, 'USAGE')
                              FROM pg_catalog.pg_namespace n
                              WHERE n.nspname = declared.name), false)
             FROM pg_catalog.unnest($1::text[]) WITH ORDINALITY AS declared(name, position)
             ORDER BY declared.position) AS schemas,
       ARRAY(SELECT (EXTRACT(EPOCH FROM pg_catalog.current_setting(declared.name, true)::interval)
                     * 1000)::bigint
             FROM pg_catalog.unnest($2::text[]) WITH ORDINALITY AS declared(name, position)
             ORDER BY declared.position) AS timeouts,
       ARRAY(SELECT pg_catalog.current_setting(declared.name)
             FROM pg_catalog.unnest($3::text[]) WITH ORDINALITY AS declared(name, position)
             ORDER BY declared.position) AS settings,
       pg_catalog.pg_current_xact_id_if_assigned()::text AS xid,
       pg_catalog.current_setting('transaction_isolation') AS isolation,
       pg_catalog.current_setting('transaction_read_only') AS read_only
