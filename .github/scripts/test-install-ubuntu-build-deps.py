import importlib.util
import pathlib
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('apt_deps', pathlib.Path(__file__).with_name('install-ubuntu-build-deps.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AptFallbackTests(unittest.TestCase):
    def test_sources_only_change_mirror_not_suite_or_signature(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / 'etc/apt/sources.list.d/ubuntu.sources'
            source.parent.mkdir(parents=True)
            source.write_text('Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\nSuites: noble noble-updates\nComponents: main universe\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n')
            mirror = root / 'etc/apt/apt-mirrors.txt'
            mirror.write_text('http://azure.archive.ubuntu.com/ubuntu\n')
            untouched = source.with_name('vendor.list')
            untouched.write_text('deb https://vendor.example/stable main\n')
            self.assertEqual(module.use_official_mirrors(root), 2)
            self.assertIn('URIs: http://archive.ubuntu.com/ubuntu', source.read_text())
            self.assertIn('Suites: noble noble-updates', source.read_text())
            self.assertIn('Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg', source.read_text())
            self.assertEqual(untouched.read_text(), 'deb https://vendor.example/stable main\n')

    def test_install_failure_retries_update_then_same_packages(self):
        calls = []
        def run(command, **kwargs):
            calls.append((command, kwargs))
            if len(calls) == 2:
                raise subprocess.CalledProcessError(100, command)
        with tempfile.TemporaryDirectory() as directory:
            module.install(['libwayland-dev', 'clang'], pathlib.Path(directory), run)
        self.assertEqual(len(calls), 4)
        self.assertEqual(calls[1][0], calls[3][0])
        self.assertIn('APT::Update::Error-Mode=any', calls[0][0])
        self.assertTrue(all(kwargs['check'] and kwargs['timeout'] <= 180 for _, kwargs in calls))

    def test_success_does_not_retry(self):
        calls = []
        module.install(['clang'], run=lambda cmd, **kw: calls.append(cmd))
        self.assertEqual(len(calls), 2)

    def test_timeout_then_failure_is_fatal_and_bounded(self):
        calls = []
        def run(command, **kwargs):
            calls.append(command)
            if len(calls) == 1:
                raise subprocess.TimeoutExpired(command, 90)
            raise subprocess.CalledProcessError(100, command)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(subprocess.CalledProcessError):
                module.install(['clang'], pathlib.Path(directory), run)
        self.assertEqual(len(calls), 2)


if __name__ == '__main__':
    unittest.main()
