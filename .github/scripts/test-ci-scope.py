import importlib.util
from pathlib import Path
import unittest
import subprocess
import tempfile
import json
import sys

spec = importlib.util.spec_from_file_location('scope', Path(__file__).with_name('ci-scope.py'))
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


def reviewed_registry():
    import hashlib
    registry = scope.load_scene_map()
    registry['contracts'] = {p: hashlib.sha256((scope.ROOT / p).read_bytes()).hexdigest()
                             for p in registry['contracts']}
    return registry


def reviewed_selection(paths, **kwargs):
    return scope.scene_scope(paths, registry=reviewed_registry(), **kwargs)


class ScopeTests(unittest.TestCase):
    def test_only_reviewed_changes_reduce_scenes_not_test_families(self):
        selected = reviewed_selection(['src/reply_router.rs']).split()
        self.assertEqual(len(selected), 9)
        self.assertEqual(reviewed_selection(['src/reply_router.rs'], full=True), 'full')
        self.assertTrue(scope.classify(['src/reply_router.rs'])['native'])
        for extra in ['src/app.rs', 'src/ui/keys.rs', 'src/ui/conversation.rs',
                      'src/ui/merged.rs', 'src/ui/reply_tail.rs', 'src/theme.rs',
                      'src/unknown.rs', '.github/workflows/ci.yml']:
            self.assertEqual(reviewed_selection(['src/reply_router.rs', extra]), 'full', extra)
        self.assertEqual(reviewed_selection([]), 'full')

    def test_nested_crate_manifests_and_locks_select_audit(self):
        for folder in ('harness', 'contrib/signin-check'):
            for name in ('Cargo.toml', 'Cargo.lock'):
                self.assertTrue(scope.classify([folder + '/' + name])['audit'])
        self.assertFalse(scope.classify(['harness/src/lib.rs'])['audit'])

    def test_apt_helper_changes_keep_native_validation(self):
        for path in ['.github/scripts/install-ubuntu-build-deps.py',
                     '.github/scripts/test-install-ubuntu-build-deps.py']:
            self.assertTrue(all(scope.classify([path]).values()), path)
            self.assertFalse(scope.ignored(path))

    def test_real_git_diff_includes_deletions_and_literal_odd_filenames(self):
        script = Path(__file__).with_name('ci-scope.py').resolve()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def git(*args):
                return subprocess.check_output(['git', '-C', tmp, *args], stderr=subprocess.DEVNULL).decode().strip()
            git('init')
            git('config', 'user.email', 'test@example.invalid')
            git('config', 'user.name', 'Test')
            (root / 'src').mkdir()
            file = root / 'src' / 'odd $filename.rs'
            file.write_text('first')
            git('add', '.')
            git('commit', '-m', 'base')
            base = git('rev-parse', 'HEAD')
            file.unlink()
            git('add', '-u')
            git('commit', '-m', 'remove')
            result = subprocess.check_output([sys.executable, str(script), '--base', base], cwd=tmp)
            self.assertEqual(json.loads(result), dict(portable=True, native=True, audit=False, nix=False))
            same = subprocess.check_output([sys.executable, str(script), '--base', 'HEAD'], cwd=tmp)
            self.assertFalse(any(json.loads(same).values()))

    def test_docs_need_no_build(self):
        self.assertFalse(any(scope.classify(['CHECKPOINT.md', 'docs/image.png']).values()))

    def test_python_packaging_is_portable_not_four_native_builds(self):
        self.assertEqual(scope.classify(['contrib/beeper-tools/package_dmg.py']),
                         dict(portable=True, native=False, audit=False, nix=False))

    def test_runtime_keeps_native_without_nix_fanout(self):
        for path in ('src/app.rs', 'assets/icon.png', 'tests/native.rs', 'packaging/macos/bundle.sh'):
            self.assertTrue(scope.classify([path])['native'])
            self.assertTrue(scope.classify([path])['portable'])
            self.assertFalse(scope.classify([path])['nix'])

    def test_dependencies_and_ci_routing_require_all_checks(self):
        for path in ('Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml', 'build.rs', '.github/workflows/ci.yml', '.github/scripts/ci-scope.py'):
            self.assertTrue(all(scope.classify([path]).values()))

    def _git_repo(self, root, files):
        def git(*args):
            return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.DEVNULL).decode().strip()
        git('init')
        git('config', 'user.email', 'test@example.invalid')
        git('config', 'user.name', 'Test')
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        git('add', '.')
        git('commit', '-m', 'base')
        return git

    def _scope(self, script, root, base):
        out = subprocess.check_output([__import__('sys').executable, str(script), '--base', base], cwd=root)
        return json.loads(out)

    def test_version_only_bump_keeps_packaging_and_native_skips_audit_and_nix(self):
        script = Path(__file__).with_name('ci-scope.py').resolve()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git = self._git_repo(root, {
                'Cargo.toml': '[package]\nname = "demo"\nversion = "0.1.0"\n',
                'Cargo.lock': '# lock\n[[package]]\nname = "demo"\nversion = "0.1.0"\n',
                'CHANGELOG.md': '# notes\n',
            })
            base = git('rev-parse', 'HEAD')
            (root / 'Cargo.toml').write_text('[package]\nname = "demo"\nversion = "0.2.0"\n')
            (root / 'Cargo.lock').write_text('# lock\n[[package]]\nname = "demo"\nversion = "0.2.0"\n')
            (root / 'CHANGELOG.md').write_text('# notes\nmore\n')
            git('add', '.'); git('commit', '-m', 'bump')
            self.assertEqual(self._scope(script, root, base),
                             dict(portable=True, native=True, audit=False, nix=False))

    def test_dependency_or_code_changes_keep_every_check(self):
        script = Path(__file__).with_name('ci-scope.py').resolve()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git = self._git_repo(root, {
                'Cargo.toml': '[package]\nname = "demo"\nversion = "0.1.0"\n[dependencies]\nureq = "3"\n',
                'Cargo.lock': '[[package]]\nname = "ureq"\nversion = "3.4.0"\nchecksum = "aaa"\n',
            })
            base = git('rev-parse', 'HEAD')
            # Dependency line edit in the manifest: full checks.
            (root / 'Cargo.toml').write_text('[package]\nname = "demo"\nversion = "0.1.0"\n[dependencies]\nureq = "4"\n')
            git('add', '.'); git('commit', '-m', 'dep')
            self.assertTrue(all(self._scope(script, root, base).values()))
            # Lockfile checksum churn (a real dependency bump): full checks.
            git('checkout', '-q', base)
            (root / 'Cargo.lock').write_text('[[package]]\nname = "ureq"\nversion = "3.5.0"\nchecksum = "bbb"\n')
            git('add', '.'); git('commit', '-m', 'lock')
            self.assertTrue(all(self._scope(script, root, base).values()))

    def test_nix_changes_are_checked(self):
        for path in ('flake.lock', 'flake.nix', 'packaging/flatpak/manifest.yml'):
            self.assertTrue(scope.classify([path])['nix'])

    def test_unknown_implementation_fails_open_and_mixed_scope_unions(self):
        self.assertTrue(scope.classify(['new-code/file'])['native'])
        self.assertTrue(scope.classify(['README.md', 'src/app.rs'])['native'])
        self.assertTrue(all(scope.classify([], full=True).values()))


class SceneMapTests(unittest.TestCase):
    def test_current_registry_is_complete(self):
        self.assertEqual(scope.scene_map_errors(), [])

    def test_new_or_deleted_module_fails_completeness_and_fails_open(self):
        registry = reviewed_registry()
        registry['modules'].pop('src/ui/settings.rs')
        self.assertTrue(scope.scene_map_errors(registry=registry))
        self.assertEqual(scope.scene_scope(['src/reply_router.rs'], registry=registry), 'full')
        registry['modules']['src/brand_new.rs'] = {'coverage': 'full', 'reason': 'new'}
        self.assertTrue(scope.scene_map_errors(registry=registry))

    def test_unmapped_new_scene_fails_check_and_fails_open(self):
        registry = reviewed_registry()
        registry['scenes'].append('new-gallery-scene')
        self.assertTrue(scope.scene_map_errors(registry=registry))
        self.assertEqual(scope.scene_scope(['src/ui/settings.rs'], registry=registry), 'full')

    def test_unknown_scene_reference_is_rejected(self):
        registry = reviewed_registry()
        registry['modules']['src/ui/info.rs']['coverage'] = ['typo']
        self.assertTrue(scope.scene_map_errors(registry=registry))

    def test_contract_drift_invalidates_all_skips(self):
        registry = reviewed_registry()
        registry['contracts']['src/demo.rs'] = 'stale'
        self.assertEqual(scope.scene_scope(['src/ui/settings.rs'], registry=registry), 'full')

    def test_safe_mixed_paths_union_and_docs_do_not_widen(self):
        a = set(reviewed_selection(['src/ui/settings.rs']).split())
        b = set(reviewed_selection(['src/ui/message_info.rs']).split())
        both = reviewed_selection(['src/ui/settings.rs', 'src/ui/message_info.rs', 'README.md'])
        self.assertEqual(set(both.split()), a | b)
        self.assertLess(len(both.split()), len(scope.load_scene_map()['scenes']))

    def test_global_and_unknown_paths_fail_open_even_with_safe_change(self):
        for path in ['src/app.rs', 'src/theme.rs', 'src/i18n.rs', 'src/model.rs',
                     'src/ui/keys.rs', 'src/ui/focus.rs', 'src/ui/hover_actions.rs',
                     'src/ui/widgets.rs', 'src/ui/pane.rs', 'src/ui/reply_tail.rs', 'src/ui/info.rs',
                     'assets/fonts/new.ttf', 'locales/fi.po', 'Cargo.toml',
                     '.github/scripts/scene-map.json', 'unknown/new-code.rs']:
            self.assertEqual(reviewed_selection(['src/ui/settings.rs', path]), 'full', path)

    def test_stale_contract_does_not_fail_completeness_but_forces_full(self):
        registry = reviewed_registry()
        registry['contracts']['src/demo.rs'] = 'old-base-hash'
        self.assertEqual(scope.scene_map_errors(registry=registry), [])
        self.assertEqual(scope.scene_scope(['src/ui/settings.rs'], registry=registry), 'full')

    def test_missing_contract_rejects_skips(self):
        registry = reviewed_registry()
        registry['contracts'].pop('src/ui/conversation.rs')
        self.assertTrue(scope.scene_map_errors(registry=registry))
        self.assertEqual(scope.scene_scope(['src/ui/settings.rs'], registry=registry), 'full')

    def test_test_and_standalone_bin_changes_keep_baseline_screenshots(self):
        for path in ['src/demo/frame_perf.rs', 'src/bin/solmu-control.rs']:
            self.assertEqual(reviewed_selection([path]), 'baseline')
            self.assertTrue(scope.classify([path])['native'])
        self.assertEqual(reviewed_selection(['src/demo/frame_perf.rs', 'src/ui/message_info.rs']),
                         reviewed_selection(['src/ui/message_info.rs']))

    def test_all_reviewed_modules_select_nonempty_known_scenes(self):
        registry = reviewed_registry()
        reduced = 0
        for path, entry in registry['modules'].items():
            result = reviewed_selection([path])
            if entry['coverage'] == 'full':
                self.assertEqual(result, 'full', path)
            else:
                reduced += 1
                self.assertEqual(set(result.split()), set(entry['coverage']) if entry['coverage'] else {'baseline'}, path)
        self.assertGreaterEqual(reduced, 5)


if __name__ == '__main__':
    unittest.main()
