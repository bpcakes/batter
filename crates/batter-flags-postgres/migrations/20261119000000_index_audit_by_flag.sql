-- One flag's audit events in version order, for keyed history reads in both
-- directions, so a rarely changed flag does not scan every other flag's events.
CREATE INDEX flag_audit_by_flag ON flags.flag_audit (flag_name, version);
