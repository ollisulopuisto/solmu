"""Optional macOS CI-only screenshot evidence, no Actions artifact storage.

Only captures the mounted DMG's Finder window, never the desktop. A capture
failure is explicit and does not pretend that visual acceptance passed.
"""

from __future__ import annotations

import base64
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


def capture(mount: Path, directory: Path) -> None:
    script_path = directory / "capture.applescript"
    try:
        for label, bounds in [
            ("initial", "{80, 80, 855, 620}"),
            ("enlarged", "{20, 20, 1300, 920}"),
        ]:
            script = """on run argv
      tell application "Finder"
        set rootFolder to POSIX file (item 1 of argv) as alias
        open rootFolder
        set w to container window of rootFolder
        activate
        set bounds of w to BOUNDS
        delay 3
        return id of w
      end tell
    end run
    """.replace("BOUNDS", bounds)
            script_path = directory / "capture.applescript"
            script_path.write_text(script)
            window_id = subprocess.check_output(
                ["/usr/bin/osascript", str(script_path), str(mount)], text=True
            ).strip()
            if not window_id.isdecimal():
                raise RuntimeError(
                    f"Finder did not return a numeric window ID: {window_id}"
                )
            screenshot = directory / (label + ".png")
            subprocess.run(
                [
                    "/usr/sbin/screencapture",
                    "-x",
                    "-o",
                    "-l",
                    window_id,
                    str(screenshot),
                ],
                check=True,
            )
            if not screenshot.is_file() or not screenshot.stat().st_size:
                raise RuntimeError("Screenshot did not produce a file")
            # An explicit opt-in for ephemeral public CI, not normal local builds.
            # Recover the bytes from the run log; do not allocate artifact retention.
            print("DMG_WELCOME_PNG_BEGIN " + label)
            print(base64.b64encode(screenshot.read_bytes()).decode())
            print("DMG_WELCOME_PNG_END " + label)
    finally:
        close = """on run argv
 tell application "Finder"
  close container window of (POSIX file (item 1 of argv) as alias)
 end tell
end run
"""
        script_path.write_text(close)
        subprocess.run(["/usr/bin/osascript", str(script_path), str(mount)], check=True)


def capture_image(image: Path) -> None:
    # Keep the directory recoverable if the OS refuses detach. Never remove a
    # still-mounted volume's files in a TemporaryDirectory cleanup.
    directory = Path(tempfile.mkdtemp(prefix="dmg-welcome-evidence-"))
    mount = directory / "volume"
    mount.mkdir()
    attached = False
    try:
        subprocess.run(
            [
                "/usr/bin/hdiutil",
                "attach",
                str(image),
                "-readonly",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
                str(mount),
            ],
            check=True,
        )
        attached = True
        capture(mount, directory)
    finally:
        if attached:
            for attempt in range(3):
                result = subprocess.run(
                    ["/usr/bin/hdiutil", "detach", str(mount)],
                    check=False,
                )
                if result.returncode == 0:
                    attached = False
                    break
                time.sleep(attempt + 1)
        if attached:
            print(
                f"DMG_WELCOME_CAPTURE_UNAVAILABLE: evidence mount retained at {mount}"
            )
        else:
            shutil.rmtree(directory)
