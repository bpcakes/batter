-- Each audit event identifies one write across restores and version reuse.
-- A previously stored heartbeat has no evidence tying it to that history.
ALTER TABLE flags.flag_audit ADD COLUMN publication_id uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE flags.flag_processes ADD COLUMN publication_id uuid;
UPDATE flags.flag_control SET schema_revision = 2 WHERE singleton AND schema_revision < 2;
