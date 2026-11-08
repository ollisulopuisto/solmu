"""Regression tests for literal workflow routing without third-party modules."""
from pathlib import Path
import json
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def workflow(name):
    return (ROOT / '.github' / 'workflows' / name).read_text()


def run_block(text, marker):
    block = text.split(marker, 1)[1]
    lines = []
    for line in block.splitlines():
        if line and not line.startswith('          '):
            break
        lines.append(line[10:] if line.startswith('          ') else line)
    return '\n'.join(lines)


class WorkflowTests(unittest.TestCase):
    def test_trial_is_explicit_and_selects_only_requested_native_target(self):
        text = workflow('trial-binaries.yml')
        trigger = text.split('on:', 1)[1].split('permissions:', 1)[0]
        self.assertNotIn('pull_request:', trigger)
        self.assertNotIn('push:', trigger)
        self.assertIn('workflow_dispatch:', trigger)
        block = run_block(text, '        run: |\n')
        targets = ['aarch64-apple-darwin', 'x86_64-apple-darwin',
                   'x86_64-unknown-linux-gnu', 'x86_64-pc-windows-msvc']
        for target in targets + ['all', 'invalid']:
            with tempfile.NamedTemporaryFile() as out:
                env = dict(os.environ, TARGET=target, GITHUB_OUTPUT=out.name)
                result = subprocess.run(['bash', '-eu', '-c', block], env=env, capture_output=True)
                if target == 'invalid':
                    self.assertNotEqual(result.returncode, 0)
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    rows = json.loads(Path(out.name).read_text().split('=', 1)[1])['include']
                    self.assertEqual([r['target'] for r in rows], targets if target == 'all' else [target])
        self.assertIn('fromJSON(needs.select.outputs.matrix)', text)
        self.assertIn('FAST_BEEPER_UPLOAD_ARTIFACTS', text)

    def test_manual_mac_version_is_accepted_by_native_packaging(self):
        text = workflow('release.yml')
        block = run_block(text.split('name: Resolve version and DMG name', 1)[1], '        run: |\n')
        for event, tag in [('workflow_dispatch', 'main'), ('push', 'v0.24.0'), ('push', 'v0.25.0-rc.2')]:
            script = block.replace('${{ github.event_name }}', event).replace('${{ github.run_number }}', '7').replace("${{ inputs.macos_arch || 'arm64' }}", 'arm64')
            with tempfile.NamedTemporaryFile() as out:
                env = dict(os.environ, GITHUB_REF_NAME=tag, GITHUB_ENV=out.name)
                result = subprocess.run(['bash', '-eu', '-c', script], cwd=ROOT, env=env, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                values = dict(line.split('=', 1) for line in Path(out.name).read_text().splitlines())
                self.assertRegex(values['PACKAGE_VERSION'], r'^\d+\.\d+\.\d+(?:-(?:alpha|beta|rc)\.\d+)?$')
                if event == 'workflow_dispatch':
                    self.assertTrue(values['PACKAGE_VERSION'].endswith('-alpha.7'))
                    self.assertTrue(values['DMG_NAME'].endswith('-manual.7-macos-arm64.dmg'))
                else:
                    self.assertEqual(values['PACKAGE_VERSION'], tag[1:])
                    self.assertEqual(values['DMG_NAME'], f'solmu-{tag}-macos-universal.dmg')
        self.assertIn('--version "$PACKAGE_VERSION" --target "$PACKAGE_TARGET"', text)
        self.assertIn("if: github.event_name == 'workflow_dispatch'", text)

    def test_manual_mac_architecture_choice_and_tag_universal(self):
        text = workflow('release.yml')
        self.assertIn('default: arm64', text)
        block = run_block(text.split('name: Resolve version and DMG name', 1)[1], '        run: |\n')
        for event, choice, expected in [('workflow_dispatch', 'arm64', 'arm64'), ('workflow_dispatch', 'universal', 'universal'), ('push', 'arm64', 'universal')]:
            script = block.replace('${{ github.event_name }}', event).replace('${{ github.run_number }}', '7').replace("${{ inputs.macos_arch || 'arm64' }}", choice)
            with tempfile.NamedTemporaryFile() as out:
                result = subprocess.run(['bash', '-eu', '-c', script], cwd=ROOT, env=dict(os.environ, GITHUB_REF_NAME='v0.24.6', GITHUB_ENV=out.name), capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                values = dict(line.split('=', 1) for line in Path(out.name).read_text().splitlines())
                self.assertEqual(values['MACOS_ARCH'], expected)
                self.assertEqual(values['PACKAGE_TARGET'], 'macos-' + expected)
                self.assertTrue(values['DMG_NAME'].endswith('-macos-' + expected + '.dmg'))

    def test_mac_build_and_verification_use_exact_selected_slices(self):
        build = ROOT / 'packaging/macos/build-release.sh'
        verify = ROOT / 'packaging/macos/verify-architectures.sh'
        for architecture in ['arm64', 'universal', 'invalid']:
            with tempfile.TemporaryDirectory() as directory:
                work = Path(directory)
                bin_dir = work / 'bin'
                bin_dir.mkdir()
                for name in ['cargo', 'rustup', 'lipo']:
                    stub = bin_dir / name
                    stub.write_text('#!/bin/bash\nset -eu\necho "' + name + ' $*" >> "$LOG"\n' +
                        ('mkdir -p "target/$5/release"; touch "target/$5/release/solmu"\n' if name == 'cargo' else '') +
                        ('if [ "$1" = -archs ]; then echo "$SLICES"; else touch "${@: -1}"; fi\n' if name == 'lipo' else ''))
                    stub.chmod(0o755)
                env = dict(os.environ, PATH=str(bin_dir) + ':' + os.environ['PATH'], LOG=str(work/'calls'), SLICES='arm64')
                result = subprocess.run(['bash', str(build), architecture], cwd=work, env=env, capture_output=True)
                if architecture == 'invalid':
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse((work/'calls').exists())
                    continue
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = (work/'calls').read_text()
                self.assertIn('aarch64-apple-darwin', calls)
                self.assertEqual('x86_64-apple-darwin' in calls, architecture == 'universal')
                self.assertTrue((work/'macos-binary/solmu').exists())
                for slices in ['arm64', 'x86_64 arm64', 'x86_64']:
                    env['SLICES'] = slices
                    result = subprocess.run(['bash', str(verify), 'app-binary', architecture], env=env, capture_output=True)
                    self.assertEqual(result.returncode == 0, slices == ('arm64' if architecture == 'arm64' else 'x86_64 arm64'))
        verifier = (ROOT / 'packaging/macos/verify.sh').read_text()
        self.assertIn('verify_architectures "$app/Contents/MacOS/solmu"', verifier)
        self.assertIn('verify_architectures "$app/Contents/MacOS/solmu-client"', verifier)

    def test_ci_selection_modes(self):
        text = workflow('ci.yml')
        block = run_block(text.split('id: scope', 1)[1], '        run: |\n')
        for event, scope, expected in [
            ('schedule', '', dict(portable=False, native=False, audit=True, nix=False)),
            ('workflow_dispatch', 'fast', dict(portable=True, native=False, audit=False, nix=False)),
            ('workflow_dispatch', 'full', dict(portable=True, native=True, audit=True, nix=True)),
            ('merge_group', '', dict(portable=True, native=True, audit=True, nix=True)),
            ('push', '', dict(portable=True, native=True, audit=True, nix=True)),
        ]:
            with tempfile.NamedTemporaryFile() as out:
                env = dict(os.environ, EVENT_NAME=event, REQUESTED_SCOPE=scope,
                           BASE='0' * 40, HEAD='HEAD', GITHUB_OUTPUT=out.name)
                result = subprocess.run(['bash', '-eu', '-c', block], cwd=ROOT, env=env, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                actual = dict(line.split('=') for line in Path(out.name).read_text().splitlines())
                self.assertEqual(actual, {k: str(v).lower() for k, v in expected.items()})

    def test_full_matrices_belong_to_tags_and_manual_runs(self):
        text = workflow('ci.yml')
        trigger = text.split('on:', 1)[1].split('permissions:', 1)[0]
        self.assertIn('tags: ["v*"]', trigger)
        self.assertIn("github.event_name == 'push' && !startsWith(github.ref, 'refs/tags/')", text)
        # The heavy compile legs skip routine main pushes and PRs alike.
        self.assertEqual(text.count("(github.event_name != 'push' || startsWith(github.ref, 'refs/tags/'))"), 2)

    def test_expensive_checks_follow_selection_and_portable_failure(self):
        text = workflow('ci.yml')
        self.assertEqual(text.count('needs: [changes, portable]'), 6)
        self.assertIn('!cancelled() && !failure()', text)
        self.assertIn("vars.FAST_BEEPER_UPLOAD_ARTIFACTS == 'true'", text)
        self.assertIn('cancel-in-progress: true', workflow('flatpak.yml'))
        self.assertIn("upload-artifact: ${{ vars.FAST_BEEPER_UPLOAD_ARTIFACTS == 'true' }}", workflow('flatpak.yml'))


if __name__ == '__main__':
    unittest.main()
