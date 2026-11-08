"""Run explicitly on a disposable macOS runner; no Apple account or real secrets."""

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "macos_ci_credentials", ROOT / "packaging/macos/macos_ci_credentials.py",
)
HELPER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPER)


def checked(condition):
    if not condition:
        raise AssertionError("Native keychain smoke check failed")


class NativeKeychainTransportTests(unittest.TestCase):
    def test_notary_password_uses_private_tty_prompt(self):
        # Fake only the network-facing command, exercise the real pty/fork helper.
        with tempfile.TemporaryDirectory(prefix="notary prompt smoke ") as directory:
            folder = Path(directory)
            executable = folder / "xcrun"
            marker = folder / "passed"
            executable.write_text(
                '#!/usr/bin/env python3\n'
                'import getpass, os, pathlib, sys, termios\n'
                'if not sys.stdin.isatty(): raise SystemExit(3)\n'
                'if termios.tcgetattr(0)[3] & termios.ECHO: raise SystemExit(4)\n'
                'if "--password" in sys.argv or "APPLE_APP_PASSWORD" in os.environ:\n'
                '    raise SystemExit(5)\n'
                'if sys.argv[1:3] != ["notarytool", "store-credentials"]:\n'
                '    raise SystemExit(6)\n'
                'value = getpass.getpass("App-specific password: ")\n'
                'if value != \'dummy "quotes" \\\\ backslash $value\':\n'
                '    raise SystemExit(7)\n'
                'pathlib.Path(os.environ["TEST_MARKER"]).write_text("passed")\n'
            )
            executable.chmod(0o700)
            env = {"PATH": str(folder) + os.pathsep + os.environ["PATH"],
                   "APPLE_APP_PASSWORD": 'dummy "quotes" \\ backslash $value',
                   "APPLE_ID": "dummy@example.invalid", "APPLE_TEAM_ID": "DUMMYTEAM",
                   "KEYCHAIN_PATH": str(folder / "dummy.keychain"),
                   "TEST_MARKER": str(marker)}
            with patch.dict(os.environ, env):
                HELPER.store_notary_credentials()
            checked(marker.read_text() == "passed")

    def test_create_unlock_import_partition_and_delete(self):
        with tempfile.TemporaryDirectory(prefix="solmu signing smoke ") as directory:
            folder = Path(directory)
            keychain = folder / 'transport "quoted" \\ keychain.keychain-db'
            key = folder / "test.key"
            cert = folder / "test.crt"
            bundle = folder / "test.p12"
            password_file = folder / "test-password"
            value = 'dummy "quotes" \\ backslash $value'
            password_file.write_text(value)
            password_file.chmod(0o600)
            env = {"KEYCHAIN_PATH": str(keychain), "CERTIFICATE_PATH": str(bundle),
                   "KEYCHAIN_PASSWORD": value, "P12_PASSWORD": value}
            # Only a throwaway, self-signed identity. Its passphrase is dummy data.
            subprocess.run(  # noqa: S603 - fixed executable, generated fixture paths only
                ["/usr/bin/openssl", "req", "-x509", "-newkey", "rsa:2048",
                 "-nodes", "-keyout", str(key), "-out", str(cert), "-days", "1",
                 "-subj", "/CN=Solmu transport smoke test"],
                capture_output=True, check=True,
            )
            subprocess.run(  # noqa: S603 - fixed executable, generated fixture paths only
                ["/usr/bin/openssl", "pkcs12", "-export", "-inkey", str(key),
                 "-in", str(cert), "-out", str(bundle), "-passout",
                 "file:" + str(password_file)], capture_output=True, check=True,
            )
            with self.assertRaises(RuntimeError):  # noqa: PT027 - stdlib runner
                HELPER.security("unlock-keychain", "-p", "dummy",
                                str(folder / "missing.keychain-db"))
            original = HELPER.security("list-keychains", "-d", "user")
            try:
                with patch.dict(os.environ, env):
                    HELPER.import_certificate()
                result = subprocess.run(  # noqa: S603 - fixed certificate lookup
                    ["/usr/bin/security", "find-certificate", "-c",
                     "Solmu transport smoke test", str(keychain)],
                    capture_output=True, check=True,
                )
                checked(b"Solmu transport smoke test" in result.stdout)
                current = HELPER.keychain_paths(
                    HELPER.security("list-keychains", "-d", "user"),
                )
                checked(set(HELPER.keychain_paths(original)).issubset(current))
                checked(str(keychain) in current)
                # Prove the password was not silently truncated by -i quoting.
                HELPER.security("lock-keychain", str(keychain))
                with self.assertRaises(RuntimeError):  # noqa: PT027 - standalone stdlib runner
                    HELPER.security("unlock-keychain", "-p", "incorrect-dummy-password",
                                    str(keychain))
                HELPER.security("unlock-keychain", "-p", value, str(keychain))
                with self.assertRaises(RuntimeError):  # noqa: PT027 - standalone stdlib runner
                    HELPER.security("import", str(folder / "missing.p12"), "-P", value,
                                    "-k", str(keychain))
            finally:
                HELPER.security("delete-keychain", str(keychain))


if __name__ == "__main__":
    unittest.main()
