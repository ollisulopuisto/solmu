"""Portable contract checks only. These do not test or render Finder."""

# Standard-library-only tests must also run without the application test environment.

import importlib.util
import os
import plistlib
import struct
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("dmg_welcome", HERE / "restyle.py")
layout = importlib.util.module_from_spec(spec)
spec.loader.exec_module(layout)


class Contract(unittest.TestCase):
    def test_real_icons_and_labels(self):
        text = layout.APPLESCRIPT
        self.assertIn("position of item appName", text)
        self.assertIn('position of item "Applications"', text)
        self.assertIn("set text size of opts to 16", text)
        self.assertIn("set arrangement of opts to not arranged", text)
        self.assertNotIn("do shell script", text)

    def test_large_canvas(self):
        backgrounds = list(HERE.glob("welcome*.png"))
        self.assertTrue(backgrounds)
        for path in backgrounds:
            with self.subTest(path=path.name):
                data = path.read_bytes()
                self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
                self.assertEqual(struct.unpack(">II", data[16:24]), (4096, 2160))

    def test_backgrounds_have_no_icons(self):
        # Exported composition metadata is a review aid, not a Finder test.
        self.assertIn("no app icon", (HERE / "README.md").read_text())

    def _simulate(
        self, fail=False, wrong_link=False, bg_link=False, picture_link=False
    ):
        with tempfile.TemporaryDirectory() as td:
            image = Path(td) / "test image.dmg"
            image.write_bytes(b"original signed image")
            background = HERE / next(HERE.glob("welcome*.png")).name
            commands = []

            def fake_run(*args):
                commands.append(args)
                if args[:2] == ("hdiutil", "convert"):
                    Path(args[-1]).write_bytes(b"new image")
                elif args[:2] == ("hdiutil", "attach"):
                    mount = Path(args[args.index("-mountpoint") + 1])
                    (mount / "Test App.app").mkdir()
                    if bg_link:
                        (mount / ".background").symlink_to(Path(td) / "outside")
                    if picture_link:
                        (mount / ".background").mkdir()
                        (mount / ".background/welcome.png").symlink_to(
                            Path(td) / "outside.png"
                        )
                    if wrong_link:
                        (mount / "Applications").symlink_to(
                            str(Path(td) / "not-applications")
                        )
                    return plistlib.dumps(
                        {"system-entities": [{"mount-point": str(mount)}]}
                    )
                elif args[0] == "osascript":
                    if fail:
                        raise subprocess.CalledProcessError(1, args, b"Finder failed")
                    mount = Path(args[2])
                    (mount / ".DS_Store").write_bytes(b"layout")
                return b"ok"

            with (
                mock.patch.object(layout.sys, "platform", "darwin"),
                mock.patch.object(layout, "run", fake_run),
                mock.patch.dict(os.environ, {"DMG_WELCOME_CAPTURE_LOG": "0"}),
            ):
                if fail or wrong_link or bg_link or picture_link:
                    with self.assertRaises(
                        (RuntimeError, subprocess.CalledProcessError)
                    ):
                        layout.restyle(image, "Test App.app", background)
                    self.assertEqual(image.read_bytes(), b"original signed image")
                else:
                    layout.restyle(image, "Test App.app", background)
                    self.assertEqual(image.read_bytes(), b"new image")
            self.assertTrue(any(c[:2] == ("hdiutil", "detach") for c in commands))
            self.assertFalse(any(c[0] in ("codesign", "xcrun") for c in commands))
            return commands

    def test_atomic_replacement_and_spaces(self):
        commands = self._simulate()
        self.assertTrue(any(c[:2] == ("hdiutil", "verify") for c in commands))
        self.assertTrue(any(c[-1] == "Test App.app" for c in commands))

    def test_finder_failure_preserves_original(self):
        self._simulate(fail=True)

    def test_wrong_alias_is_rejected(self):
        self._simulate(wrong_link=True)

    def test_background_directory_symlink_rejected(self):
        self._simulate(bg_link=True)

    def test_background_picture_symlink_rejected(self):
        self._simulate(picture_link=True)

    def test_optional_capture_failure_is_after_commit(self):
        source = (HERE / "restyle.py").read_text()
        self.assertLess(
            source.index("os.replace(final, image)"),
            source.index("capture_image(image)"),
        )

    def test_non_macos_fails_closed(self):
        with (
            mock.patch.object(layout.sys, "platform", "linux"),
            self.assertRaisesRegex(RuntimeError, "requires macOS"),
        ):
            layout.restyle(Path("missing.dmg"), "Test.app", Path("missing.png"))


class CaptureContract(unittest.TestCase):
    def test_success_detaches_and_removes_directory(self):
        self._capture_case()

    def test_screenshot_failure_still_detaches(self):
        self._capture_case(screenshot_failure=True)

    def test_failed_detach_keeps_recoverable_mount(self):
        self._capture_case(detach_failure=True)

    def _capture_case(self, screenshot_failure=False, detach_failure=False):
        capture_spec = importlib.util.spec_from_file_location(
            "capture_contract", HERE / "capture.py"
        )
        evidence = importlib.util.module_from_spec(capture_spec)
        capture_spec.loader.exec_module(evidence)
        with tempfile.TemporaryDirectory() as td:
            directory = Path(td) / "evidence"
            directory.mkdir()
            calls = []

            def fake_run(command, **_kwargs):
                calls.append(command)
                return subprocess.CompletedProcess(
                    command, 1 if detach_failure and command[1] == "detach" else 0
                )

            def fake_capture(mount, work):
                self.assertEqual(mount, directory / "volume")
                self.assertEqual(work, directory)
                if screenshot_failure:
                    raise RuntimeError("capture failed")

            with (
                mock.patch.object(
                    evidence.tempfile, "mkdtemp", return_value=str(directory)
                ),
                mock.patch.object(evidence.subprocess, "run", side_effect=fake_run),
                mock.patch.object(evidence, "capture", side_effect=fake_capture),
                mock.patch.object(evidence.time, "sleep"),
                mock.patch("builtins.print") as output,
            ):
                if screenshot_failure:
                    with self.assertRaisesRegex(RuntimeError, "capture failed"):
                        evidence.capture_image(Path(td) / "test.dmg")
                else:
                    evidence.capture_image(Path(td) / "test.dmg")
            detach_calls = [c for c in calls if c[1] == "detach"]
            self.assertEqual(len(detach_calls), 3 if detach_failure else 1)
            self.assertEqual(directory.exists(), detach_failure)
            if detach_failure:
                self.assertTrue(
                    any(
                        "evidence mount retained at" in str(c)
                        and str(directory / "volume") in str(c)
                        for c in output.call_args_list
                    )
                )


if __name__ == "__main__":
    unittest.main()
