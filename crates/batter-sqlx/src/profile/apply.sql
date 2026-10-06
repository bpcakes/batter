SELECT pg_catalog.set_config(setting.name, setting.value, false)
FROM ROWS FROM (pg_catalog.unnest($1::text[]), pg_catalog.unnest($2::text[]))
     WITH ORDINALITY AS setting(name, value, position)
ORDER BY setting.position
