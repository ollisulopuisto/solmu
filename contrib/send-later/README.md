# Send Later: scheduler service setup

`solmu-scheduler` is a small companion daemon that owns the durable
Send Later queue. Solmu talks to it over a localhost socket; the
daemon talks to the Beeper API. Because the daemon holds the queue on
disk, scheduled messages go out even while Solmu is closed.

Pieces:

- `solmu-scheduler` - the daemon binary (built from this repo).
- A **control token**: a random secret the app and the daemon share,
  stored in a 0600 file. The daemon refuses to start without one and
  refuses any request that does not bear it. The token is never written
  into settings, logs, or the repository.
- A **launchd agent** (macOS) that keeps the daemon running.
- A **managed SSH tunnel** when the Beeper server runs on another
  machine (the Studio deploy).

## Local install (daemon on the same Mac as Solmu)

```
./contrib/send-later/install_macos.sh
```

The script:

1. builds `solmu-scheduler` (or copies a prebuilt one from
   `target/release/` if present) into `~/Library/Application Support/zapfast/bin/`;
2. generates a 256-bit control token into
   `~/Library/Application Support/zapfast/scheduler-token` with mode 0600
   (reusing an existing token if one is already there);
3. writes `~/Library/LaunchAgents/com.solmu.scheduler.plist`
   (KeepAlive, run at login, token passed via `SOLMU_CONTROL_TOKEN_FILE`,
   never on the command line or in the plist);
4. loads the agent.

Solmu reads the token from the same file
(`~/.config/zapfast/scheduler-token`, symlinked by the installer into the
app's config dir when it differs). Set `Settings -> scheduler_url` only
if you moved the socket off the default `http://127.0.0.1:23376`.

## Studio deploy (Beeper server on another machine)

The daemon should run **next to the Beeper server**, so message delivery
does not depend on the MacBook being awake. Solmu then reaches the
daemon through a managed SSH tunnel, the same pattern the Beeper API
tunnel (`127.0.0.1:23374`) already uses:

1. Install the daemon on the Studio machine (macOS: the same
   `install_macos.sh` / launchd agent applies; `install_linux.sh` installs the
   `solmu-scheduler.service` systemd user unit on a Linux host instead).
2. Give the MacBook a tunnel: `ssh -N -L 23376:127.0.0.1:23376 dst@192.168.1.222`,
   managed by the same mechanism that maintains the API tunnel.
3. Copy the token file to the MacBook's config dir (0600); both sides
   must hold the same secret.

The token travels the SSH tunnel only; the socket never leaves
localhost on either machine.

Actual Studio changes (installing the binary, creating the user unit,
extending the tunnel config) are a separate deploy step with its own
permission - this directory only prepares the artifacts.

## Queue durability and safety

- The queue lives in `queue.json` next to the token file, saved
  atomically (temp file + rename) after every change.
- A send attempt that ends without an answer (timeout, crash) becomes
  `uncertain` and is NEVER resent automatically - reconcile in the chat
  and retry by hand.
- A message due while the daemon was down becomes `missed`; it needs an
  explicit retry within the 15-minute grace window or it stays parked.
- Enqueue validates the chat and account against the Beeper API, so a
  message cannot be queued into a chat the account cannot send to.

## Bundled in the DMG (Studio deploy)

The current release workflow does not bundle the optional scheduler binary.
A custom bundle can include `solmu-scheduler` inside Solmu.app at
`Contents/MacOS/solmu-scheduler`. Do not assume an installed DMG contains it.
The launcher offers a one-time install on the Server Mac over the existing
SSH trust, only after explicit in-app approval: it checks the remote
architecture (Apple Silicon only), copies the daemon, writes the 0600
control token and the launchd agent, and kickstarts the service. Nothing
is installed or started without that approval.

## Stop and remove the macOS scheduler

Run these commands on the host where the scheduler was installed, not merely
on the client Mac. First inspect and reconcile any pending or uncertain sends.
Stopping the service prevents future queue processing; it cannot recall a send
already in flight.

```sh
launchctl bootout "gui/$(id -u)" "$HOME/Library/LaunchAgents/com.solmu.scheduler.plist"
```

Check that the service is no longer loaded before removing its files. Both the
standalone installer and launcher use **zapfast**, not **solmu**, for their
Application Support directory. Inspect the installed plist's ProgramArguments
before deleting it. The standalone installer names its binary `solmu-scheduler`;
the current launcher copy step names it `zapfast-scheduler` even though its
plist references `solmu-scheduler`. That installer mismatch is a separate code
issue, not a reason to create or start a second service during removal.

After the service is stopped, remove only its known files:

```sh
rm -f "$HOME/Library/LaunchAgents/com.solmu.scheduler.plist" \
  "$HOME/Library/Application Support/zapfast/bin/solmu-scheduler" \
  "$HOME/Library/Application Support/zapfast/bin/zapfast-scheduler" \
  "$HOME/.config/zapfast/scheduler-token" \
  "$HOME/Library/Application Support/zapfast/scheduler-token" \
  "$HOME/Library/Application Support/zapfast/beeper-token"
```

Do not delete the whole `zapfast` directory: other app data can live there.
`queue.json` and `scheduler.log` remain for reconciliation. They can contain
private data; inspect your retention/backup needs before removing them separately.
A client-side cached scheduler token may also remain in its isolated profile;
stop the client and remove that token if you intend to retire the connection.
Removing a local token copy does not revoke the Beeper API token at its source.
