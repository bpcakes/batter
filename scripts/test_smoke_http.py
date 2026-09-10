"""Negative controls for the HTTP process smoke's telemetry oracle."""

import unittest

from smoke_http import check_completion_fields, check_operation_filter


class CompletionFieldsTests(unittest.TestCase):
    fields = ('method="GET"', 'route="/work"', 'status=500',
              'http_outcome="server_error"', 'latency_ms=1.5')

    def line(self, event_fields):
        # Every expected field is present on the span, even for broken events.
        return ('WARN request{request_id=example-1}:batter.http{'
                + " ".join(self.fields)
                + '}: batter: HTTP response boundary finished '
                + " ".join(event_fields))

    def test_accepts_event_fields_for_success_client_error_and_server_error(self):
        for status, outcome in ((200, "completed"), (404, "client_error"),
                                (500, "server_error"), (503, "server_error")):
            with self.subTest(status=status):
                fields = ('method="GET"', 'route="/work"', f"status={status}",
                          f'http_outcome="{outcome}"', 'latency_ms=1.5')
                check_completion_fields(self.line(fields), "/work", status)

    def test_rejects_each_field_missing_from_event_even_when_span_has_it(self):
        for missing in self.fields:
            with self.subTest(missing=missing), self.assertRaises(RuntimeError):
                check_completion_fields(
                    self.line([field for field in self.fields if field != missing]),
                    "/work", 500,
                )

    def test_rejects_conflicting_event_fields_even_when_span_matches(self):
        for expected, wrong in zip(self.fields, (
            'method="POST"', 'route="/other"', 'status=5000',
            'http_outcome="completed"', 'other_latency_ms=1.5',
        )):
            with self.subTest(expected=expected), self.assertRaises(RuntimeError):
                check_completion_fields(
                    self.line([wrong if field == expected else field for field in self.fields]),
                    "/work", 500,
                )

    def test_correlation_must_belong_to_event_even_when_span_matches(self):
        with self.assertRaises(RuntimeError):
            check_completion_fields(self.line(self.fields), "/work", 500, "example-1")
        check_completion_fields(
            self.line(self.fields + ('request_id="example-1"',)), "/work", 500, "example-1",
        )
        with self.assertRaises(RuntimeError):
            check_completion_fields(
                self.line(self.fields + ('request_id="wrong-id"',)), "/work", 500, "example-1",
            )

    def test_rejects_line_without_completion_boundary(self):
        with self.assertRaises(RuntimeError):
            check_completion_fields(" ".join(self.fields), "/work", 500)


class OperationFilterTests(unittest.TestCase):
    def test_rejects_info_operation_completion_among_allowed_events(self):
        for timestamp in ("", "2026-09-09T10:00:00.000000Z "):
            with self.subTest(timestamp=timestamp):
                output = (
                    f'{timestamp} INFO request{{request_id=example-1}}: http_service: handler finished\n'
                    f'{timestamp} WARN batter: operation boundary finished outcome="deadline_exceeded"\n'
                    f'{timestamp} INFO request{{request_id=example-2}}: batter: operation boundary finished '
                    'outcome="succeeded"\n'
                )
                with self.assertRaisesRegex(RuntimeError, "INFO operation event"):
                    check_operation_filter(output)

    def test_accepts_warn_operation_completion_and_application_info(self):
        check_operation_filter(
            '2026-09-09T10:00:00.000000Z  INFO request{request_id=example-1}: http_service: handler finished\n'
            '2026-09-09T10:00:00.000000Z  WARN batter: operation boundary finished outcome="deadline_exceeded"\n'
        )

    def test_accepts_absent_operation_events(self):
        check_operation_filter("")


if __name__ == "__main__":
    unittest.main()
