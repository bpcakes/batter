-- Initial flags schema. Forward-only after delivery; ledger is flags._sqlx_migrations.
DO $$ BEGIN
  IF current_setting('server_version_num')::integer < 180000 THEN
    RAISE EXCEPTION 'PostgreSQL 18 is required';
  END IF;
END $$;
CREATE TABLE flags.flag_control (
  singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
  version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
  channel text NOT NULL,
  schema_revision integer NOT NULL DEFAULT 1 CHECK (schema_revision >= 1)
);
INSERT INTO flags.flag_control(channel)
VALUES ('batter_flags_' || left(encode(sha256(convert_to('flags', 'UTF8')), 'hex'), 16));
CREATE TABLE flags.flag_state (
  name text PRIMARY KEY CHECK (name ~ '^[a-z][a-z0-9_]{0,63}$'),
  kind text NOT NULL CHECK (kind IN ('release','mode','switch')),
  value_type text NOT NULL CHECK (value_type IN ('toggle','enum','switch')),
  salt bytea NOT NULL DEFAULT uuid_send(gen_random_uuid()),
  state jsonb NOT NULL CHECK (jsonb_typeof(state) = 'object'
    AND state ? 'encoding' AND jsonb_typeof(state->'encoding') = 'number'
    AND (state->>'encoding')::numeric >= 1
    AND (state->>'encoding')::numeric = trunc((state->>'encoding')::numeric)),
  created_version bigint NOT NULL CHECK (created_version > 0),
  updated_version bigint NOT NULL CHECK (updated_version >= created_version),
  archived_version bigint NULL CHECK (archived_version = updated_version),
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  CHECK ((kind = 'switch') = (value_type = 'switch'))
);
CREATE TABLE flags.flag_audit (
  version bigint PRIMARY KEY,
  flag_name text NOT NULL,
  operation text NOT NULL CHECK (operation IN ('create','set','archive','restore','reshuffle')),
  actor text NOT NULL,
  reason text NOT NULL,
  guard text NOT NULL,
  recorded_by name NOT NULL DEFAULT session_user,
  before_state jsonb NULL,
  after_state jsonb NOT NULL,
  recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE flags.flag_catalogs (
  manifest_digest bytea PRIMARY KEY,
  catalog_digest bytea NOT NULL,
  manifest jsonb NOT NULL,
  first_seen timestamptz NOT NULL DEFAULT clock_timestamp(),
  last_referenced_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE flags.flag_catalog_entries (
  manifest_digest bytea NOT NULL REFERENCES flags.flag_catalogs ON DELETE CASCADE,
  key text NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{0,63}$'),
  kind text NOT NULL CHECK (kind IN ('release','mode','switch')),
  value_type text NOT NULL CHECK (value_type IN ('toggle','enum','switch')),
  fingerprint bytea NOT NULL,
  variants text[] NOT NULL,
  default_variant text NULL,
  expiry date NULL,
  owners text[] NULL,
  only_in text NULL,
  safe_state text NULL,
  stale_rule text NULL,
  governed_by text NULL,
  flip_discipline text NOT NULL,
  PRIMARY KEY (manifest_digest, key),
  CHECK ((kind = 'switch') = (value_type = 'switch'))
);
CREATE INDEX flag_catalog_entries_key ON flags.flag_catalog_entries(key);
CREATE TABLE flags.flag_processes (
  process_id uuid PRIMARY KEY,
  application text NOT NULL,
  environment text NOT NULL,
  build text NOT NULL,
  manifest_digest bytea NOT NULL REFERENCES flags.flag_catalogs,
  snapshot_source text NOT NULL CHECK (snapshot_source IN ('not_loaded','persisted_file','control_plane')),
  snapshot_version bigint NULL CHECK (snapshot_version >= 0),
  declared_revision bytea NOT NULL,
  declared_shared_revision bytea NOT NULL,
  row_encoding smallint NOT NULL DEFAULT 1 CHECK (row_encoding >= 0),
  held_keys text[] NOT NULL CHECK (cardinality(held_keys) <= 64),
  held_overflow bigint NOT NULL CHECK (held_overflow >= 0),
  floor_engaged_keys text[] NOT NULL CHECK (cardinality(floor_engaged_keys) <= 64),
  floor_overflow bigint NOT NULL CHECK (floor_overflow >= 0),
  liveness_seconds integer NOT NULL CHECK (liveness_seconds > 0),
  retention_seconds integer NOT NULL CHECK (retention_seconds > 0),
  expires_at timestamptz NOT NULL,
  started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  last_seen_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  stopped_at timestamptz NULL
);
CREATE INDEX flag_processes_expires_at ON flags.flag_processes(expires_at);
CREATE INDEX flag_processes_manifest ON flags.flag_processes(manifest_digest);

CREATE FUNCTION flags.reject_change() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
BEGIN RAISE EXCEPTION 'flags operation forbidden'; END $$;
REVOKE ALL ON FUNCTION flags.reject_change() FROM PUBLIC;

CREATE FUNCTION flags.guard_control() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
BEGIN
  IF NEW.singleton IS DISTINCT FROM OLD.singleton
     OR NEW.channel IS DISTINCT FROM OLD.channel
     OR NEW.schema_revision < OLD.schema_revision
     OR (NEW.version <> OLD.version + 1 AND NOT
         (NEW.version = OLD.version AND NEW.schema_revision > OLD.schema_revision)) THEN
    RAISE EXCEPTION 'invalid flags counter transition';
  END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION flags.guard_control() FROM PUBLIC;
CREATE TRIGGER guard_control BEFORE UPDATE ON flags.flag_control
FOR EACH ROW EXECUTE FUNCTION flags.guard_control();
CREATE TRIGGER control_no_delete BEFORE DELETE OR INSERT ON flags.flag_control
FOR EACH STATEMENT EXECUTE FUNCTION flags.reject_change();

CREATE FUNCTION flags.check_audited_counter() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
DECLARE found boolean;
BEGIN
  IF NEW.version = OLD.version THEN RETURN NULL; END IF;
  EXECUTE format('SELECT EXISTS (SELECT FROM %I.flag_audit WHERE version = $1)', TG_TABLE_SCHEMA)
    INTO found USING NEW.version;
  IF NOT found THEN RAISE EXCEPTION 'flags counter lacks audit'; END IF;
  RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION flags.check_audited_counter() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER counter_audited AFTER UPDATE ON flags.flag_control
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION flags.check_audited_counter();

CREATE FUNCTION flags.guard_state() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
DECLARE head bigint; audited boolean;
BEGIN
  -- Lock even for raw writers: a concurrent counter change cannot invalidate this check.
  EXECUTE format('SELECT version FROM %I.flag_control WHERE singleton FOR UPDATE', TG_TABLE_SCHEMA)
    INTO head;
  EXECUTE format('SELECT EXISTS (SELECT FROM %I.flag_audit WHERE version = $1)', TG_TABLE_SCHEMA)
    INTO audited USING head;
  IF head < 1 OR audited OR NEW.updated_version <> head THEN
    RAISE EXCEPTION 'flags state requires fresh counter';
  END IF;
  IF TG_OP = 'INSERT' THEN
    IF NEW.created_version <> head OR NEW.archived_version IS NOT NULL THEN
      RAISE EXCEPTION 'invalid initial flags version';
    END IF;
    NEW.created_at := clock_timestamp();
  ELSE
    IF NEW.name IS DISTINCT FROM OLD.name OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.value_type IS DISTINCT FROM OLD.value_type
       OR NEW.created_version IS DISTINCT FROM OLD.created_version
       OR NEW.updated_version <= OLD.updated_version THEN
      RAISE EXCEPTION 'invalid flags state transition';
    END IF;
    IF NEW.salt IS DISTINCT FROM OLD.salt AND
       (NEW.state IS DISTINCT FROM OLD.state OR NEW.archived_version IS DISTINCT FROM OLD.archived_version) THEN
      RAISE EXCEPTION 'invalid flags salt transition';
    END IF;
    IF NEW.archived_version IS NOT NULL AND NEW.archived_version <> head THEN
      RAISE EXCEPTION 'invalid flags archive version';
    END IF;
    NEW.created_at := OLD.created_at;
  END IF;
  NEW.updated_at := clock_timestamp();
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION flags.guard_state() FROM PUBLIC;
CREATE TRIGGER guard_state BEFORE INSERT OR UPDATE ON flags.flag_state
FOR EACH ROW EXECUTE FUNCTION flags.guard_state();
CREATE TRIGGER state_no_delete BEFORE DELETE ON flags.flag_state
FOR EACH STATEMENT EXECUTE FUNCTION flags.reject_change();

CREATE FUNCTION flags.guard_audit() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
DECLARE head bigint;
BEGIN
  EXECUTE format('SELECT version FROM %I.flag_control WHERE singleton', TG_TABLE_SCHEMA) INTO head;
  IF NEW.version <> head OR head < 1 THEN RAISE EXCEPTION 'invalid flags audit version'; END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION flags.guard_audit() FROM PUBLIC;
CREATE TRIGGER guard_audit BEFORE INSERT ON flags.flag_audit
FOR EACH ROW EXECUTE FUNCTION flags.guard_audit();
CREATE TRIGGER audit_immutable BEFORE UPDATE OR DELETE ON flags.flag_audit
FOR EACH STATEMENT EXECUTE FUNCTION flags.reject_change();

CREATE FUNCTION flags.audit_state() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp AS $$
DECLARE actor text := current_setting('batter_flags.actor', true);
  reason text := current_setting('batter_flags.reason', true);
  guard text := current_setting('batter_flags.guard', true);
  operation text; before_state jsonb; channel text;
BEGIN
  IF actor IS NULL OR octet_length(actor) NOT BETWEEN 1 AND 128 OR actor ~ '[[:cntrl:]]'
     OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 1024 OR reason ~ '[[:cntrl:]]'
     OR guard IS NULL OR guard !~ '^(unconditional|unchanged_since:[0-9]+)$' THEN
    RAISE EXCEPTION 'flags audit metadata required';
  END IF;
  IF TG_OP = 'INSERT' THEN operation := 'create';
  ELSE
    before_state := to_jsonb(OLD);
    operation := CASE WHEN NEW.salt IS DISTINCT FROM OLD.salt THEN 'reshuffle'
      WHEN OLD.archived_version IS NULL AND NEW.archived_version IS NOT NULL THEN 'archive'
      WHEN OLD.archived_version IS NOT NULL AND NEW.archived_version IS NULL THEN 'restore'
      ELSE 'set' END;
  END IF;
  EXECUTE format('INSERT INTO %I.flag_audit(version, flag_name, operation, actor, reason, guard, before_state, after_state)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8)', TG_TABLE_SCHEMA)
    USING NEW.updated_version, NEW.name, operation, actor, reason, guard, before_state, to_jsonb(NEW);
  EXECUTE format('SELECT channel FROM %I.flag_control WHERE singleton', TG_TABLE_SCHEMA) INTO channel;
  PERFORM pg_notify(channel, NEW.updated_version::text);
  RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION flags.audit_state() FROM PUBLIC;
CREATE TRIGGER audit_state AFTER INSERT OR UPDATE ON flags.flag_state
FOR EACH ROW EXECUTE FUNCTION flags.audit_state();
DO $$ DECLARE relation text; BEGIN
  FOREACH relation IN ARRAY ARRAY['flag_control','flag_state','flag_audit','flag_catalogs','flag_catalog_entries','flag_processes'] LOOP
    EXECUTE format('CREATE TRIGGER no_truncate BEFORE TRUNCATE ON flags.%I FOR EACH STATEMENT EXECUTE FUNCTION flags.reject_change()', relation);
  END LOOP;
END $$;
