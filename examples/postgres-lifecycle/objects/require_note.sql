-- Managed constraint trigger routine: edit this file in place; the release
-- installs it in the same transaction as records_note_present.
CREATE OR REPLACE FUNCTION lifecycle.require_note() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    -- A deferred check receives the row as of its event; read its current state.
    IF EXISTS (SELECT 1 FROM lifecycle.records WHERE id = NEW.id AND btrim(note) = '') THEN
        RAISE EXCEPTION 'record % needs a note', NEW.id USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;
