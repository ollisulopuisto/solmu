"""Regression tests for lookup failures and report-only result classification."""
import importlib.util
from pathlib import Path
import subprocess
import unittest

spec = importlib.util.spec_from_file_location('report', Path(__file__).with_name('security-pilot-report.py'))
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


class ClassificationTests(unittest.TestCase):
    def test_clean_requires_valid_successful_lookup(self):
        self.assertEqual(report.classify(0, (0, 1), [], True), 'NO_FINDINGS')
        self.assertEqual(report.classify(2, (0, 1), [], True), 'UNKNOWN')
        self.assertEqual(report.classify(0, (0, 1), [], False), 'UNKNOWN')

    def test_npm_service_error_is_not_clean_even_with_zero_exit(self):
        result = subprocess.CompletedProcess([], 0, '{"error":{"code":"ENETUNREACH"}}', '')
        self.assertEqual(report.npm_result(result)[0], 'UNKNOWN')

    def test_npm_findings_and_valid_empty_response(self):
        result = subprocess.CompletedProcess([], 1, '{"metadata":{"vulnerabilities":{"high":1}},"vulnerabilities":{"example":{"severity":"high","via":[{"url":"https://example.invalid/advisory"}]}}}', '')
        state, findings = report.npm_result(result)
        self.assertEqual(state, 'FINDINGS')
        self.assertEqual(findings[0]['package'], 'example')
        result = subprocess.CompletedProcess([], 0, '{"metadata":{"vulnerabilities":{"total":0}},"vulnerabilities":{}}', '')
        self.assertEqual(report.npm_result(result)[0], 'NO_FINDINGS')

    def test_malformed_npm_is_rejected(self):
        with self.assertRaises(ValueError):
            report.npm_result(subprocess.CompletedProcess([], 0, 'network failed', ''))


if __name__ == '__main__':
    unittest.main()
