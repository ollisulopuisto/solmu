# Overnight source checkpoint, October 28, 2026

This section supersedes the older v10 snapshot below. Local source only, not a
published release. Mac-only release target; native Mac build/runtime not verified.

- Manual same-person links open one combined timeline with exact account/source
  labels, explicit primary chat and source reply override. No remote merge call.
- Loaded-history search and source filter do not change the sending destination.
- Linked outgoing text only. Attachments and typing remain blocked in linked view;
  incoming media uses the ordinary renderer. ID-only edit/selection/translation
  actions are refused rather than routed through the wrong member.
- Independent history errors/retries/older paging retain each member's identity.
- Linked layout skips distant rows using measured/cached heights. Actual generic
  2,002-message window inspected at bottom, scrolled up and searched. No total
  app/Mac RAM or frame-time acceptance claim.
- 163 adapter/model tests and 26 launcher/diagnostic/build-helper tests pass; real GUI-library browse and
  linked assertions pass. Full inherited GUI suite remains incomplete. Clippy on the portable harness completes with warnings, including inherited/stub
  dead code and style warnings. Full-project lint is not verified clean.
- In-session linked draft restoration checked. Restart/OS Keychain and real
  account sends remain unverified. No live account effects were made.

# Historical v10 snapshot (superseded by the overnight checkpoint above)

Rail network/account navigation now replaces Calls/dropdowns in Beeper mode.
Compact global pins remain visible across browsing scope; same-account identity
is in full hover/header labels, with short markers in compact controls.
Avatar pipeline uses Beeper's own authenticated asset service only, caps transfers
at2MiB/4Mpx input, normalizes PNG to max512px, uses shared disposable disk budget,
bounded8-job worker,4096 source hints and2048 UI entries with15-minute refresh.
HTTP/data/unsupported avatar sources keep initials; no third-party token fetches.
Generic fixture avatars visible in Linux list/header/pins. Mac/live avatars not
verified.132 adapter tests and real GUI-library browse assertions pass; latest
GUI binary builds. Full GUI unit suite remains incomplete.25 launcher/diagnostic/build-helper tests pass.
No real sends, native keyring, actual local AI or full headless/resource acceptance.
Merged timeline not implemented. CHANGELOG-SCOPE.md distinguishes next work/ideas.

# Historical Beeper parity ledger (v9, not current implementation status)

Checkpoint: 2026-10-27, v9 engineering slice. Not full parity.
Reference: 21 private user screenshots covering General, Appearance,
Notifications, Hotkeys, AI, Plus, Integrations, Storage and per-account settings.
Screenshots and personal content are deliberately absent from this package.

## Evidence levels

- Fixture-tested: compiled production adapter tested against a generic local API.
- GUI-tested: targeted tests use real app/UI/settings sources, with unrelated
  WhatsApp transport and native-call leaves explicitly stubbed.
- Visually checked: actual Linux egui window, not a drawn mockup.
- Unverified: real accounts, macOS, headless Server and total resource use.

124 adapter/model/account harness tests pass. 31 settings/search tests, three
new app behavior tests and one new notification behavior test pass in the real
GUI library. These are separate suites, not a claim of 155 independent cases:
some source coverage overlaps. Full inherited library suite was attempted but
not completed within the execution window. No real sends or account changes.

## Core messaging

| Feature | State | Remaining check |
|---|---|---|
| Unified multi-account inbox, including same network | Fixture-tested + visible | Real bridge inventory |
| Stable backend-origin/account identity and network/name labels | Fixture-tested | Accounts with no chats not yet listed in settings |
| Paginated chats and message history, older paging | Fixture-tested | Long real histories |
| Text send, same-chat reply, bridge send state | Fixture-tested + earlier GUI fixture | Real account/send authorization |
| Failed/uncertain send retains text/files | Fixture-tested + earlier GUI fixture | Real uncertain transport cases |
| Incoming media and emoji reactions | Fixture-tested; images visible | Audio/video playback and richer assets |
| File upload + send | Fixture-tested | Native picker-to-send GUI roundtrip |
| Archive/unarchive, pin, indefinite mute | Command-dispatch + SDK request fixture | Live network support, UI operation/result parity |
| Text edit, delete-for-me/everyone | Command-dispatch + SDK request fixture | Live support; pending-ID and network-specific constraints |
| Add reaction | Command-dispatch fixture | Real network support |
| Remove own reaction | Fixture roundtrip, key from current message/account | Real network and full GUI reaction coverage |
| Contacts, avatars, group/account administration | Missing or explicit refusal | Exact capabilities and separate confirmations |
| Calls, recording, clipboard image send, stickers, polls | Missing or explicit refusal | Native/media/backend work |

## General and Appearance references

| Reference behavior | State |
|---|---|
| Incognito: no automatic reads and no typing | Implemented; app tests + visual; explicit Mark read still works |
| Send read receipts / send typing | Existing account controls retained; read-receipts off now prevents API mark-read because API cannot separate the two |
| Compact list | Implemented; real pixels checked |
| Show message previews | Global + backend-origin/stable-account setting; visual checked |
| Show unread counts vs dot | Implemented; visual checked |
| Show network/account labels | Implemented; full identity retained in conversation header |
| Per-account Show in Inbox | All, unread, hidden; isolation test; search still finds hidden accounts |
| Per-account notifications / preview / sound | Implemented and behavior-tested; settings visible |
| Account identity/device status | Stable ID and supplied label shown; device administration unavailable, no fake remove button |
| Archive next chat; send and archive | Not complete |
| Chat reminders | Basic 1h/24h/clear menu, create/clear/invalid timestamp fixtures and read-only GUI gesture checked; actual reminder delivery unverified |
| Send later / undo-send delay | Not implemented; inspected SDK has no send-later field, separate design needed |
| Spaces, merged chats, topics, labels, blasts | Not complete; inspect exact API/capabilities before promising server behavior |
| Emoji replacement, copy formatting, composer toolbar variants | Inherited behavior only; screenshot parity incomplete |
| Themes, wallpaper, zoom, font | Existing native controls; not equivalent to all Beeper choices |
| Timestamps and system/chat events | Inherited defaults; screenshot-specific controls missing |
| Network icons/avatar variants/composer details | Not complete |
| Download folder, startup, close, keep running | Existing controls; macOS behavior unverified |
| Spellcheck | Native integration not implemented |
| Custom web CSS | Not transferable to native egui; native theme alternative needed |
| Touch ID | macOS-specific integration missing; ordinary local app lock is not equivalent |

## Notifications and Hotkeys

Baseline message/group notifications, global sound/content settings and local
account choices exist. Screenshot-specific message requests, focused-chat
notifications, reaction/typing alerts, delay, repeat and sound-volume parity are
not complete. Per-account privacy tests confirm suppressed previews do not
expose sender or text.

Existing Solmu shortcuts are not full Beeper keyboard parity. App/chat/
message/composer bindings, account/filter/space navigation and Texts.app presets
need an action registry, persisted remapping and conflict detection. Bindings
for unavailable features must not pretend to work.

## Local AI

User requests local API instead of OpenAI. Translation, talk-to-type,
voice-note transcription and mention assistance are NOT implemented yet.
No cloud requests, fallback or model downloads were added. Existing local
Wyoming/llama.cpp services should be reused, but actual endpoint/model/protocol
is unknown. Wyoming is not assumed to use an OpenAI-compatible API. Adapters
need bounded jobs, cancellable work, explicit on-demand content selection,
strict loopback or approved local-network routing, redirect refusal and tests.
No auto-send. Secrets must not enter source or chat.

## Disk, memory and responsiveness

Implemented: disposable Beeper media cache has a 256 MiB default budget with
16/64/128/256/512/1024/4096 MiB Settings choices. Only this adapter's `beeper-*`
regular files in its cache folder are eligible. Oldest files are evicted at
startup and before new transfers. The maximum transfer is reserved before
writing, including unknown-length streams. The byte budget also bounds an
in-flight media file; media above the budget is refused. No symlink following,
folder recursion, saved-download or message-database deletion. Clear and measure
operations are queued while a download holds the cache lock, without blocking
chat commands. Cache hit retention is oldest-first, not full LRU.

Settings show cache-only usage and distinguish it from backend storage. Clear
has a confirmation. Upstream recovery is conditional: evicted originals can be
fetched again only while Beeper or the network still holds them. This does not
limit or delete existing Beeper Desktop data, Server DB/media, user downloads,
model files, source/build artifacts or the whole disk footprint. Per-account
cache quotas and thumbnail/original tiers are future work.

Existing safeguards: separate bounded upload/download workers, viewport-aware
layout, inactive transcript caps, bounded image residency, JSON/file limits.
The active long-scrolled transcript may still grow; uploads make temporary
base64 copies. No production RAM or CPU savings measured. Benchmark complete
Desktop versus Server + bridges + native client with identical accounts and
workload. Polling less often does not sleep a bridge or prove memory savings.

## Entitlements and integration references

Plus screenshot is a feature reference, not purchase/trial approval. No paid
server features are unlocked or bypassed. The client consumes the user's local
API; it does not recreate Beeper's API/MCP administration UI. Remote API access
stays untouched. No account disconnect/migration/relinking is authorized.

SDK contracts checked against:
https://github.com/beeper/desktop-api-js
https://developers.beeper.com/desktop-api
The user's live local spec and network capabilities still need validation.

## Practical sequence

1. Finish safe storage and stable-account behavior validation; GET-only real
   account inventory and macOS smoke run. Keep backend compatibility separate
   from local features.
2. Finish command/UI result and failure-state coverage for everyday messaging,
   then reminders/send later/undo-send and keyboard/notification parity.
3. Implement protocol-verified local AI and richer media/native integration.
4. After parity, consider reply sessions, person views, saved views and topic
   assistance. These are ideas, not shipped features or approved auto-actions.

No dates are promised. Real compatibility and platform checks remain gates,
not reasons to stop independent local implementation.


## v5 delta over the delivered v4 checkpoint

- Per-account settings now use canonical endpoint origin + stable account ID.
  A targeted app test proves identical IDs on two backends remain isolated.
  v4 account-only choices are not silently migrated to a new backend.
- Own reaction removal now has an encoded-key/account-correct roundtrip test.
- Basic server reminder menu (1 hour,24 hours,clear), matching SDK shape.
  Invalid timestamps refused; read-only reminder gesture sends zero changes.
- Beeper chat menu hides unsupported WA favorite/label/lock/clear/delete-chat
  controls instead of presenting them as working. Timed mute is not offered.
- Unknown-length oversize media test verifies partial-file cleanup; final and
  temporary symlink cache entries are refused. Backend read-receipts setting
  off prevents API MarkRead since API does not separate network receipts.

124 adapter tests pass;31 settings/search tests and four app/notification tests
pass (five targeted parity tests include the preference case already among31).
Real Linux GUI builds. Actual compact/settings/account/cache/reminder pixels
inspected; read-only reminder gesture shows explicit refusal, zero HTTP writes.
No new real-account/Mac/headless/performance/AI claim.


## v6 delta over preserved v5 / user-delivered v4

- Two working notification choices: focused-chat alerts and ordinary group
  alerts. Group mentions/replies to you bypass the ordinary-group switch.
- Beeper edits keep the original transcript until the API returns the updated
  message. Failure/refusal retains proposal and exact message identity. It
  never overwrites a newer composer or converts a newer draft into an edit.
- Text drafts use a separate SQLCipher archive, scoped by backend origin/chat,
  loaded on startup and saved on SaveDraft. No plaintext fallback, no duplicate
  message history. Keyring failures are explicit and advise keeping the app
  open/copying unsaved words. Native OS-keyring runtime remains unverified.
  Encryption/durability test uses an explicit test key, not a working OS keyring.
-125 adapter/model/account tests pass. Eight targeted real-library parity
  tests pass, including encrypted draft restart/isolation and failed-edit/
  notification behavior.31 settings/search and two encrypted-draft tests also
  pass separately; these overlap the eight, do not add them as independent
  totals. Full inherited suite remains incomplete.
- Actual Linux notification settings pixels inspected. Real accounts,Mac,
  headless, reminder delivery,total resource use and local AI still unverified.


## v7 delta over preserved v6

Local AI text translation is now implemented as a first explicit on-demand
path, not full AI parity. Message menu stages only the selected message in
Settings for review. Nothing is sent until Translate is clicked. Settings
provide endpoint, existing model name and target language. Result is labeled
as generated, editable and copyable, never automatically sent to any chat.

This adapter uses llama.cpp's documented /v1/chat/completions route. It does
not assume Wyoming speech uses that protocol. No speech transcription,
talk-to-type or AI mentions yet. Endpoint defaults to loopback; public hosts,
DNS hostnames other than canonicalized localhost, credentials, query/fragment,
custom routes, redirects, proxies and IPv4-mapped IPv6 are refused. Private
LAN literal IP requires an explicit setting saying text leaves this computer.
No cloud fallback, Beeper token sharing, model download or automatic request.
No endpoint-auth support yet.1 job at a time,90s timeout,32KiB input and1MiB
response cap; no streaming or user cancel yet. Actual running local model,
quality and Olli's service protocol compatibility remain unverified.

Four adapter tests cover exact selected text/model, public/DNS/LAN boundaries,
redirect refusal and no network work for idle/invalid jobs. GUI staging test
confirms Translate action causes no backend command/request. Actual GUI roundtrip
to clearly labeled generic local fixture was inspected in pixels. Fixture
translation is NOT evidence of real inference. No source message/account names
are sent with the selected text. The actual input is shown before request.

129 adapter/model/account tests and9 targeted real-library parity tests pass.
Prior31 settings/search and2 encrypted-draft tests overlap these as documented.
Real Linux GUI builds; copied portable source build and harness rechecked.
Full inherited suite still incomplete. No real-account/Mac/headless/resource
validation. This latest section supersedes earlier historical AI-missing notes.

Protocol source inspected:
https://raw.githubusercontent.com/ggml-org/llama.cpp/master/tools/server/README.md


## v8 delta: actual Mac feedback and visible browsing fix

The user confirms v4 compiled/started on Apple Silicon and runs with Beeper
Desktop open. The initial transport failure resolved when Desktop was open.
This is real Desktop-backed startup evidence, not all-network mutation, native
keyring, headless or performance acceptance. User Activity Monitor snapshot
shows Solmu208.2MB plus Desktop339.4MB and one renderer311.3MB. This is a
partial single snapshot, not a total-process comparative benchmark. Keeping
Desktop running adds this UI rather than demonstrating a memory saving.

- Visible Network and Account dropdowns now browse the unified list. Exact API
  network is separate from decorated labels. Same-network accounts use stable
  origin+account IDs; search/archive remain inside the chosen scope, unread
  chips follow it. All networks/accounts resets the scope. Selection is local
  session state, not a remote mutation or a persistent preference.
- Authoritative account inventory includes accounts with no chats, supplied
  identity/network/status, and per-account settings. Missing network/status is
  labeled rather than guessed. Chat network fills older API omissions.
- HTML-looking message bodies become conservative styled text. No HTML engine,
  scripts/styles/iframe execution, remote assets, or markup-provided actions.
  Common/numeric entities and block/list boundaries are decoded. Bold, italic,
  strike, inline code, fenced blocks, blockquotes and list bullets survive as
  the WhatsApp-style markup the renderer already parses; properly nested tags
  style correctly, overlapping tags style by their spans, and unclosed tags
  degrade to literal characters. Markers that open or close mid-word carry a
  zero-width-space hint so the renderer's word-boundary rules accept them;
  styles spanning line breaks close and reopen per line; code content that
  holds backticks or fence lines is backslash-escaped, and the renderer
  resolves those escapes in inline code and fenced blocks alike.
  This is not full rich-text fidelity: links represented only in href may lose
  their URL, uncommon entities can remain literal, malformed markup may be
  imperfect. Parser behaviour is fixture-tested; only live captures per bridge
  prove every bridge sends the shapes the fixtures assume.
- Opaque Matrix-like IDs are no longer presented as human names. A supplied
  matching participant name can be used; otherwise label says name unavailable.
  This does not invent a contact name or prove contacts resolved on real data.
- Join/invite notice noise remains visible pending a reliable upstream event
  type contract. We do not delete/hide ordinary chat text based on English
  phrases that could be legitimate messages. Hidden/deleted API flags still
  honored. Full state-event presentation is not claimed.
- Launcher preflight separates unreachable endpoint from rejected token before
  Cargo/UI; loopback proxy bypass added in launcher and Rust API. No backend is
  installed/started automatically. Read-only/profile isolation retained.
- New --beeper-target reads only one explicit existing official CLI target;
  exact target ID/baseURL/token are validated privately. It never starts,
  creates, selects, installs or changes target/account. A selected target never
  inherits a possibly wrong Desktop token from shell. No secrets printed.
-131 adapter/model/account fixture tests and10 Python launcher tests pass.
  Real GUI-library assertion example passes network/same-network/origin/search/
  unread/chatless navigation. New unit-suite rebuild has not completed in this
  run; earlier9 unit-test results belong to v7, not rerun v8. Linux GUI builds.
  Network dropdown/selected-work-only and rich-text fallback pixels inspected.
  Native Mac verification of this new checkpoint remains pending.

See HEADLESS-SETUP.md for a supported fresh-session path. User says setting up
connections again is acceptable. No installations, new account connections,
Desktop removal/migration or real sends performed here. Server issue38 remains
open; Instagram/Facebook restart persistence and total memory are gates.


## v9 correction: supersedes briefly delivered v8

Final audit found v8 filtered chat-name search but not message search hits.
Main sent a hold-off warning after v8 delivery. v9 fixes the actual displayed
message-hit vector using current origin/account/network scope. Hits whose chat
is absent from inventory are excluded when scope is active. Contacts/self
suggestions without account identity are omitted in scoped searches, rather
than presented as belonging to the chosen account. Clearing scope restores
unified search. This is display filtering of the global API search response,
not a promise the server fetched only that account's messages.

The real GUI-library assertion example now includes cross-account and unknown-
chat search hits, and verifies changing scope does not alter open conversation
or existing composer ownership. Actual scoped history search shows only Work
hit; clicking opens that exact Work chat. Actual local-fixture send from this
search routes only to the Work chat path, and its confirmed bubble is visible.
Separate read-only gesture causes zero POSTs and restores text. These fixture
writes are not real account sends.131 harness tests and25 launcher/diagnostic/build-helper tests pass;
copied source Linux build/assertions also rechecked. GUI unit-suite rebuild
remains incomplete. Earlier Mac-v4 and headless limitations still apply.
