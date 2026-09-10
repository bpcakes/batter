"""Controls preventing compile-only or skipped probes from becoming live evidence."""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

from reference_live import CASES, complete_execution, complete_inventory, local_endpoint
import reference_live


class ReferenceLiveControls(unittest.TestCase):
    def test_failed_prerequisite_stops_before_inventory_or_fixtures(self):
        endpoint = "postgres://probe@127.0.0.1/postgres?sslmode=disable"
        for result in (SimpleNamespace(ok=True, stdout=b"f\n"),
                       SimpleNamespace(ok=False, stdout=b"t\n")):
            with patch.dict("os.environ", {"POSTGRES_TEST_ADMIN_URL": endpoint}), \
                    patch.object(reference_live, "run", return_value=result) as run:
                with self.assertRaisesRegex(SystemExit, "pg_catalog.pg_shdescription"):
                    reference_live.main()
                self.assertEqual(run.call_count, 1)
                self.assertEqual(run.call_args.args[0][0], "psql")

    def test_requires_local_postgres_endpoint_without_tls_or_host_override(self):
        self.assertTrue(local_endpoint("postgres://postgres@127.0.0.1:5432/postgres?sslmode=disable"))
        for value in ("", "https://localhost/db", "postgres://remote.invalid/db",
                      "postgres://localhost", "postgres://localhost/db?sslmode=require",
                      "postgres://localhost/db?host=remote.invalid", "postgres://[bad/db"):
            with self.subTest(value=value):
                self.assertFalse(local_endpoint(value))

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
