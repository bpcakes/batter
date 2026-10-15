-- Managed routine: edit this file in place; the release applies it after migrations.
CREATE OR REPLACE FUNCTION lifecycle.touch_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END
$$;
