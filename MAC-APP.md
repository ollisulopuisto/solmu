# Solmu - native Finder app

This is a source-backed tester app, not a verified native installer yet. The
current main CI/Nix package is not this Finder app or DMG.

On an Apple Silicon Mac with the Rust/Xcode prerequisites already installed:

    cargo build --locked --release --bin solmu
    python3 contrib/beeper-tools/package_app_dmg.py \
      --binary target/release/solmu --output dist/beeper-app-2026.10.30.1 \
      --version 2026.10.30.1 --commit "$(git rev-parse HEAD)"

This uses the committed original icon and native bundle tooling. The app has a
fork-specific bundle identifier, a compiled AppKit launcher and the real Rust
GUI executable. No Terminal, Python, shell script or Rust toolchain is needed
by the installed app. Python/Rust are build-time tools only. The build is ad-hoc
signed, unsigned by a Developer ID, and not notarized. Do not disable Gatekeeper
system-wide. Runtime, Dock identity, drag installation and SSH lifecycle still
need native verification.

Drag `Solmu.app` to Applications. On first open, enter the existing
Server Mac's hostname/IP and SSH user. SSH key authentication and trusted host
keys must already work. Close the previous manual tunnel on local port23374;
the app refuses to use an occupied port instead of borrowing an unverified
connection. The app reads exactly the existing `server` Beeper CLI target on
the remote Mac, pins it to loopback23374, then owns a private SSH tunnel while
the GUI runs. An unexpected endpoint or failed SSH request is an error, not a
reason to reset or relink accounts. No Server installer or account setup runs.

Only host, username and the explicit write-mode choice are saved in a private
launcher.json beside the existing `.zapfast-beeper-prototype` profile. API tokens
are not in the app or saved launcher configuration. They are passed in process
memory/environment to the GUI, as in the prior authenticated launcher. The
launcher does not print private Server output. The first-run checkbox defaults
to read-only; enable it for sending, typing and marking chats read.

The existing icon is preserved, not a claim of new branding. The upstream
bundle/login-agent identity is not reused. Existing stock Solmu is not replaced.
The Beeper backend does not implement stock updater commands; a fork-specific
update feed is not configured, so install future verified fork DMGs manually.

Native acceptance still required: compile the Swift launcher, inspect Finder and
Dock, launch from Applications, existing SSH/tunnel lifecycle and GUI exit,
unknown-host/key failure, occupied local port refusal, token privacy, and network
inventory/history/pagination on the existing Server. No claim that missing
WhatsApp/Signal or history404 has been fixed by packaging.

## App lock Touch ID and App Nap

When the app lock has a password, Settings > Privacy > "Unlock with Touch ID"
offers Touch ID on the lock screen. The password itself is stored in the macOS
keychain only after the next successful password unlock; turning the setting
off or changing the password forgets it. Every unlock still runs the same
PBKDF2 verifier - Touch ID fills the password, it never skips the check.

Solmu holds an NSProcessInfo activity while a window is shown, so macOS does
not put it into App Nap for someone looking at it (idle system sleep stays
allowed). With the window hidden to the tray or minimized the activity is
released and macOS may pause Solmu, so new messages can arrive late. Settings >
System > "Allow App Nap" lifts the activity in the foreground too.

## Closing and reopening the window

Cmd+W and the red close button both request the same window-close path.
With background mode enabled the app stays running. Cmd+Tab activates the
app and requests a window when none is visible; a Dock click uses the tray
reopen handler. Cmd+H is AppKit hiding, not window destruction.
