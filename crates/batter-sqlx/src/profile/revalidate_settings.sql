SELECT pg_catalog.set_config(setting.name, pg_catalog.current_setting(setting.name), false)
FROM pg_catalog.unnest($1::text[]) WITH ORDINALITY AS setting(name, position)
ORDER BY setting.position
