-- Inspect catalogs only: never execute application functions or probe writes.
WITH tables(name, oid) AS (
    VALUES
        ('runlimit_fixed_windows', pg_catalog.to_regclass('runlimit_fixed_windows')),
        ('runlimit_capacity_shards', pg_catalog.to_regclass('runlimit_capacity_shards'))
),
columns(table_name, name, type_name, generated, insertable, updatable) AS (
    VALUES
        ('runlimit_fixed_windows', 'policy_id', 'text', '', true, true),
        ('runlimit_fixed_windows', 'scope_id', 'text', '', true, true),
        ('runlimit_fixed_windows', 'config_fingerprint', 'bytea', '', true, false),
        ('runlimit_fixed_windows', 'subject_key', 'bytea', '', true, false),
        ('runlimit_fixed_windows', 'window_started_at', 'timestamptz', '', true, true),
        ('runlimit_fixed_windows', 'window_expires_at', 'timestamptz', '', true, true),
        ('runlimit_fixed_windows', 'used', 'int8', '', true, true),
        ('runlimit_fixed_windows', 'capacity_shard', 'int2', 's', false, false),
        ('runlimit_capacity_shards', 'capacity_shard', 'int2', '', false, false),
        ('runlimit_capacity_shards', 'row_count', 'int8', '', false, false)
),
constraints(table_name, name, definition) AS (
    VALUES
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_pkey',
         'PRIMARY KEY (config_fingerprint, subject_key)'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_policy_id_size',
         'CHECK (((octet_length(policy_id) >= 1) AND (octet_length(policy_id) <= 128)))'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_scope_id_size',
         'CHECK (((octet_length(scope_id) >= 1) AND (octet_length(scope_id) <= 128)))'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_fingerprint_size',
         'CHECK ((octet_length(config_fingerprint) = 32))'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_subject_key_size',
         'CHECK ((octet_length(subject_key) = 32))'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_valid_window',
         'CHECK ((window_expires_at > window_started_at))'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_positive_usage',
         'CHECK ((used > 0))'),
        ('runlimit_fixed_windows', 'runlimit_fixed_windows_capacity_shard_fkey',
         'FOREIGN KEY (capacity_shard) REFERENCES runlimit_capacity_shards(capacity_shard)'),
        ('runlimit_capacity_shards', 'runlimit_capacity_shards_pkey',
         'PRIMARY KEY (capacity_shard)'),
        ('runlimit_capacity_shards', 'runlimit_capacity_shards_valid_shard',
         'CHECK (((capacity_shard >= 0) AND (capacity_shard <= 255)))'),
        ('runlimit_capacity_shards', 'runlimit_capacity_shards_nonnegative_count',
         'CHECK ((row_count >= 0))'),
        ('runlimit_capacity_shards', 'runlimit_capacity_shards_hard_max',
         'CHECK ((row_count <= 65536))')
),
triggers(name, function_name, kind, attributes, old_table, new_table) AS (
    VALUES
        ('runlimit_fixed_windows_immutable_storage_key',
         'runlimit_fixed_windows_reject_storage_key_update', 19,
         ARRAY['config_fingerprint', 'subject_key'], NULL, NULL),
        ('runlimit_fixed_windows_capacity_insert',
         'runlimit_fixed_windows_capacity_after_insert', 4,
         ARRAY[]::text[], NULL, 'runlimit_inserted_windows'),
        ('runlimit_fixed_windows_capacity_delete',
         'runlimit_fixed_windows_capacity_after_delete', 8,
         ARRAY[]::text[], 'runlimit_deleted_windows', NULL)
),
functions AS (
    SELECT expected.name, expected.body, p.*, n.oid AS schema_oid
    FROM ROWS FROM (pg_catalog.unnest($1::text[]), pg_catalog.unnest($2::text[])) AS expected(name, body)
    JOIN tables t ON t.name = 'runlimit_fixed_windows'
    JOIN pg_catalog.pg_class relation ON relation.oid = t.oid
    JOIN pg_catalog.pg_namespace n ON n.oid = relation.relnamespace
    LEFT JOIN pg_catalog.pg_proc p
        ON p.pronamespace = n.oid AND p.proname = expected.name AND p.pronargs = 0
),
issues(object, requirement) AS (
    SELECT t.name, 'permanent ordinary table without row-level security or inheritance'
    FROM tables t
    LEFT JOIN pg_catalog.pg_class c ON c.oid = t.oid
    WHERE c.oid IS NULL OR c.relkind <> 'r' OR c.relpersistence <> 'p'
        OR c.relrowsecurity OR c.relforcerowsecurity
        OR EXISTS (SELECT FROM pg_catalog.pg_inherits i
                   WHERE i.inhrelid = c.oid OR i.inhparent = c.oid)

    UNION ALL
    SELECT 'runlimit_capacity_shards', 'same schema as runlimit_fixed_windows'
    FROM pg_catalog.pg_class c, pg_catalog.pg_class w
    WHERE c.oid = (SELECT oid FROM tables WHERE name = 'runlimit_capacity_shards')
        AND w.oid = (SELECT oid FROM tables WHERE name = 'runlimit_fixed_windows')
        AND c.relnamespace <> w.relnamespace

    UNION ALL
    SELECT t.name, 'USAGE on the containing schema'
    FROM tables t JOIN pg_catalog.pg_class c ON c.oid = t.oid
    WHERE NOT pg_catalog.has_schema_privilege(c.relnamespace, 'USAGE')

    UNION ALL
    SELECT e.table_name || '.' || e.name, 'NOT NULL ' || e.type_name || ' column with published generation mode'
    FROM columns e JOIN tables t ON t.name = e.table_name
    LEFT JOIN pg_catalog.pg_attribute a
        ON a.attrelid = t.oid AND a.attname = e.name AND a.attnum > 0 AND NOT a.attisdropped
    WHERE a.attnum IS NULL OR NOT a.attnotnull
        OR a.atttypid <> pg_catalog.to_regtype('pg_catalog.' || e.type_name)
        OR a.atttypmod <> -1 OR a.attgenerated::text <> e.generated OR a.attidentity <> ''

    UNION ALL
    SELECT t.name || '.' || a.attname, 'no additional required columns without defaults'
    FROM tables t JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid
    WHERE a.attnum > 0 AND NOT a.attisdropped AND a.attnotnull
        AND NOT a.atthasdef AND a.attidentity = '' AND a.attgenerated = ''
        AND NOT EXISTS (SELECT FROM columns e WHERE e.table_name = t.name AND e.name = a.attname)

    UNION ALL
    SELECT t.name || '.' || a.attname, 'no additional generated columns'
    FROM tables t JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid
    WHERE a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated <> ''
        AND NOT EXISTS (SELECT FROM columns e WHERE e.table_name = t.name AND e.name = a.attname)

    UNION ALL
    SELECT 'runlimit_fixed_windows.capacity_shard', 'published stored XOR shard expression'
    WHERE NOT EXISTS (
        SELECT FROM tables t
        JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attname = 'capacity_shard'
        JOIN pg_catalog.pg_attrdef d ON d.adrelid = t.oid AND d.adnum = a.attnum
        WHERE t.name = 'runlimit_fixed_windows'
          AND pg_catalog.pg_get_expr(d.adbin, d.adrelid) =
              '((get_byte(config_fingerprint, 0) # get_byte(subject_key, 0)))::smallint'
          -- Built-in functions and operators have no pg_depend entries, while
          -- identically named schema-local replacements do. The expression text
          -- alone is search-path-sensitive, so reject replacement dependencies.
          AND NOT EXISTS (
              SELECT FROM pg_catalog.pg_depend dependency
              WHERE dependency.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass
                  AND dependency.objid = d.oid
                  AND dependency.refclassid IN (
                      'pg_catalog.pg_proc'::pg_catalog.regclass,
                      'pg_catalog.pg_operator'::pg_catalog.regclass
                  )
          )
    )

    UNION ALL
    SELECT e.table_name || '.' || e.name, 'validated, enforced, nondeferrable ' || e.definition
    FROM constraints e JOIN tables t ON t.name = e.table_name
    LEFT JOIN pg_catalog.pg_constraint c ON c.conrelid = t.oid AND c.conname = e.name
    WHERE c.oid IS NULL OR NOT c.convalidated OR c.condeferrable
        -- conenforced was introduced in PostgreSQL 18.
        OR NOT COALESCE((pg_catalog.to_jsonb(c)->>'conenforced')::boolean, true)
        OR pg_catalog.pg_get_constraintdef(c.oid) <> e.definition
        -- Match the immutable built-in expression, not a same-named function
        -- resolved from an application schema in the current search path.
        OR EXISTS (
            SELECT FROM pg_catalog.pg_depend dependency
            WHERE dependency.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass
                AND dependency.objid = c.oid
                AND dependency.refclassid IN (
                    'pg_catalog.pg_proc'::pg_catalog.regclass,
                    'pg_catalog.pg_operator'::pg_catalog.regclass
                )
        )
        OR (c.contype = 'p' AND NOT EXISTS (
            SELECT FROM pg_catalog.pg_index i
            WHERE i.indexrelid = c.conindid AND i.indisvalid AND i.indisready AND i.indislive
        ))
        OR (c.contype = 'f' AND (
            c.confrelid <> (SELECT oid FROM tables WHERE name = 'runlimit_capacity_shards')
            OR EXISTS (SELECT FROM pg_catalog.pg_trigger g
                       WHERE g.tgconstraint = c.oid AND g.tgenabled NOT IN ('O', 'A'))
        ))

    UNION ALL
    SELECT t.name || '.' || c.conname, 'no additional behavioral constraints'
    FROM tables t JOIN pg_catalog.pg_constraint c ON c.conrelid = t.oid
    WHERE c.contype <> 'n' AND NOT EXISTS (
        SELECT FROM constraints e WHERE e.table_name = t.name AND e.name = c.conname
    )

    UNION ALL
    SELECT t.name || '.' || idx.relname, 'no additional unique indexes restricting admission'
    FROM tables t
    JOIN pg_catalog.pg_index i ON i.indrelid = t.oid
    JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
    WHERE i.indisunique AND NOT i.indisprimary

    UNION ALL
    SELECT 'runlimit_fixed_windows_expiry_idx', 'valid nonpartial ascending btree index on window_expires_at'
    WHERE NOT EXISTS (
        SELECT FROM tables t
        JOIN pg_catalog.pg_class idx ON idx.relnamespace =
            (SELECT relnamespace FROM pg_catalog.pg_class WHERE oid = t.oid)
            AND idx.relname = 'runlimit_fixed_windows_expiry_idx'
        JOIN pg_catalog.pg_index i ON i.indexrelid = idx.oid AND i.indrelid = t.oid
        JOIN pg_catalog.pg_am am ON am.oid = idx.relam
        JOIN pg_catalog.pg_opclass op ON op.oid = i.indclass[0]
        WHERE t.name = 'runlimit_fixed_windows' AND am.amname = 'btree' AND op.opcdefault
            AND i.indisvalid AND i.indisready AND i.indislive AND NOT i.indisunique
            AND i.indpred IS NULL AND i.indexprs IS NULL AND i.indnkeyatts = 1
            AND i.indoption[0] = 0
            AND pg_catalog.pg_get_indexdef(i.indexrelid, 1, true) = 'window_expires_at'
    )

    UNION ALL
    SELECT e.name, 'enabled published trigger, function, columns and transition tables'
    FROM triggers e
    LEFT JOIN pg_catalog.pg_trigger g
        ON g.tgrelid = (SELECT oid FROM tables WHERE name = 'runlimit_fixed_windows')
        AND g.tgname = e.name
    LEFT JOIN functions f ON f.name = e.function_name
    WHERE g.oid IS NULL OR g.tgisinternal OR g.tgenabled NOT IN ('O', 'A')
        OR g.tgtype <> e.kind OR g.tgfoid IS DISTINCT FROM f.oid
        OR g.tgnargs <> 0 OR g.tgqual IS NOT NULL
        OR g.tgoldtable::text IS DISTINCT FROM e.old_table
        OR g.tgnewtable::text IS DISTINCT FROM e.new_table
        OR ARRAY(SELECT a.attname::text FROM pg_catalog.unnest(g.tgattr) WITH ORDINALITY AS k(num, position)
                 JOIN pg_catalog.pg_attribute a ON a.attrelid = g.tgrelid AND a.attnum = k.num
                 ORDER BY k.position) <> e.attributes

    UNION ALL
    SELECT t.name || '.' || g.tgname, 'no additional user triggers'
    FROM tables t JOIN pg_catalog.pg_trigger g ON g.tgrelid = t.oid
    WHERE NOT g.tgisinternal AND NOT (t.name = 'runlimit_fixed_windows' AND
        EXISTS (SELECT FROM triggers e WHERE e.name = g.tgname))

    UNION ALL
    SELECT t.name || '.' || r.rulename, 'no rewrite rules'
    FROM tables t JOIN pg_catalog.pg_rewrite r ON r.ev_class = t.oid

    UNION ALL
    SELECT f.name, 'published plpgsql security-definer trigger function and fixed search_path'
    FROM functions f LEFT JOIN pg_catalog.pg_language l ON l.oid = f.prolang
    WHERE f.oid IS NULL OR l.lanname <> 'plpgsql' OR NOT f.prosecdef
        OR f.prorettype <> 'pg_catalog.trigger'::pg_catalog.regtype
        OR f.prokind <> 'f' OR f.provolatile <> 'v' OR f.proisstrict OR f.proretset
        OR f.proconfig IS DISTINCT FROM ARRAY['search_path=pg_catalog, pg_temp']
        OR pg_catalog.btrim(f.prosrc) <> pg_catalog.btrim(f.body)

    UNION ALL
    SELECT f.name, 'function owner has schema USAGE, ledger SELECT/UPDATE, and capacity-function EXECUTE permissions'
    FROM functions f JOIN tables t ON t.name = 'runlimit_capacity_shards'
    WHERE f.oid IS NOT NULL AND t.oid IS NOT NULL
      AND f.name <> 'runlimit_fixed_windows_reject_storage_key_update' AND (
        NOT pg_catalog.has_schema_privilege(f.proowner, f.schema_oid, 'USAGE')
        OR EXISTS (
            SELECT FROM pg_catalog.pg_attribute a
            WHERE a.attrelid = t.oid AND NOT a.attisdropped AND (
                (a.attname IN ('capacity_shard', 'row_count')
                 AND NOT pg_catalog.has_column_privilege(f.proowner, t.oid, a.attnum, 'SELECT'))
                OR (a.attname = 'row_count'
                    AND NOT pg_catalog.has_column_privilege(f.proowner, t.oid, a.attnum, 'UPDATE'))
            )
        )
        OR EXISTS (
            SELECT FROM (VALUES
                ('pg_catalog.count()'),
                ('pg_catalog.format(text,"any")')
            ) AS routines(signature)
            WHERE NOT pg_catalog.has_function_privilege(
                f.proowner,
                pg_catalog.to_regprocedure(routines.signature),
                'EXECUTE'
            )
        )
    )

    UNION ALL
    SELECT e.table_name || '.' || e.name, permission || ' privilege for current_user'
    FROM columns e JOIN tables t ON t.name = e.table_name
    JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attname = e.name AND NOT a.attisdropped
    CROSS JOIN (VALUES ('SELECT'), ('INSERT'), ('UPDATE')) AS p(permission)
    WHERE (permission = 'SELECT' OR (permission = 'INSERT' AND e.insertable)
           OR (permission = 'UPDATE' AND e.updatable))
        AND NOT pg_catalog.has_column_privilege(t.oid, a.attnum, permission)

    UNION ALL
    SELECT t.name, 'DELETE privilege for current_user'
    FROM tables t WHERE t.name = 'runlimit_fixed_windows' AND t.oid IS NOT NULL
        AND NOT pg_catalog.has_table_privilege(t.oid, 'DELETE')

    UNION ALL
    SELECT t.name || '.ctid', 'table SELECT privilege for cleanup system-column access'
    FROM tables t WHERE t.name = 'runlimit_fixed_windows' AND t.oid IS NOT NULL
        AND NOT pg_catalog.has_table_privilege(t.oid, 'SELECT')

    UNION ALL
    SELECT t.name, 'UPDATE privilege on at least one column for row locking'
    FROM tables t WHERE t.oid IS NOT NULL
        AND NOT pg_catalog.has_any_column_privilege(t.oid, 'UPDATE')

    UNION ALL
    SELECT 'session_replication_role', 'origin or local so capacity and foreign-key triggers execute'
    WHERE pg_catalog.current_setting('session_replication_role') NOT IN ('origin', 'local')

    UNION ALL
    SELECT signature, 'EXECUTE privilege for current_user'
    FROM (VALUES
        ('pg_catalog.pg_advisory_xact_lock(bigint)'),
        ('pg_catalog.set_config(text,text,boolean)'),
        ('pg_catalog.clock_timestamp()'),
        ('pg_catalog.get_byte(bytea,integer)'),
        ('pg_catalog.octet_length(text)'),
        ('pg_catalog.octet_length(bytea)'),
        ('pg_catalog.cardinality(anyarray)'),
        ('pg_catalog.unnest(anyarray)'),
        ('pg_catalog.count()')
    ) AS routines(signature)
    WHERE NOT pg_catalog.has_function_privilege(pg_catalog.to_regprocedure(signature), 'EXECUTE')
)
SELECT object, requirement FROM issues ORDER BY object, requirement
