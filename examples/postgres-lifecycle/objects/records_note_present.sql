-- Managed constraint trigger: dropped and re-created with its routine in one
-- transaction whenever either changes; it checks when the writer commits.
CREATE CONSTRAINT TRIGGER records_note_present
    AFTER INSERT OR UPDATE OF note ON lifecycle.records
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION lifecycle.require_note();
