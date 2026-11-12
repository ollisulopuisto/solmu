# Roadmap

Ideas and planned work. An entry here is not a commitment or a schedule.

## Standalone Beeper-less mode (direct Matrix login, E2EE keys, bridges)

Status: idea, not scheduled. Requested by the maintainer.

Goal: use Solmu as a regular Beeper client, logging into a Beeper account directly, without Beeper Desktop or a Beeper Server running somewhere.

Notes from the current source (README.md, SERVER-SIGN-IN.md, src/beeper_signin.rs, src/backend/beeper.rs):

- Solmu is a front end for the Beeper Desktop/Server HTTP API (`/v1/accounts`, `/v1/chats`, `/v1/chats/{id}/messages`, assets) with a Bearer token. Defaults are `http://127.0.0.1:23373` (Desktop) and `127.0.0.1:23374` (Server).
- It has no Matrix client, no bridges of its own and no direct connection to Beeper Cloud.
- The packaged Mac app starts a launcher that tunnels over SSH to a host running Beeper Server. It is read-only unless write mode is enabled.
- The in-app sign-in only signs an already installed, running local Server into an existing Beeper account (email code, then verification from a verified client or a recovery key). It refuses registration and does not install Server or link networks.
- Beeper Desktop's local API can be used directly as a developer route when it is enabled and a token is available.

What a standalone mode would need:

- Matrix login against the Beeper homeserver.
- End-to-end encryption: device keys, cross-signing, key backup and verification.
- Bridges for the chat networks, or another way to reach them.

This is a large project, not a small feature.

Not verified: nothing here was run, and Beeper's current API and login options were not checked against their documentation.
