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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base')
    parser.add_argument('--head', default='HEAD')
    parser.add_argument('--full', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
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


if __name__ == '__main__':
    main()
