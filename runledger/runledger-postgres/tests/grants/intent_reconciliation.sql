-- Frozen expected grants for RunledgerOperation::IntentReconciliation.
GRANT USAGE ON SCHEMA "jobs" TO "jobs_service";
GRANT SELECT ("is_enabled") ON TABLE "jobs"."job_definitions" TO "jobs_service";
GRANT SELECT ("job_type") ON TABLE "jobs"."job_definitions" TO "jobs_service";
GRANT SELECT ("version") ON TABLE "jobs"."job_definitions" TO "jobs_service";
GRANT SELECT ("id") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("job_type") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("organization_id") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("promoted_job_id") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("status") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("id") ON TABLE "jobs"."job_queue" TO "jobs_service";
GRANT SELECT ("job_type") ON TABLE "jobs"."job_queue" TO "jobs_service";
GRANT SELECT ("organization_id") ON TABLE "jobs"."job_queue" TO "jobs_service";
GRANT SELECT ("run_number") ON TABLE "jobs"."job_queue" TO "jobs_service";
GRANT SELECT ("status") ON TABLE "jobs"."job_queue" TO "jobs_service";
