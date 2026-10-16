-- Managed constraint trigger: edit this file in place. PostgreSQL cannot
-- replace a constraint trigger, so when this definition changes or drifts,
-- apply drops and re-creates it after its routine, in one transaction. It
-- checks when the writer commits, after every insert and update of a row.
CREATE CONSTRAINT TRIGGER records_note_present
    AFTER INSERT OR UPDATE ON lifecycle.records
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION lifecycle.require_note();
