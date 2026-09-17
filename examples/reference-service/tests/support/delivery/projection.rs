use super::*;

// Native termination may happen without a handler write. Exercise the actual
// authenticated read boundaries against those retained pairs, including both
// certainty values, rather than simulating a successful finalizer.
pub(super) async fn probe(fixture: &CommandFixture, delivery: Uuid) -> ProbeResult {
    let paths = [
        format!("/deliveries/{delivery}"),
        "/delivery-commands/command-a".to_owned(),
    ];
    for (job_state, attempt, effect_state, possible, expected) in [
        ("DEAD_LETTERED", 3, "AWAITING_ATTEMPT", false, "exhausted"),
        (
            "DEAD_LETTERED",
            3,
            "RETRYABLE_UNDISPATCHED",
            false,
            "exhausted",
        ),
        ("DEAD_LETTERED", 3, "RECONCILE_NEEDED", true, "exhausted"),
        (
            "DEAD_LETTERED",
            1,
            "AWAITING_ATTEMPT",
            false,
            "manual_resolution",
        ),
        ("CANCELED", 1, "RECONCILE_NEEDED", true, "manual_resolution"),
        (
            "SUCCEEDED",
            1,
            "AWAITING_ATTEMPT",
            false,
            "manual_resolution",
        ),
        ("DEAD_LETTERED", 3, "CONFIRMED", false, "confirmed"),
    ] {
        sqlx::query(
            "UPDATE job_queue SET status = $2::job_status, attempt = $3
            WHERE id = (SELECT job_id FROM reference_deliveries WHERE id = $1)",
        )
        .bind(delivery)
        .bind(job_state)
        .bind(attempt)
        .execute(&fixture.pool)
        .await?;
        sqlx::query(
            "UPDATE reference_delivery_effects SET state = $2, acceptance_possible = $3,
            dispatch_possible_at = CASE WHEN $3 THEN clock_timestamp() ELSE NULL END,
            resolve_before = CASE WHEN $3 THEN clock_timestamp() + interval '1 hour' ELSE NULL END,
            provider_effect_id = CASE WHEN $2 = 'CONFIRMED' THEN 'provider:retained' ELSE NULL END
            WHERE delivery_id = $1",
        )
        .bind(delivery)
        .bind(effect_state)
        .bind(possible)
        .execute(&fixture.pool)
        .await?;
        for path in &paths {
            let (status, body) =
                envelope_request(&fixture.app_a, Method::GET, path, TOKEN_A, None).await?;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["delivery"]["provider"]["state"], expected);
            assert_eq!(
                body["delivery"]["provider"]["acceptance_possible"],
                possible
            );
            if effect_state == "CONFIRMED" {
                assert_eq!(
                    body["delivery"]["provider"]["provider_effect_id"],
                    "provider:retained"
                );
            }
        }
        let retained: String = sqlx::query_scalar(
            "SELECT state FROM reference_delivery_effects WHERE delivery_id = $1",
        )
        .bind(delivery)
        .fetch_one(&fixture.pool)
        .await?;
        assert_eq!(
            retained, effect_state,
            "read projected state by mutating retained facts"
        );
    }

    missing_effect_is_an_invariant(fixture, delivery, &paths).await
}

async fn missing_effect_is_an_invariant(
    fixture: &CommandFixture,
    delivery: Uuid,
    paths: &[String],
) -> ProbeResult {
    sqlx::query("DELETE FROM reference_delivery_effects WHERE delivery_id = $1")
        .bind(delivery)
        .execute(&fixture.pool)
        .await?;
    for path in paths {
        let (status, _) =
            envelope_request(&fixture.app_a, Method::GET, path, TOKEN_A, None).await?;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "missing effect became a 404"
        );
        let (foreign_status, _) =
            envelope_request(&fixture.app_b, Method::GET, path, TOKEN_B, None).await?;
        if path.starts_with("/deliveries/") {
            assert_eq!(foreign_status, StatusCode::NOT_FOUND);
        } else {
            // The other owner deliberately submitted the same key earlier.
            assert_eq!(foreign_status, StatusCode::OK);
        }
    }
    Ok(())
}
