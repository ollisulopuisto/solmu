"""Regression tests for lookup failures and report-only result classification."""

import importlib.util
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "report", Path(__file__).with_name("security-pilot-report.py")
)
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


class ClassificationTests(unittest.TestCase):
    def test_clean_requires_valid_successful_lookup(self):
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            report.classify(0, (0, 1), [], True), "NO_FINDINGS"
        )
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            report.classify(2, (0, 1), [], True), "UNKNOWN"
        )
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            report.classify(0, (0, 1), [], False), "UNKNOWN"
        )

    def test_npm_service_error_is_not_clean_even_with_zero_exit(self):
        result = subprocess.CompletedProcess(
            [], 0, '{"error":{"code":"ENETUNREACH"}}', ""
        )
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            report.npm_result(result)[0], "UNKNOWN"
        )

    def test_npm_findings_and_valid_empty_response(self):
        result = subprocess.CompletedProcess(
            [],
            1,
            '{"metadata":{"vulnerabilities":{"high":1}},"vulnerabilities":{"example":{"severity":"high","via":[{"url":"https://example.invalid/advisory"}]}}}',
            "",
        )
        state, findings = report.npm_result(result)
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            state, "FINDINGS"
        )
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            findings[0]["package"], "example"
        )
        result = subprocess.CompletedProcess(
            [], 0, '{"metadata":{"vulnerabilities":{"total":0}},"vulnerabilities":{}}', ""
        )
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            report.npm_result(result)[0], "NO_FINDINGS"
        )

    def test_malformed_npm_is_rejected(self):
        with self.assertRaises(ValueError):  # noqa: PT027 - stdlib-only runner.
            report.npm_result(subprocess.CompletedProcess([], 0, "network failed", ""))

    def test_yanked_crate_null_advisory_preserves_warning(self):
        warnings = {"yanked": [{"advisory": None, "package": {"name": "demo"}}]}
        self.assertEqual(  # noqa: PT009 - stdlib unittest, no pytest dependency.
            report.rust_warnings(warnings),
            [{"kind": "yanked", "package": "demo", "id": None}],
        )

    def test_full_history_includes_merge_resolution_patches(self):
        git_binary = shutil.which("git")
        with tempfile.TemporaryDirectory() as tmp:

            def git(*args, check=True):
                return subprocess.run(  # noqa: S603 - synthetic Git argv.
                    [git_binary, *args],
                    cwd=tmp,
                    check=check,
                    capture_output=True,
                    text=True,
                ).stdout.strip()

            git("init", "-b", "main")
            git("config", "user.name", "Synthetic test")
            git("config", "user.email", "test@example.invalid")
            marker = Path(tmp) / "marker.txt"
            marker.write_text("base\n")
            git("add", ".")
            git("commit", "-m", "base")
            git("checkout", "-b", "side")
            marker.write_text("side\n")
            git("commit", "-am", "side")
            git("checkout", "main")
            marker.write_text("main\n")
            git("commit", "-am", "main")
            git("merge", "side", check=False)
            marker.write_text("synthetic_merge_resolution_marker\n")
            git("add", ".")
            git("commit", "-m", "resolve")
            source = Path(report.__file__).read_text()
            match = re.search(r"--log-opts=([^'\"]+)", source)
            options = match.group(1).split()
            patches = git("log", "-p", *options)
            self.assertIn(  # noqa: PT009 - stdlib unittest, no pytest dependency.
                "+synthetic_merge_resolution_marker", patches
            )


if __name__ == "__main__":
    unittest.main()
