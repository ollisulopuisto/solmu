#!/usr/bin/env python3
"""Bounded apt install with an official Ubuntu mirror fallback for CI."""
import pathlib
import subprocess
import sys


def use_official_mirrors(root):
    apt = root / 'etc/apt'
    paths = [apt / 'sources.list', apt / 'apt-mirrors.txt', apt / 'apt-mirrors-security.txt']
    paths += sorted((apt / 'sources.list.d').glob('*.list'))
    paths += sorted((apt / 'sources.list.d').glob('*.sources'))
    replacements = {
        'mirror+file:/etc/apt/apt-mirrors-security.txt': 'http://security.ubuntu.com/ubuntu',
        'mirror+file:/etc/apt/apt-mirrors.txt': 'http://archive.ubuntu.com/ubuntu',
        'azure.archive.ubuntu.com': 'archive.ubuntu.com',
    }
    changed = 0
    for path in paths:
        if not path.is_file():
            continue
        old = path.read_text()
        new = old
        for before, after in replacements.items():
            new = new.replace(before, after)
        if new != old:
            path.write_text(new)
            changed += 1
    return changed


def install(packages, root=pathlib.Path('/'), run=subprocess.run):
    options = ['-o', 'Acquire::Retries=2', '-o', 'Acquire::http::Timeout=15',
               '-o', 'Acquire::https::Timeout=15']
    for attempt in range(2):
        try:
            run(['apt-get', *options, '-o', 'APT::Update::Error-Mode=any', 'update'],
                check=True, timeout=90)
            run(['apt-get', *options, 'install', '-y', '--no-install-recommends', *packages],
                check=True, timeout=180)
            return
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            if attempt:
                raise
            # Never change suites, components, signing keys or verification.
            print('apt failed; retrying once with official Ubuntu mirrors', flush=True)
            use_official_mirrors(root)


if __name__ == '__main__':
    if len(sys.argv) < 2:
        raise SystemExit('Pass the build dependency package names')
    install(sys.argv[1:])
