-- Bound PostgreSQL storage without requiring every writer to understand the
-- capacity protocol immediately. The ledger is exact, while admission is
-- sharded so unrelated keys do not contend on one global counter.
--
-- Keep the shard derivation and the per-shard limit stable: both are persisted
-- cross-replica protocol. With 256 shards and 65,536 rows per shard, the table
-- can contain at most 16,777,216 counter rows.

-- Hold this lock through the backfill and trigger installation. Writers using
-- an older runlimit binary wait for the migration transaction to commit, then
-- their INSERT/DELETE statements are accounted for by the triggers below.
LOCK TABLE runlimit_fixed_windows IN ACCESS EXCLUSIVE MODE;

CREATE TABLE runlimit_capacity_shards (
    capacity_shard SMALLINT PRIMARY KEY,
    row_count BIGINT NOT NULL,
    CONSTRAINT runlimit_capacity_shards_valid_shard
        CHECK (capacity_shard BETWEEN 0 AND 255),
    CONSTRAINT runlimit_capacity_shards_nonnegative_count
        CHECK (row_count >= 0),
    CONSTRAINT runlimit_capacity_shards_hard_max
        CHECK (row_count <= 65536)
);

INSERT INTO runlimit_capacity_shards (capacity_shard, row_count)
SELECT shard::SMALLINT, 0
FROM pg_catalog.generate_series(0, 255) AS shards(shard);

ALTER TABLE runlimit_fixed_windows
    ADD COLUMN capacity_shard SMALLINT
        GENERATED ALWAYS AS (
            (
                pg_catalog.get_byte(config_fingerprint, 0)
                # pg_catalog.get_byte(subject_key, 0)
            )::SMALLINT
        ) STORED NOT NULL,
    ADD CONSTRAINT runlimit_fixed_windows_capacity_shard_fkey
        FOREIGN KEY (capacity_shard)
        REFERENCES runlimit_capacity_shards (capacity_shard);

UPDATE runlimit_capacity_shards AS shards
SET row_count = counts.row_count
FROM (
    SELECT capacity_shard, pg_catalog.count(*) AS row_count
    FROM runlimit_fixed_windows
    GROUP BY capacity_shard
) AS counts
WHERE shards.capacity_shard = counts.capacity_shard;

CREATE FUNCTION runlimit_fixed_windows_reject_storage_key_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF
        NEW.config_fingerprint IS DISTINCT FROM OLD.config_fingerprint
        OR NEW.subject_key IS DISTINCT FROM OLD.subject_key
    THEN
        RAISE EXCEPTION
            'runlimit fixed-window storage keys are immutable'
            USING
                ERRCODE = 'check_violation',
                CONSTRAINT = 'runlimit_fixed_windows_immutable_storage_key',
                TABLE = 'runlimit_fixed_windows';
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION runlimit_fixed_windows_capacity_after_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    shard_delta RECORD;
    current_row_count BIGINT;
BEGIN
    -- Lock affected ledger rows in protocol order. Updating through a separate
    -- loop only after each row is locked makes the lock ordering explicit.
    FOR shard_delta IN
        SELECT capacity_shard, pg_catalog.count(*) AS added_rows
        FROM runlimit_inserted_windows
        GROUP BY capacity_shard
        ORDER BY capacity_shard
    LOOP
        current_row_count := NULL;
        EXECUTE pg_catalog.format(
            'SELECT row_count
             FROM %I.runlimit_capacity_shards
             WHERE capacity_shard = $1
             FOR UPDATE',
            TG_TABLE_SCHEMA
        )
        INTO current_row_count
        USING shard_delta.capacity_shard;

        IF current_row_count IS NULL THEN
            RAISE EXCEPTION
                'runlimit capacity ledger is missing shard %',
                shard_delta.capacity_shard
                USING
                    ERRCODE = 'foreign_key_violation',
                    TABLE = 'runlimit_capacity_shards';
        END IF;

        -- 65,536 is the compile-time per-shard hard maximum. Test by
        -- subtraction so even a malformed, enormous transition table cannot
        -- overflow BIGINT while checking the bound.
        IF shard_delta.added_rows > 65536 - current_row_count THEN
            RAISE EXCEPTION
                'runlimit PostgreSQL capacity exhausted for shard %',
                shard_delta.capacity_shard
                USING
                    ERRCODE = 'check_violation',
                    CONSTRAINT = 'runlimit_capacity_shards_hard_max',
                    TABLE = 'runlimit_capacity_shards';
        END IF;

        EXECUTE pg_catalog.format(
            'UPDATE %I.runlimit_capacity_shards
             SET row_count = row_count + $2
             WHERE capacity_shard = $1',
            TG_TABLE_SCHEMA
        )
        USING shard_delta.capacity_shard, shard_delta.added_rows;
    END LOOP;

    RETURN NULL;
END;
$$;

CREATE FUNCTION runlimit_fixed_windows_capacity_after_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    shard_delta RECORD;
    current_row_count BIGINT;
BEGIN
    -- DELETE follows the same ascending lock protocol as INSERT so concurrent
    -- multi-shard admission and cleanup cannot deadlock on the ledger.
    FOR shard_delta IN
        SELECT capacity_shard, pg_catalog.count(*) AS removed_rows
        FROM runlimit_deleted_windows
        GROUP BY capacity_shard
        ORDER BY capacity_shard
    LOOP
        current_row_count := NULL;
        EXECUTE pg_catalog.format(
            'SELECT row_count
             FROM %I.runlimit_capacity_shards
             WHERE capacity_shard = $1
             FOR UPDATE',
            TG_TABLE_SCHEMA
        )
        INTO current_row_count
        USING shard_delta.capacity_shard;

        IF current_row_count IS NULL THEN
            RAISE EXCEPTION
                'runlimit capacity ledger is missing shard %',
                shard_delta.capacity_shard
                USING
                    ERRCODE = 'foreign_key_violation',
                    TABLE = 'runlimit_capacity_shards';
        END IF;

        IF shard_delta.removed_rows > current_row_count THEN
            RAISE EXCEPTION
                'runlimit capacity ledger underflow for shard %',
                shard_delta.capacity_shard
                USING
                    ERRCODE = 'check_violation',
                    CONSTRAINT = 'runlimit_capacity_shards_nonnegative_count',
                    TABLE = 'runlimit_capacity_shards';
        END IF;

        EXECUTE pg_catalog.format(
            'UPDATE %I.runlimit_capacity_shards
             SET row_count = row_count - $2
             WHERE capacity_shard = $1',
            TG_TABLE_SCHEMA
        )
        USING shard_delta.capacity_shard, shard_delta.removed_rows;
    END LOOP;

    RETURN NULL;
END;
$$;

CREATE TRIGGER runlimit_fixed_windows_immutable_storage_key
BEFORE UPDATE OF config_fingerprint, subject_key
ON runlimit_fixed_windows
FOR EACH ROW
EXECUTE FUNCTION runlimit_fixed_windows_reject_storage_key_update();

CREATE TRIGGER runlimit_fixed_windows_capacity_insert
AFTER INSERT ON runlimit_fixed_windows
REFERENCING NEW TABLE AS runlimit_inserted_windows
FOR EACH STATEMENT
EXECUTE FUNCTION runlimit_fixed_windows_capacity_after_insert();

CREATE TRIGGER runlimit_fixed_windows_capacity_delete
AFTER DELETE ON runlimit_fixed_windows
REFERENCING OLD TABLE AS runlimit_deleted_windows
FOR EACH STATEMENT
EXECUTE FUNCTION runlimit_fixed_windows_capacity_after_delete();
