-- Managed trigger: bound after its routine on every apply that changes either.
CREATE OR REPLACE TRIGGER records_touch_updated_at
    BEFORE UPDATE ON lifecycle.records
    FOR EACH ROW EXECUTE FUNCTION lifecycle.touch_updated_at();
