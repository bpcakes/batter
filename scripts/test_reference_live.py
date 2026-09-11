"""Controls preventing compile-only or skipped probes from becoming live evidence."""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

from reference_live import CASES, cluster_identity, complete_execution, complete_inventory, local_endpoint
import reference_live


class ReferenceLiveControls(unittest.TestCase):
    def test_failed_prerequisite_stops_before_inventory_or_fixtures(self):
        endpoint = "postgres://probe@127.0.0.1/postgres?sslmode=disable"
        for result in (SimpleNamespace(ok=True, stdout=b"f|123\n"),
                       SimpleNamespace(ok=False, stdout=b"t|123\n")):
            with patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": endpoint, "POSTGRES_TEST_OBSERVER_URL": endpoint}), \
                    patch.object(reference_live, "run", return_value=result) as run:
                with self.assertRaisesRegex(SystemExit, "pg_catalog.pg_shdescription"):
                    reference_live.main()
                self.assertEqual(run.call_count, 1)
                self.assertEqual(run.call_args.args[0][0], "psql")
                self.assertIn("current_setting('track_counts')::boolean", run.call_args.args[0][-1])

    def test_secondary_endpoint_is_required_before_any_execution(self):
        with patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": "postgres://probe@127.0.0.1/postgres",
                                       "POSTGRES_TEST_OBSERVER_URL": ""}), \
                patch.object(reference_live, "run") as run:
            with self.assertRaisesRegex(SystemExit, "POSTGRES_TEST_OBSERVER_URL"):
                reference_live.main()
            run.assert_not_called()

    def test_invalid_secondary_endpoint_stops_before_any_execution(self):
        for endpoint in ("postgres://remote.invalid/db", "postgres://localhost/db?sslmode=require",
                         "postgres://localhost/db?host=remote.invalid"):
            with self.subTest(endpoint=endpoint), \
                    patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": "postgres://probe@127.0.0.1/postgres",
                                              "POSTGRES_TEST_OBSERVER_URL": endpoint}), \
                    patch.object(reference_live, "run") as run:
                with self.assertRaisesRegex(SystemExit, "POSTGRES_TEST_OBSERVER_URL"):
                    reference_live.main()
                run.assert_not_called()

    def test_failed_secondary_stops_before_inventory(self):
        endpoint = "postgres://probe@127.0.0.1/postgres"
        for failure in (SimpleNamespace(ok=True, stdout=b"f\n"),
                        SimpleNamespace(ok=False, stdout=b"t\n")):
            with patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": endpoint,
                                           "POSTGRES_TEST_OBSERVER_URL": endpoint}), \
                    patch.object(reference_live, "run", side_effect=[SimpleNamespace(ok=True, stdout=b"t|123\n"), failure]) as run:
                with self.assertRaisesRegex(SystemExit, "Secondary PostgreSQL"):
                    reference_live.main()
                self.assertEqual(run.call_count, 2)

    def test_cluster_identity_requires_valid_successful_output(self):
        for identity in (0, 123, -123, -(1 << 63), (1 << 63) - 1):
            with self.subTest(identity=identity):
                self.assertEqual(cluster_identity(SimpleNamespace(ok=True, stdout=f"t|{identity}\n".encode())), identity)
        for output in (b"t\n", b"t|\n", b"t|unknown\n", b"f|123\n", b"t|123|extra\n",
                       b"t|-\n", b"t|1_23\n", b"t|" + b"9" * 5000, b"t|9223372036854775808\n", b"t|-9223372036854775809\n"):
            self.assertIsNone(cluster_identity(SimpleNamespace(ok=True, stdout=output)))
        self.assertIsNone(cluster_identity(SimpleNamespace(ok=False, stdout=b"t|123\n")))

    def test_same_cluster_stops_before_inventory_even_with_different_urls(self):
        with patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": "postgres://probe@127.0.0.1/db",
                                       "POSTGRES_TEST_OBSERVER_URL": "postgres://probe@localhost/other"}), \
                patch.object(reference_live, "run", return_value=SimpleNamespace(ok=True, stdout=b"t|123\n")) as run:
            with self.assertRaisesRegex(SystemExit, "same cluster"):
                reference_live.main()
            self.assertEqual(run.call_count, 2)

    def test_requires_local_postgres_endpoint_without_tls_or_host_override(self):
        self.assertTrue(local_endpoint("postgres://postgres@127.0.0.1:5432/postgres?sslmode=disable"))
        for value in ("", "https://localhost/db", "postgres://remote.invalid/db",
                      "postgres://localhost", "postgres://localhost/db?sslmode=require",
                      "postgres://localhost/db?host=remote.invalid", "postgres://[bad/db"):
            with self.subTest(value=value):
                self.assertFalse(local_endpoint(value))

    def test_distinct_clusters_reach_complete_inventory_and_execution(self):
        inventory = "\n".join(f"{case}: test" for case in CASES).encode()
        executed = "\n".join(f"test {case} ... ok" for case in CASES)
        executed += f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n"
        results = [SimpleNamespace(ok=True, stdout=value, output="")
                   for value in (b"t|123\n", b"t|456\n", inventory, executed.encode())]
        with patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": "postgres://probe@127.0.0.1/db",
                                       "POSTGRES_TEST_OBSERVER_URL": "postgres://probe@localhost/other"}), \
                patch.object(reference_live, "run", side_effect=results) as run, \
                patch("builtins.print"):
            reference_live.main()
            self.assertEqual(run.call_count, 4)
            self.assertEqual(run.call_args_list[2].args[0], reference_live.COMMAND + ["--", "--ignored", "--list"])
            self.assertEqual(run.call_args_list[3].args[0], reference_live.COMMAND + ["--", "--ignored", "--test-threads=1"])

    def test_inventory_requires_every_named_ignored_case(self):
        output = "\n".join(f"{case}: test" for case in CASES)
        self.assertTrue(complete_inventory(output))
        self.assertFalse(complete_inventory(output.replace(next(iter(CASES)), "unexpected")))
        self.assertFalse(complete_inventory("0 tests, 0 benchmarks"))

    def test_success_requires_each_case_and_unfiltered_summary(self):
        output = "\n".join(f"test {case} ... ok" for case in CASES)
        summary = f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n"
        self.assertTrue(complete_execution(output + summary))
        self.assertFalse(complete_execution(summary))
        self.assertFalse(complete_execution(output + summary.replace(f"{len(CASES)} passed", f"{len(CASES) - 1} passed")))
        self.assertFalse(complete_execution(output + summary.replace(f"{len(CASES)} passed", f"{len(CASES) + 1} passed")))
        self.assertFalse(complete_execution(output + summary.replace("0 ignored", "1 ignored")))
        self.assertFalse(complete_execution(output + summary.replace("0 filtered", "1 filtered")))
        self.assertFalse(complete_execution(output.replace("... ok", "... ignored", 1) + summary))


if __name__ == "__main__":
    unittest.main()
