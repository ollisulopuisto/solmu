#!/usr/bin/env python3
"""CI-only macOS credentials: passwords travel over stdin/TTY, never argv."""

import os
import pty
import re
import select
import signal
import subprocess
import sys
import termios
import time


def quote(value):
    """Quote security(1)'s line protocol, refusing command separators."""
    if any(c in value for c in "\r\n\x00"):
        raise ValueError("Keychain input contains a forbidden line separator")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def child_env():
    return {k: v for k, v in os.environ.items() if k not in {
        "KEYCHAIN_PASSWORD", "P12_PASSWORD", "APPLE_APP_PASSWORD",
        "BUILD_CERTIFICATE_BASE64",
    }}


def security(*args):
    # One command per invocation prevents a later success masking a failure.
    # Interactive security may report command errors on stderr with exit zero.
    # Fail closed on all diagnostics; never print secret-bearing input/output.
    line = " ".join(quote(a) for a in args) + "\n"
    result = subprocess.run(
        ["/usr/bin/security", "-i"], input=line, text=True,
        capture_output=True, env=child_env(), check=False,
    )
    if result.returncode or (result.stderr or "").strip():
        raise RuntimeError("Keychain operation failed (output withheld)")
    return result.stdout or ""


def keychain_paths(output):
    # security prints each path in literal outer quotes, not shell escaping.
    # Strip only that pair, retaining embedded quotes and backslashes exactly.
    paths = []
    for raw_line in output.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if not (line.startswith('"/') and line.endswith('"')):
            raise RuntimeError("Unexpected keychain search-list output (withheld)")
        paths.append(line[1:-1])
    return paths


def import_certificate():
    keychain = os.environ["KEYCHAIN_PATH"]
    password = os.environ["KEYCHAIN_PASSWORD"]
    original = keychain_paths(security("list-keychains", "-d", "user"))
    security("create-keychain", "-p", password, keychain)
    security("set-keychain-settings", "-lut", "21600", keychain)
    security("unlock-keychain", "-p", password, keychain)
    security("import", os.environ["CERTIFICATE_PATH"], "-P",
             os.environ["P12_PASSWORD"], "-T", "/usr/bin/codesign", "-k", keychain)
    security("set-key-partition-list", "-S", "apple-tool:,apple:,codesign:",
             "-s", "-k", password, keychain)
    current = keychain_paths(security("list-keychains", "-d", "user"))
    paths = list(dict.fromkeys([*original, *current, keychain]))
    security("list-keychains", "-d", "user", "-s", *paths)


def store_notary_credentials():
    # notarytool documents a secure prompt when --password is omitted. Give it
    # a real TTY (getpass can use /dev/tty), with echo disabled before spawning.
    password = os.environ["APPLE_APP_PASSWORD"]
    quote(password)  # Refuse multiline input before interacting with a prompt.
    command = ["xcrun", "notarytool", "store-credentials", os.environ.get("NOTARY_PROFILE", "native-packages"),
               "--apple-id", os.environ["APPLE_ID"], "--team-id",
               os.environ["APPLE_TEAM_ID"], "--keychain", os.environ["KEYCHAIN_PATH"]]
    pid, master = pty.fork()
    if pid == 0:
        attrs = termios.tcgetattr(0)
        attrs[3] &= ~termios.ECHO
        termios.tcsetattr(0, termios.TCSANOW, attrs)
        # Fixed notarytool command; no shell or secret-bearing argument.
        os.execvpe(command[0], command, child_env())  # noqa: S606
    output = b""
    sent = False
    status = None
    deadline = time.monotonic() + 120
    try:
        while status is None:
            done, child_status = os.waitpid(pid, os.WNOHANG)
            if done:
                status = child_status
                break
            if time.monotonic() > deadline:
                raise RuntimeError("Notary credential prompt timed out")
            if select.select([master], [], [], 0.1)[0]:
                try:
                    chunk = os.read(master, 4096)
                except OSError:
                    continue
                output = (output + chunk)[-8192:]
                if not sent and re.search(rb"password[^\r\n]*:\s*$", output.lower()):
                    os.write(master, (password + "\n").encode())
                    sent = True
        if os.waitstatus_to_exitcode(status) != 0 or not sent:
            raise RuntimeError("Notary credential setup failed (output withheld)")
    finally:
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(master)


if __name__ == "__main__":
    if sys.argv[1:] == ["import-certificate"]:
        import_certificate()
    elif sys.argv[1:] == ["store-notary-credentials"]:
        store_notary_credentials()
    else:
        raise SystemExit("Expected import-certificate or store-notary-credentials")
