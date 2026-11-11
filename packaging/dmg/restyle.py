#!/usr/bin/env python3
"""Apply direction A before signing/notarizing a DMG. macOS system tools only."""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile

# Finder owns both real icons and their labels. None are painted into welcome.png.
APPLESCRIPT = '''on run argv
    set rootPath to item 1 of argv
    set appName to item 2 of argv
    tell application "Finder"
        set rootFolder to POSIX file rootPath as alias
        open rootFolder
        set w to container window of rootFolder
        set current view of w to icon view
        set toolbar visible of w to false
        set statusbar visible of w to false
        set bounds of w to {80, 80, 855, 620}
        set opts to icon view options of w
        set arrangement of opts to not arranged
        set icon size of opts to 128
        set text size of opts to 16
        -- Preserve a matching colour if Finder uses it beyond the picture.
        -- The large picture, not this fallback, is the primary coverage method.
        set background color of opts to {5911, 6682, 8224}
        set background picture of opts to POSIX file (rootPath & "/.background/welcome.png")
        set position of item appName of rootFolder to {199, 247}
        set position of item "Applications" of rootFolder to {576, 247}
        close w
        open rootFolder
        delay 2
        set bounds of container window of rootFolder to {80, 80, 855, 620}
        update rootFolder without registering applications
        delay 3
        close container window of rootFolder
    end tell
end run
'''


def run(*args: str) -> bytes:
    return subprocess.check_output(args, stderr=subprocess.STDOUT)


def restyle(image: Path, app_name: str, background: Path) -> None:
    if sys.platform != 'darwin':
        raise RuntimeError('Finder DMG styling requires macOS; no image was changed')
    image = image.resolve(strict=True)
    background = background.resolve(strict=True)
    if Path(app_name).name != app_name or not app_name.endswith('.app'):
        raise ValueError('Expected the exact top-level .app filename')
    # A sibling temporary directory makes final replacement atomic. A failure at
    # any earlier stage leaves the original DMG intact, including its signature.
    with tempfile.TemporaryDirectory(prefix='.dmg-welcome-', dir=image.parent) as td:
        work = Path(td)
        writable = work / 'writable.dmg'
        mounted = work / 'mounted'
        mounted.mkdir()
        print(run('hdiutil', 'convert', str(image), '-format', 'UDRW', '-o', str(writable)).decode())
        attached = False
        try:
            result = plistlib.loads(run('hdiutil', 'attach', str(writable), '-readwrite', '-nobrowse',
                '-noautoopen', '-mountpoint', str(mounted), '-plist'))
            attached = True
            if not any(e.get('mount-point') == str(mounted) for e in result.get('system-entities', [])):
                raise RuntimeError('hdiutil did not mount the requested image')
            if not (mounted / app_name).is_dir():
                raise RuntimeError(f'Mounted image is missing {app_name}')
            applications = mounted / 'Applications'
            if applications.is_symlink():
                if os.readlink(applications) != '/Applications':
                    raise RuntimeError('Unexpected Applications link target')
            elif applications.exists():
                raise RuntimeError('Applications exists but is not the expected symlink')
            else:
                applications.symlink_to('/Applications')
            extras = [p.name for p in mounted.iterdir()
                      if not p.name.startswith('.') and p.name not in (app_name, 'Applications')]
            if extras:
                raise RuntimeError(f'Unexpected visible payload files need an explicit layout: {extras}')
            bg_dir = mounted / '.background'
            bg_dir.mkdir(exist_ok=True)
            shutil.copyfile(background, bg_dir / 'welcome.png')
            script = work / 'layout.applescript'
            script.write_text(APPLESCRIPT)
            print(run('osascript', str(script), str(mounted), app_name).decode())
            ds_store = mounted / '.DS_Store'
            if not ds_store.is_file() or ds_store.stat().st_size == 0:
                raise RuntimeError('Finder did not save the layout')
            run('sync')
            run('hdiutil', 'detach', str(mounted))
            attached = False
            final = work / 'styled.dmg'
            print(run('hdiutil', 'convert', str(writable), '-format', 'UDZO', '-imagekey',
                'zlib-level=9', '-o', str(final)).decode())
            print(run('hdiutil', 'verify', str(final)).decode())
            if os.environ.get('DMG_WELCOME_CAPTURE_LOG') == '1' and os.environ.get('GITHUB_ACTIONS') == 'true':
                # Capture the FINAL read-only image, not just the source artwork.
                from capture import capture
                evidence_mount = work / 'evidence'
                evidence_mount.mkdir()
                evidence_attached = False
                try:
                    run('hdiutil', 'attach', str(final), '-readonly', '-nobrowse',
                        '-noautoopen', '-mountpoint', str(evidence_mount))
                    evidence_attached = True
                    capture(evidence_mount, work)
                except (OSError, RuntimeError, subprocess.CalledProcessError) as exc:
                    print(f'DMG_WELCOME_CAPTURE_UNAVAILABLE: {exc}')
                finally:
                    if evidence_attached:
                        run('hdiutil', 'detach', str(evidence_mount))
            os.replace(final, image)
        finally:
            if attached:
                # Do not force-detach a possibly busy disk. Failure is visible.
                run('hdiutil', 'detach', str(mounted))
    print(f'DMG welcome layout saved: {image.name}; sign/notarize only after this step')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image', type=Path)
    parser.add_argument('app_name')
    parser.add_argument('background', type=Path)
    args = parser.parse_args()
    try:
        restyle(args.image, args.app_name, args.background)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as exc:
        if isinstance(exc, subprocess.CalledProcessError):
            print(exc.output.decode(errors='replace'), file=sys.stderr)
        parser.exit(1, f'DMG styling failed: {exc}\n')


if __name__ == '__main__':
    main()
