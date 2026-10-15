-- Versioned history keeps tables, columns, constraints and data changes.
-- Routines and triggers live in ../objects and are applied after this.
CREATE SCHEMA IF NOT EXISTS lifecycle;

CREATE TABLE lifecycle.records (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    note text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now()
);
