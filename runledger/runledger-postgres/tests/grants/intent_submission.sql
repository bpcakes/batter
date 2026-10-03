-- Frozen expected grants for RunledgerOperation::IntentSubmission.
GRANT USAGE ON SCHEMA "jobs" TO "jobs_service";
GRANT SELECT ("enqueue_request"), INSERT ("enqueue_request") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("enqueue_request_version") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("execution_resource_key") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("id"), UPDATE ("id") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("idempotency_key"), INSERT ("idempotency_key") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("job_type"), INSERT ("job_type") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("max_attempts") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("next_run_at") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("organization_id"), INSERT ("organization_id") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("payload") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("priority") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("promoted_job_id") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("stage") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT SELECT ("status") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
GRANT INSERT ("timeout_seconds") ON TABLE "jobs"."job_enqueue_intents" TO "jobs_service";
