-- The create-table migration shipped in runlimit-postgres 0.1.0 and must keep
-- its published checksum. Apply storage tuning separately so existing
-- installations upgrade without a migration checksum mismatch.
--
-- Counter increments update only non-indexed columns and can use HOT updates
-- when their heap page has room. Window renewals also update the indexed
-- window_expires_at column and therefore still require regular vacuuming.
ALTER TABLE runlimit_fixed_windows
    SET (fillfactor = 80);
