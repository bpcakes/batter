-- Managed constraint trigger routine: edit this file in place; apply installs
-- it before records_note_present, in the same transaction.
CREATE OR REPLACE FUNCTION lifecycle.require_note() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    -- A deferred check receives the row as of its event; read its current
    -- state. Every insert and update queues an event, so the event of a row's
    -- final version reads that version even after its key changed.
    IF EXISTS (SELECT 1 FROM lifecycle.records WHERE id = NEW.id AND btrim(note) = '') THEN
        RAISE EXCEPTION 'record % needs a note', NEW.id USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;
