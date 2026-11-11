# Solmu

Solmu is a native desktop chat client for a Beeper account. It uses Rust and egui to show chats from the networks connected to your Beeper Desktop or Server endpoint, without embedding a browser engine.

This fork builds on [ZapFast](https://github.com/crmne/zapfast). Its macOS app adds a launcher that connects to an existing Beeper Server host through SSH. Solmu is not a Beeper Server distribution and does not connect directly to Beeper Cloud on its own.

## Install on macOS

Choose an available Solmu DMG from this repository's [Releases](https://github.com/ollisulopuisto/solmu/releases). Check its source commit, architecture and release notes. Routine manual builds target **Apple Silicon**; a universal build is a separate choice. Draft releases require repository access and can contain several older assets, so use the intended version, not an arbitrary file in the list.

1. Download the DMG. Compare its SHA-256 with the release asset digest or build attestation when provided: `shasum -a 256 path/to/Solmu.dmg`.
2. Open the image and drag **Solmu.app** to **Applications**.
3. Open Solmu and configure the existing Server connection as described below.

The current macOS release workflow signs with a Developer ID, notarizes the DMG, validates the stapled ticket, and checks the app inside it. This does not apply retroactively to older unsigned trial downloads or to an ad-hoc local build. Do not disable system-wide protections for an unverified file.

The repository also contains Linux and Windows build paths. The packaged Mac connection flow is the focus here; do not assume the inherited cross-platform packaging or old trial instructions provide the same setup.

## First connection

The packaged app expects:

- An already configured Beeper Server host, with its existing CLI target named `server` pointing to `http://127.0.0.1:23374`.
- Working SSH key authentication and a trusted host key between the client and Server host.
- That host remaining awake, running and reachable for live chat access.

Enter the Server host's SSH hostname/IP and your SSH username in the Connect window. The launcher reads the existing target and owns a loopback SSH tunnel. It does not install Beeper Server, link networks, reset accounts or ask for an SSH password. Close any old manual tunnel occupying local port `23374` first; an occupied port is an error, not a connection to borrow.

The packaged launcher starts **read-only** unless you explicitly enable write mode. Write mode permits actions such as sending, typing indicators and read-state changes. This is a client-side guard, not a less-powerful API token: treat the token as sensitive either way.

The launcher also recognizes the prepared Linux target-reader helper. Linux host deployment, network availability and permissions need their own setup; do not assume a Mac-only service such as iMessage can move to Linux. See [Linux server notes](contrib/linux-server/README.md).

The adapter can also use an existing Beeper Desktop/Server API directly. That is a separate developer/launcher route, not the packaged SSH connection above. It still requires the chosen endpoint and its authorization to remain available. See [the source launcher](contrib/beeper-tools/run_client.py).

## Features

- **Local flat groups.** Latest/Groups views, many-to-many local membership, additive drag with Undo and deduplicated unread counts. [Details and limits](docs/flat-groups.md).
- **Multi-network chat lists.** Account/network filters, unread and unreplied views, plus a persistent Reply Later queue.
- **Chat tabs and linked conversations.** Keep several chats open, or manually group related chats across accounts while preserving their source identity.
- **Reading and replying.** Message history, quoted replies, avatars, unread badges, text search and supported media/file handling through the Beeper API.
- **Search with limits shown.** Loaded-history search is distinct from API search. API searches fetch a bounded page and can be partial; linked-view API search targets the selected member rather than every linked source.
- **Draft safety.** Failed and uncertain sends are surfaced instead of blindly retried. Inspect history before retrying an uncertain send.
- **Desktop controls.** Keyboard shortcuts, light/dark themes, app lock and macOS background/window behavior.

Availability depends on what the endpoint and connected network support. A visible control or inherited ZapFast feature does not establish Beeper parity. For example, the current Beeper adapter refuses location sends and attachment sends with reply/mention/view-once metadata rather than pretending to send them. History pagination and network errors can leave content incomplete.

[PARITY.md](PARITY.md) and [BACKEND.md](BACKEND.md) contain detailed limits and historical acceptance notes. Some inherited documents still describe a WhatsApp-only client or older unsigned trials; the adapter, launcher and current build scripts are the authority for what this fork builds. Automated tests and notarization do not establish a successful live network login, complete history or delivery.

### Send Later

A separate `solmu-scheduler` daemon implements a durable queue against the Beeper API. It must run next to the backend and remain awake/reachable for delivery while the desktop is closed. A timeout or crash with an unknown send result is marked uncertain, not automatically resent.

Scheduler installation is an explicit deployment step. The Mac launcher has an approval-gated installer when a compatible scheduler binary is bundled; the current release workflow does **not** bundle that optional binary. Do not assume downloading a DMG installs or enables Send Later. See [scheduler setup and removal](contrib/send-later/README.md) and verify your setup before depending on scheduled delivery.

## Build from source

The repository pins Rust **1.98.0** in [`rust-toolchain.toml`](rust-toolchain.toml). Native builds need the platform's GUI, audio and media development libraries. On macOS, install Xcode Command Line Tools as well.

```sh
git clone https://github.com/ollisulopuisto/solmu.git
cd solmu
cargo build --locked --release --bin solmu
```

A bare developer binary is not the signed Finder app. To assemble the Beeper Mac app on a Mac:

```sh
bash packaging/macos/bundle.sh \
  target/release/solmu dist/Solmu.app 0.24.7 beeper
```

Without a configured Developer ID this produces an ad-hoc-signed app, not a notarized release. Version numbers in this example must match the version you build. The release workflow uses [`build-release.sh`](packaging/macos/build-release.sh), [`bundle.sh`](packaging/macos/bundle.sh) and the native-packages configuration to produce its DMG. Signing/notarization needs separately configured credentials; they are not part of this repository.

To exercise the adapter against generic local data, start the mock and then the client in another terminal:

```sh
uv run contrib/beeper-mock/mock_server.py
```

```sh
SOLMU_BACKEND=beeper \
BEEPER_DESKTOP_BASE_URL=http://127.0.0.1:8077 \
BEEPER_ACCESS_TOKEN=mock BEEPER_READONLY=1 cargo run --bin solmu
```

`mock` is a fixture credential, not an instruction to put a real token in shell history. The mock cannot establish real-network behavior.

Common source checks:

```sh
cargo fmt --all --check
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
cargo clippy --locked --all-targets -- -D warnings
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the larger check matrix. `contrib/beeper-tools/check-source.sh` is a portable adapter harness, not a substitute for the full native app build or installed-app acceptance.

## Privacy and security

The client is native, but your messaging still depends on Beeper and its network bridges. Solmu does not replace their service, encryption or retention rules. The backend host can access the data and credentials needed to serve your chats; protect it and its backups.

The Mac SSH launcher saves its host, username and write-mode choice locally. It obtains the API token from the existing remote target and passes it to the client in process environment/memory, not in its saved launcher configuration. Other in-app authorization paths use the OS keyring and refuse a plaintext fallback. These are different paths, not a promise that every token is always stored in Keychain.

Local profiles, caches, drafts and scheduler queues contain private data. App lock is not a replacement for disk encryption, a secure OS account or backend access control. Optional media, GIF and local-model features have their own network/data paths; inspect settings before using them with private content.

Direct API configuration accepts loopback HTTP or remote HTTPS and does not follow redirects with the bearer token. Do not expose a Beeper API, scheduler socket or token publicly. Never post real conversations, contact names, tokens, recovery keys, QR codes or unredacted logs in an issue.

## License and credits

[MIT License](LICENSE), with upstream ZapFast credit retained. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for dependency notices. Beeper Server and connected network services are separate dependencies, not software licensed or redistributed by this README.
