import tempfile
from pathlib import Path
import unittest

from record_verification import nextest_summary


class VerificationSummaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.log = Path(self.temp.name) / "test.log"

    def test_records_executed_tests_and_explicit_retries(self):
        self.log.write_text(
            "RETRY 1/2 example\n\x1b[32;1mSummary [ 1.2s]\x1b[0m 17 tests run: 17 passed, 350 skipped\n"
        )
        result = nextest_summary(self.log)
        self.assertEqual(result["tests_run"], 17)
        self.assertEqual(result["retries"], ["RETRY 1/2 example"])

    def test_missing_summary_is_rejected(self):
        self.log.write_text("Finished test profile; no tests executed\n")
        with self.assertRaises(ValueError):
            nextest_summary(self.log)

    def test_partial_or_failed_summary_is_rejected(self):
        self.log.write_text("Summary [ 1.2s] 17 tests run: 16 passed, 1 failed\n")
        with self.assertRaises(ValueError):
            nextest_summary(self.log)

    def test_zero_test_run_is_rejected(self):
        self.log.write_text("Summary [ 1.2s] 0 tests run: 0 passed, 350 skipped\n")
        with self.assertRaises(ValueError):
            nextest_summary(self.log)

    def test_concatenated_attempts_are_not_a_single_passing_run(self):
        self.log.write_text(
            "Summary [ 1.2s] 17 tests run: 16 passed, 1 failed\nSummary [ 1.2s] 17 tests run: 17 passed\n"
        )
        with self.assertRaises(ValueError):
            nextest_summary(self.log)


if __name__ == "__main__":
    unittest.main()
