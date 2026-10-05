-- Frozen expected grants for RunledgerOperation::SchemaSnapshot.
GRANT USAGE ON SCHEMA "jobs" TO "jobs_service";
GRANT SELECT ON TABLE "jobs"."_sqlx_migrations" TO "jobs_service";
GRANT SELECT ON TABLE "jobs"."job_queue" TO "jobs_service";
GRANT SELECT ON TABLE "jobs"."runledger_migration_history" TO "jobs_service";
GRANT SELECT ON TABLE "jobs"."workflow_runs" TO "jobs_service";
GRANT SELECT ON TABLE "jobs"."workflow_steps" TO "jobs_service";
