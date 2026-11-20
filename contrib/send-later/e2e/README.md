# Send Later end-to-end tests

Runs the real `solmu-scheduler` daemon, built from this repo's sources,
against a stub Beeper Server API, and checks the behaviors Send Later's
safety rules promise. No network, no credentials, no real chats.

    contrib/send-later/e2e/run-e2e.sh

What the suite covers (25 checks):

- Socket hardening: POSTs bearing an Origin header, a missing control
  token, or a non-JSON content type are refused.
- Schedule and deliver: the due message goes out exactly once, with the
  exact words, and the returned message id is recorded.
- Cancel before due never sends; edits change what is delivered.
- A definite rejection (HTTP error) marks the item Failed with the reason
  and is never retried automatically.
- kill -9 mid-send: the item stays claimed on disk, restarts as Uncertain,
  and is never resent automatically. `reconcile` checks the chat history
  (`found` settles it Sent, `not_found` leaves it Uncertain) and refuses
  non-Uncertain items. `retry` of an Uncertain item needs an explicit
  `{"confirm_duplicate":true}` because a copy may already have reached
  the chat; a confirmed retry resends.
- A clean restart still fires queued items on time.

Pieces:

- `run-e2e.sh` builds the daemon in a small harness crate under
  `/tmp/scheduler-harness` (overridable with `SL_E2E_HARNESS`) so the app
  itself does not need to compile, then invokes `cases.sh`.
- `cases.sh` starts `stub.py` and the daemon on localhost and asserts the
  behaviors above. Override `SL_E2E_WORK`, `SL_E2E_STUB_PORT`,
  `SL_E2E_DAEMON_PORT` for parallel runs.
- `stub.py` is a minimal Beeper Server API: chat lookup, message search,
  and sends, with `hang`/`reject` modes driven by `$SL_E2E_WORK/mode` to
  exercise crash and rejection paths.

The daemon's own unit tests (`src/scheduler.rs`, `src/scheduler_api.rs`)
cover the same rules at the module level; this suite proves the assembled
binary keeps them.

### Closed-client account-routing regression

`closed_client.py --binary /path/to/solmu-scheduler` starts the real daemon
with a synthetic loopback Beeper API. It verifies that a local profile ID is
refused, then a separate enqueue client schedules with the chat's bridge
account and exits before the due time. The daemon must deliver the exact
text once and persist Sent after that client has exited. It uses temporary
queue files and fixture tokens, never real accounts. This is not proof of a
particular Studio installation, SSH configuration or real-network delivery.
The main e2e runner invokes this check after its existing cases.
