"""Controls preventing incomplete SQLx live runs from becoming evidence."""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

import sqlx_live


def inventory(cases):
    return "\n".join(f"{case}: test" for case in cases).encode()


def execution(cases):
    lines = "\n".join(f"test {case} ... ok" for case in cases)
    summary = (f"\ntest result: ok. {len(cases)} passed; 0 failed; 0 ignored; "
               "0 measured; 0 filtered out;")
    return (lines + summary).encode()


class SqlxLiveControls(unittest.TestCase):
    def test_inventory_requires_every_exact_name(self):
        for cases in sqlx_live.TARGETS.values():
            output = inventory(cases)
            self.assertTrue(sqlx_live.complete_inventory(output, cases))
            self.assertFalse(sqlx_live.complete_inventory(
                output.replace(next(iter(cases)).encode(), b"unexpected"), cases))
            self.assertFalse(sqlx_live.complete_inventory(b"0 tests, 0 benchmarks", cases))

    def test_execution_rejects_missing_skipped_or_summary_only(self):
        for cases in sqlx_live.TARGETS.values():
            output = execution(cases)
            self.assertTrue(sqlx_live.complete_execution(output, cases))
            self.assertFalse(sqlx_live.complete_execution(output.split(b"test result:")[1], cases))
            self.assertFalse(sqlx_live.complete_execution(
                output.replace(b"... ok", b"... ignored", 1), cases))
            missing = next(iter(cases)).encode()
            self.assertFalse(sqlx_live.complete_execution(
                output.replace(b"test " + missing + b" ... ok\n", b""), cases))

    def test_duplicate_success_line_cannot_replace_a_case(self):
        cases = sqlx_live.TARGETS["pool_ownership_live"]
        first, second = list(cases)[:2]
        output = execution(cases).replace(f"test {first} ... ok".encode(),
                                          f"test {second} ... ok".encode())
        self.assertFalse(sqlx_live.complete_execution(output, cases))

    def test_missing_environment_fails_before_invocation(self):
        with patch.dict("os.environ", {}, clear=True), \
                patch.object(sqlx_live, "run_parallel") as run:
            self.assertEqual(sqlx_live.main(), 1)
            run.assert_not_called()

    def test_main_discovers_then_executes_each_target(self):
        results = []
        for cases in sqlx_live.TARGETS.values():
            results.extend([
                SimpleNamespace(ok=True, stdout=inventory(cases)),
                SimpleNamespace(ok=True, stdout=execution(cases)),
            ])
        with patch.dict("os.environ", {"DATABASE_URL": "redacted"}, clear=True), \
                patch.object(sqlx_live, "run_parallel",
                             side_effect=[([result]) for result in results]) as run, \
                patch.object(sqlx_live, "render_outcomes"):
            self.assertEqual(sqlx_live.main(), 0)
        self.assertEqual(run.call_count, len(sqlx_live.TARGETS) * 2)


if __name__ == "__main__":
    unittest.main()
