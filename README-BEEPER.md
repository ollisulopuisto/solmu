# Solmu + Beeper: current source checkpoint

October 28, 2026. Mac-target development source, not a verified Mac app or a
full-parity release. Solmu's egui interface uses the Beeper Client API rather
than direct WhatsApp transport when `SOLMU_BACKEND=beeper` is set. Without that
setting, upstream behavior remains.

See BACKEND.md for the complete backend/deployment status and remaining gates.

## Current scope

- Chats across connected accounts, including multiple accounts on one network.
- Network rail, compact global pins, authenticated avatars with initials fallback.
- Manual local person links: one timeline, up to 8 same-origin direct chats.
- Exact-source replies. Explicit crossover sends plain text without a quote.
- Explicit per-person primary chat for fresh composition; no silent fallback.
- Independent member history paging, partial-history warning and retry.
- Loaded-history search and exact-source filter. This is not remote full-history search.
- Near-viewport row layout and borrowed timeline sorting.
- Mouse reply menu/double-click and Cmd+Option+Shift+R on Mac (Ctrl+Alt+Shift+R elsewhere) to reply to the latest matching
  non-deleted loaded message. Narrow search/source first. This does not provide
  full keyboard transcript navigation or verified screen-reader support.

Linked outgoing attachments and typing remain disabled. ID-only edit, select
and translation actions are refused rather than routed by guess. No remote merge
endpoint is used. Local links do not alter the user's Beeper account.

## Validation and limits

Portable adapter/model and launcher/diagnostic/build-helper checks are available below. Targeted real GUI-library
assertions and Linux visual fixtures pass. The adapter harness deliberately
stubs unrelated native transport and encrypted archive leaves. GUI diagnostics
used unrelated transport/native-call stubs. Neither is a native Mac acceptance test.

Native Mac compilation/runtime, OS Keychain/restart durability, live bridges and
whole-app memory remain unchecked. Clippy has inherited warnings; this is not a
clean-lint claim. No real messages or account changes occurred during fixture tests.
See PARITY.md for the feature-by-feature status and examples/README.md for
benchmark limits. The upstream README is historical, not this fork's test report.

## Local checks

    ./contrib/beeper-tools/check-source.sh

This command stages current adapter/model sources into a temporary harness and
retains the explicit unrelated leaf stubs. Running Cargo directly in `harness/`
only tests the saved snapshot, not current app sources. Neither route is a full
native GUI or real-account acceptance test.

To build the full app on a suitably provisioned Mac, use the repository's Cargo
manifest. This source contains no Beeper token. Do not add tokens to tracked files.
A running local API and explicit access token are required for real accounts.
HEADLESS-SETUP.md describes a separate read-only Server trial. Do not install,
relink, migrate or add accounts merely to try the client.

## Mock interface

    uv run contrib/beeper-mock/mock_server.py

In a separate terminal:

    SOLMU_BACKEND=beeper BEEPER_DESKTOP_BASE_URL=http://127.0.0.1:8077 BEEPER_ACCESS_TOKEN=mock cargo run

The mock uses generic data and accepts explicit fixture sends. It does not prove
live-network parity. Startup/background reads use GET; explicit commands may
perform mutations. Do not point automated fixture send scripts at a real account.

Original ZapFast source: https://github.com/crmne/zapfast (MIT).
See LICENSE and THIRD-PARTY-NOTICES.md for attribution.

Named message links written as `Title (https://address)` show the title as a
clickable link. Hover shows the full address. Standalone URLs remain visible;
message copy/export retains the original text and address. Selecting only the
visible title copies that title, like other displayed text.

Supplied Beeper `links[]` metadata can show the first matching link's title and
summary as a preview card. The original message URL remains the click target,
even when metadata reports a redirected URL. This reads metadata already in
the message; it does not contact websites or download `img`/`favicon` URLs.
Messages without supplied metadata stay bare links. Local preview fetching is
a separate opt-in feature, not part of this mapping change.
