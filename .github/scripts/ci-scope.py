#!/usr/bin/env python3
"""Select check families from tracked changed paths. Unknown code fails open.

A diff whose only build-relevant changes rewrite bare `version = "..."` lines
in Cargo.toml/Cargo.lock (a release-metadata bump: no dependency, checksum,
source or name edits) keeps portable packaging validation and native
quality/tests but skips the advisory audit and Nix fan-out, which a pure
version rewrite cannot affect. Docs-only changes already select no builds.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess

# Workflow/selector edits always keep every check family.
FULL_TRIGGER = {'.github/workflows/ci.yml', '.github/scripts/ci-scope.py',
                '.github/scripts/test-ci-scope.py', '.github/scripts/test-ci-workflows.py',
                '.github/scripts/scene-map.json', '.github/scripts/test-scene-map.py',
                '.github/scripts/install-ubuntu-build-deps.py',
                '.github/scripts/test-install-ubuntu-build-deps.py'}


def ignored(path):
    """Paths that never select a build, mirroring classify's skips."""
    if path.startswith('docs/') or path.endswith('.md'):
        return True
    if path in FULL_TRIGGER:
        return False
    return path.startswith('.github/') or path in ('.gitignore', '.gitattributes', 'LICENSE', 'THIRD-PARTY-NOTICES.md')


def version_only_change(paths, diff_text):
    """True when every build-relevant path is Cargo.toml/Cargo.lock and every
    added/removed diff line is a bare quoted version rewrite."""
    heavy = [p for p in paths if not ignored(p)]
    if not heavy or any(p not in ('Cargo.toml', 'Cargo.lock') for p in heavy):
        return False
    changed = False
    for line in diff_text.splitlines():
        if line.startswith(('+++', '---')):
            continue
        if line.startswith(('+', '-')):
            changed = True
            if not re.fullmatch(r'version = "\d+(\.\d+){1,3}"', line[1:].strip()):
                return False
    return changed


def classify(paths, full=False):
    flags = dict(portable=False, native=False, audit=False, nix=False)
    if full:
        return dict.fromkeys(flags, True)
    for path in paths:
        if path.startswith(('docs/',)) or path.endswith('.md'):
            continue
        if path.endswith(('/Cargo.toml', '/Cargo.lock')):
            flags['audit'] = True
        if path.startswith('contrib/beeper-tools/'):
            flags['portable'] = True
        elif path in ('flake.nix', 'flake.lock') or path.startswith('packaging/flatpak/'):
            flags['nix'] = True
            flags['native'] = True
        elif path in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'build.rs'):
            flags.update(dict.fromkeys(flags, True))
        elif path.startswith(('src/', 'assets/', 'tests/', 'examples/', 'packaging/macos/', 'packaging/windows/')):
            # Routine source merges keep portable+native quality feedback; the
            # three-system Nix fan-out belongs to Nix inputs, tags and full runs.
            flags['portable'] = flags['native'] = True
        elif path.startswith('harness/'):
            flags['portable'] = True
        elif path in FULL_TRIGGER:
            flags.update(dict.fromkeys(flags, True))
        elif path.startswith('.github/') or path in ('.gitignore', '.gitattributes', 'LICENSE', 'THIRD-PARTY-NOTICES.md'):
            continue
        else:
            # New or unclassified implementation must not silently lose coverage.
            flags['portable'] = flags['native'] = flags['nix'] = True
    return flags


# Every Rust module is classified, including explicit full-coverage entries.
# The reviewed map is data, and changes to it select full CI themselves.
SCENE_MAP = Path(__file__).with_name('scene-map.json')
ROOT = Path(__file__).resolve().parents[2]


def load_scene_map():
    return json.loads(SCENE_MAP.read_text())


def scene_map_errors(root=ROOT, registry=None, check_contracts=False):
    """Completeness is mechanical; reachability is a reviewed contract."""
    import hashlib
    registry = registry if registry is not None else load_scene_map()
    errors = []
    actual = {str(p.relative_to(root)) for p in (root / 'src').rglob('*.rs')}
    mapped = set(registry['modules'])
    if actual != mapped:
        errors.append('Rust module map drift: ' + repr(sorted(actual ^ mapped)))
    scenes = registry['scenes']
    if not scenes or len(scenes) != len(set(scenes)):
        errors.append('Empty or duplicate scene registry')
    workflow = (root / '.github/workflows/ci.yml').read_text()
    galleries = re.findall(r"all_scenes='([^']+)'", workflow)
    if len(galleries) != 2 or any(g.split() != scenes for g in galleries):
        errors.append('Both workflow galleries must exactly match scene registry')
    for path, entry in registry['modules'].items():
        coverage = entry['coverage']
        if not entry.get('reason'):
            errors.append('Missing review reason: ' + path)
        if coverage != 'full' and (not isinstance(coverage, list)
                                   or set(coverage) - set(scenes)):
            errors.append('Invalid scene coverage: ' + path)
    required_contracts = {p for p, e in registry['modules'].items() if e['coverage'] == 'full'} | {'Cargo.toml'}
    if set(registry['contracts']) != required_contracts:
        errors.append('Incomplete reachability contract inventory')
    for path, digest in (registry['contracts'].items() if check_contracts else []):
        if not (root / path).is_file() or hashlib.sha256((root / path).read_bytes()).hexdigest() != digest:
            errors.append('Reachability contract changed: ' + path)
    return errors


def scene_scope(paths, full=False, root=ROOT, registry=None):
    registry = registry if registry is not None else load_scene_map()
    heavy = [p for p in paths if not ignored(p)]
    # Invalid/stale registry never grants a skip. CI separately rejects missing
    # scene/module classifications, but a stale call/fixture contract is safe full.
    if full or not heavy or scene_map_errors(root, registry, check_contracts=True):
        return 'full'
    selected = set()
    for path in heavy:
        entry = registry['modules'].get(path)
        if not entry or entry['coverage'] == 'full':
            return 'full'
        selected.update(entry['coverage'])
    return ' '.join(s for s in registry['scenes'] if s in selected) or 'baseline'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base')
    parser.add_argument('--head', default='HEAD')
    parser.add_argument('--full', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    paths = []
    if args.full:
        result = classify([], full=True)
    elif not args.base or set(args.base) == {'0'}:
        result = classify([], full=True)
    else:
        # Include deletions/renames; argv, never shell interpolation of filenames.
        data = subprocess.check_output(['git', 'diff', '--name-only', '-z', args.base, args.head])
        paths = data.decode().rstrip('\0').split('\0') if data else []
        result = classify(paths)
        heavy = [p for p in paths if not ignored(p)]
        if heavy and all(p in ('Cargo.toml', 'Cargo.lock') for p in heavy):
            diff_text = subprocess.check_output(
                ['git', 'diff', args.base, args.head, '--', 'Cargo.toml', 'Cargo.lock']).decode()
            if version_only_change(paths, diff_text):
                # Release metadata only: audit and Nix cannot be affected.
                result['audit'] = result['nix'] = False
    print(json.dumps(result, sort_keys=True))
    if args.output:
        with args.output.open('a') as out:
            for name, value in result.items():
                out.write(f'{name}={str(value).lower()}\n')
            out.write('scenes=' + scene_scope(paths, full=args.full or not args.base) + '\n')


if __name__ == '__main__':
    main()
