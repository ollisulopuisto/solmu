# Fast-beeper coordination and development checkpoint

## Routine Mac builds use Apple Silicon - November 8, 2026

Per maintainer request, manual Release runs default to arm64. Universal is an
explicit input and remains the tag-release setting. The build script does not
install/compile Intel when arm64 is selected. Matching packaging targets and
architecture-specific filenames are used; verification checks exact slices in
both launcher and client. Shell-stub regressions cover arm64, universal, invalid
input and wrong slices. The already running manual.20 universal build is left
alone to avoid discarding a long compile. No merge, release dispatch or tag is
part of this workflow-default PR.

## PR242 restored linked source - November 8, 2026

Live read scope records a first-arriving exact linked member when that update
will make the combined timeline valid, but does not mark it read yet. Only a
successful initial history response plus a visible linked-view render can
finish the read, rechecking identity, focus, lock, incognito and open chat. Existing known identities are never
replaced for eligibility. The same direct/unlocked/origin/unique/2-to-8 rules
and focus/incognito gates apply. Regression covers first restoration plus
unfocused, hidden, incognito, locked, foreign identity/origin, group and
non-live exclusions. The explicit transition table and verification map are in LINKED-READ.md.
Only a matching tagged fetch completion can authorize the loaded state;
manual read is separate from automatic read. Unlink retires all restoration
phases and reopens ordinary fetch/read behavior. Failed restoration owns its
retry visibility independently of paging errors. Scope invalidation and paging
success while Failed are covered in LINKED-READ.md and native app tests.
Restoration notice, partial-timeline template and retry button use gettext;
fallback lookup is tested across every offered locale.
No merge or release.

## PR241 review repair - November 8, 2026

Retry now carries display-name text and its mention selections through the
ordinary and linked composer paths, without changing empty-draft guards or
automatically sending. Original markup stays intact. Reaction pills reserve
room below group clocks instead of overlapping them. New status/retry strings
use gettext, including footer measurement, and retry has an accessible button
label. Regression tests cover mention round-trip/draft retention, reaction
separation in both themes and the accessible retry label. Repeated mentions now encode every occurrence, deduplicating recipient IDs;
identical names get distinct editable labels. Media failure measurement also
uses the translated label. Normal mention autocomplete assigns distinct
labels to same-name recipients, reuses a recipient's existing label, and
encodes longest labels first. Its regression selects A, B, A with the same
display name. A Tab-cycle regression also exposed a scrolled-off bubble mistaking the
header search control for its child by rectangle containment. Only visible
child-focus geometry may trigger that scroll. The full path audit also switched
mention encoding to original-draft spans only, so generated wire tokens cannot
be reinterpreted as another recipient name. Ambiguous legacy labels remain
literal rather than guessing. See MENTION-RETRY.md. No catalog changes, merge
or release. Full runtime suite remains a CI gate.

## PR240 follow-up: batch filename collisions - November 8, 2026

Keep cross-platform path sanitization from PR240. Plan safe names for the
whole Save all batch, assign numbered names to normalized collisions while
reserving original names, and preflight existing destinations and sources.
Use create_new to refuse a raced-in file rather than overwrite it. Tests cover
whitespace/path/case collisions with distinct bytes, reserved suffixes, and
existing-file/missing-source refusals before any writes. Later I/O failures
can still leave a partial batch; this is not a filesystem transaction.
Unix batch exports explicitly request mode 0600 before copying bytes,
with a regression that exports a 0644 source and checks owner-only output.
No UI, merge or release changes.

## PR217 contact-key cache check repair - November 7, 2026

Instinct updates the Option-returning filter_map to use ? for its missing
phone early return, as required by strict clippy. This is the sole failing
quality diagnostic. Preserve the original one-key-per-contact optimization,
stable ordering and all main changes from merged PR216. No layout, data or
protocol changes. Format/diff pass locally; full current-head CI is the gate.
Maintainer approved green merges before the next notarized DMG; later arriving
Bolt PRs are outside this build cutoff. No main push. Revert PR217 to roll back.

## PR216 cache repair - November 7, 2026

On bolt/perf-reader-registry-cache, Instinct fixes the missing registry_cache
field in the HTTP test fixture. The branch was already mergeable, but quality
and Ubuntu tests failed compiling that fixture. Review also found cached auth
skipped chmod checks and held its mutex over upstream I/O. Cache hits now
validate private mode and Unix file identity/change stamps, not mtime alone;
platforms without that identity reload each request. Missing/invalid files
never serve stale authority. Immutable Arc snapshots free the mutex before
upstream work. Tests warm the cache before chmod, preserve mtime across a
revoking replacement, remove/corrupt the registry, and assert the cache is
unlocked whenever upstream opens. No public API route or UI changes.

16 HTTP integration tests pass in a focused harness using actual reader/API/
scheduler modules with only unrelated GUI person-model types reduced. Strict
all-target/all-feature clippy, fmt and diff checks pass. Full native app test
linking exceeded local resource/time limits; CI still owns the full suite.
Original performance figures have not been remeasured after the safety repair.
Maintainer authorized merging this task's open PRs once green, then a new DMG;
no merge before that live check, and no main pushes. Revert PR216 to roll back.

## Cmd-Tab with a closed window - November 7, 2026

Branch: fix/macos-headless-activation, owned by Instinct. User reports manual.11
still leaves the window closed after Cmd-Tab. Native Rust/egui/winit, not a
Tauri/Electron app: Cmd-Tab activates; Dock/Finder has a separate reopen path.
Closing destroys the GUI, while the shell keeps AppKit pumping with the tray.
eframe caches the event loop and winit delegate, so delegate destruction is
not the cause. The activation callback still tested whether *any* AppKit
window was visible, including unrelated native status-item/panel windows.
That cannot establish whether the chat window exists. This is a source defect;
the user's diagnostic log is still needed to confirm this gate, rather than
missing notification delivery, is the cause on his Mac.

Activation now always queues ActivateWindow. The app uses its explicit
window_hidden state to reopen through the shell, or its own viewport's
minimized flag to restore. A normal visible-window activation sends no focus
or unminimize command, preserving native dialogs. Dock/Finder Show remains
unconditional and separate. No layout, messages, data, linking, dependencies,
launcher changes or build dispatch. Human-led merge only, no main push.

Five focused actual app/shell/GUI tests pass in a source copy omitting
unrelated test bodies: closed-shell reopen, minimized restore, no forced focus
for an existing window, existing shell lifecycle, existing keyboard activation.
Strict all-target/all-feature clippy, library check, formatting and diff checks
pass locally. Native action mapping assertion added; macOS runtime and actual
Cmd-Tab acceptance remain unverified. No new DMG requested or built.
Rollback: revert this PR; #210's explicit activation subscription stays.

## Optional queue counts - November 6, 2026

Branch: feat/queue-count-preference, owned by Instinct. Wireframe shared before
implementation and approved: existing four icons with small upper-right badges,
example `[All 12][Unread 3][Reply later 5][Read later 2]`. Maintainer confirmed
these are chats for now, not individual messages. Settings > Chats > Show queue
counts defaults On, independently of unread-message count badges. Off restores
the icon-only segments and skips the count passes. Zero has no visible badge;
99+ caps large badges while tooltip/accessibility includes the exact count.

Counts reuse the list's filtering for each queue, including query, facets,
network/account, hidden-inbox later exceptions, archive and lock boundaries;
linked people count once. No sorting, preview clones or message-history scans
are added for counting. Counts cover available local items, not a server total.
No list selection or scope is mutated. The setting is in always-reachable Chats
rather than the conditional Beeper card, verified by offline settings search.

Seven actual app/settings/GUI tests pass in a focused source copy with unrelated
test bodies omitted. Covers item vs message semantics, queue/list agreement,
linked deduplication, inbox-hidden later items, search/facets/archive/lock,
preference persistence, offline setting access, 99+ bounds and Off icon-only.
Full all-target/all-feature strict clippy, check, format and diff checks pass.
Actual synthetic app screenshots inspected On/Off dark, On light and settings;
normal labels show 12/3/5/2 without layout growth. Native Mac runtime unverified.
No real messages, account/data changes, new dependencies or remote calls.
Revert this PR to roll back. Other shortcut/ring/reopen work stays separate.

## Sidebar shortcut audit - November 6, 2026

Branch: fix/shortcut-collisions, owned by Instinct. No new controls or layout:
queue wireframe remains `[All] [Unread] [Reply later] [Read later]`; digits now
follow that visible order. Cmd+Option+3 previously toggled Favorites from the
old ChatFilter::EVERY ordering, not two simultaneously firing handlers.
Favorites moves to 5; Private/Groups/Channels stay 6/7/8 and Unreplied stays 9.
Network tooltips incorrectly advertised Cmd+Option while the exact handler
uses Option alone. Help said 1..0 although filter 0 is unassigned. Fixed both.
Off Mac, the network-present visible-row shortcut swallowed Ctrl+Shift+1
before the Chats destination handler; that digit is now reserved for Chats.
Other row digits are unchanged. Mac keeps Cmd+Control+digit for visible rows.
Updated linked latest-reply docs to Cmd+Option+Shift+R, added help, removed the
nonexistent Calls shortcut promise, and corrected off-Mac focus-message docs.

Six pure registry/menu/label tests executed against extracted actual source
pass, including platform binding uniqueness. Full all-target/all-feature
`cargo check` passes. New GUI mapping and network-present regressions compile,
but local execution and full clippy exceeded the two-minute per-command limit;
CI must establish their runtime/lint result. No native Mac interaction claimed.
No new dependencies, messages, account changes, main pushes or merge.
Rollback: revert this PR. Separate active-avatar ring and optional queue-count
badges are queued next; neither is included here.

## Active avatar ring - November 6, 2026

Branch: fix/active-avatar-ring, owned by Instinct. Replaces the hard-to-see
left-edge selected marker with a 2.5-point accent ring around the circle in
the collapsed rail and favorite-avatar bar. A 5-point backing stroke separates
the ring from photos and hover backgrounds. Badges paint after the ring so
unread counts, network marks and timer marks stay on top. The ring fits the
existing cells; no width or hit-target changes.

Wireframe: selected `[| (avatar)]` becomes `[((avatar))]`, unselected stays
`[(avatar)]`. Dark and light actual app captures and selected-unread hover
capture inspected with offline synthetic chats. Active avatar remains legible,
14 unread badge clear, no clipping. `active-avatar` demo page reproduces it.
`cargo check --all-targets --all-features`, format and diff checks pass. An
isolated actual helper shape test executes and passes. Full app ring/badge
regression now executes and passes alongside 43 shortcut tests in a focused
source copy. Strict all-target/all-feature clippy passes. Native Mac appearance
unverified.
No real messages, account changes, settings or new dependencies. Revert PR
to roll back. Optional queue-count preference follows separately.

## Bolt favorites-order repair - November 6, 2026

Existing PR214 originated from a google-labs-jules[bot] commit, opened under the
maintainer account. Instinct repaired its stale-parent PACKAGING conflict after
PR210 merged, preserving activation notes and removing inherited launcher/demo
changes from the PR diff. Kept the allocation-free ASCII query fast path, but
retained normalization for Unicode: a combining-mark-only query normalizes to
empty and must keep favorites order. Added a regression for empty, whitespace,
ASCII text, combining marks, accented text and CJK. No native/runtime claim.

## Local attachment actions - November 6, 2026

Branch: fix/image-viewer-actions, owned by Instinct. Copy image and Save as
were dispatched to the Beeper worker but fell through its ignored-command
arm, including in v0.24.0. Route decoded clipboard images and attachment
save dialogs through a local desktop worker before the network/offline gate.
Clipboard writes remain on the UI thread through the existing result event;
saves return success/error events, cancellation stays quiet. No new protocol,
permissions, entitlements, persisted state or real messages.

Tests cover real extensionless PNG decode, pixels, missing/corrupt files,
exact save bytes, cancellation, same-file saves, failures, batch names and
local command routing with a disconnected/offline backend. Native macOS
pasteboard and save-dialog interaction still need acceptance on a Mac.
Rollback: revert this focused PR. The running manual DMG build predates it.

## Active unread badge/filter lane - November 6, 2026

Claim: Dock/taskbar badge vs Unread filter agreement. Reported symptom: the
Dock badge showed 1 unread while the Unread filter said "No unread chats".
Branch: fix/unread-badge-filter-agreement. Root cause: the badge counted
every unarchived, unlocked, unmuted chat that looked unread, including
channels (which have their own chip and never appear under Unread) and chats
from Beeper accounts hidden from the inbox (BeeperInbox::None) - neither can
ever appear in the Unread filter, leaving a ghost count. The Unread chip
count also skipped the inbox-visibility rule the list applies.

Fix: one shared definition, `Chat::counts_unread` (looks unread, not a
channel), used by `ChatFilter::Unread` and by `Account::unread_chat_count`.
The badge additionally skips muted chats (they stay listed in Unread without
raising the badge, as in WhatsApp) and inbox-hidden chats; the chip count
(`App::unread_chats`) now applies the same inbox-visibility rule as the
list, except under the Reply later / Read later chips, which still list
hidden chats. The badge stays app-wide across notification-enabled accounts
while the filter shows the account on screen; a network/account scope
selection narrows the list only, by design. Regression test:
`the_badge_and_the_unread_filter_count_the_same_chats` in src/app.rs.

Human-led merge; no settings, release, or workflow changes. No concurrent
claim on these files was present in the open PR list at branch time.

## Active regional-locale lane - November 6, 2026

Claim: timestamps follow the OS regional locale (fi-FI report from the
maintainer: the bubble read "The day before yesterday 20:13" in English on a
Finnish system). Owned by Instinct on branch
feat/regional-locale-timestamps. No concurrent claim on src/util.rs was
present in the checkpoint or the open PR list (#193 release workflow,
#194 macOS activation).

Scope: src/util.rs bubble/chat/day/moment/full/copy stamps and
short/long dates take the OS region (new util::region, cached):
macOS NSLocale localeIdentifier, Windows GetUserDefaultLocaleName, Linux
LC_ALL/LC_TIME/LANG. Finnish and US English formats; other regions keep
today's UI-language behavior. The UI language stays the user's separate
choice. Finnish words (tänään/eilen/toissa päivänä, lowercase weekdays and
months, partitive months in long dates, essive weekdays with "klo") live in
util.rs, not the gettext catalogs. The day filter's month and weekday
headings and the last-seen sentence stay UI-language; its date follows the
region through regional_short_date. Tests cover fi-FI and en-US against a
fixed clock.

Rollback: revert the branch; no migrations or persisted state.

## Active CI/CD efficiency lane - October 29, 2026

Claim: CI trigger/path selection and explicit manual single-target trial/hotfix
build preparation. Scope requested by maintainer: shorten feedback and avoid
unneeded hosted-runner minutes without hiding failures or bypassing releases.
Branch: chore/ci-fast-feedback. Existing Windows x64 failure remains unresolved.
No concurrent CI-edit claim was present in the current checkpoint/open PR list.

Plan: portable checks first; select native checks from changed source paths;
retain Nix for runtime/source and build inputs; make duplicate release-mode trial builds explicit
manual choices rather than every PR and merge. Keep policy always reporting,
main native coverage for runtime changes, full manual CI and existing release
workflow unchanged. Uploads remain disabled unless separately approved by the
existing gate. Human-led merge, no settings changes or release execution.

Status: PR8 human-merged on main5d4651ac (earlier handoff variant).166
current-source Rust tests,37 Python tests,7 scope tests,3 workflow tests and
13 policy regression tests pass locally, with fmt/actionlint checks. Actual
Actions behavior and minute savings remain under observation. A minimal
follow-up isolates the daily audit's concurrency group from normal CI so an
audit cannot cancel a main native run. Existing token cannot publish workflow
changes; generated incremental patch supplied without changing permissions.
Rollback is a focused PR revert; no runtime/storage migrations are involved.

## Active packaging lane - October 29, 2026

Claim: local trial packaging and its verification, owned by the current
Solmu packaging contributor. PR6 merged on main e4a2e03. Follow-up PR7
(`fix/verify-mounted-trial-dmg`) adds mounted-image verification and detach
cleanup; it is review/CI work, not a native acceptance or release claim.
Coordinate before changing the same packaging helpers. No new messaging,
backend installation or product-name change is included in this lane.

Repository coordination reviewed: CHECKPOINT.md, QUEUE-1120.md,
CHANGELOG-SCOPE.md, PARITY.md, README-BEEPER.md, CONTRIBUTING.md,
PRODUCT.md, MAC-TRIAL.md, MAC-DMG-TRIAL.md, BINARY-BUILDS.md and the
GitHub contribution/board guidance. No current root AGENTS.md, CODEX.md,
ROADMAP.md, TODO or BUGS ledger is present. The inherited contribution and
product documents describe upstream WhatsApp-only scope; README-BEEPER.md and
this fork's explicit Beeper work describe the current adapter. This does not
claim a direct Matrix implementation or override safety/privacy guidance.

Progress and checks:
- PR5 restored the active source while preserving both repair archive trees.
- Current-source portable checks stage real adapter/model modules, avoiding the
  stale saved harness snapshot. Latest local run:166 Rust and37 Python tests pass.
- Earlier main49ef5e6 native trial compilation/package passed on Apple Silicon,
  Intel Mac and Linux; ordinary Mac/Ubuntu tests passed. This is not proof that
  later commits or an actual DMG/runtime pass.
- PR7 hdiutil mount/content/detach tests are mocked. A real disk image has not
  been created, mounted, visually inspected or launched with this helper.

Blockers and handoff:
- Windows x64 native compilation and ordinary tests failed on49ef5e6. Detailed
  logs were not retrievable; do not invent a root cause from exit codes.
- The Mac Nix job on that head was cancelled during package build, not passed.
- Artifact uploads remain disabled. Do not enable uploads, trigger releases or
  change billing/settings as part of this claim. No downloadable app/DMG exists
  from the trial workflow. Human-led merges only.
- Native DMG, Gatekeeper, launcher, Keychain/restart, existing Server/bridge and
  whole-stack resource acceptance still need separately scoped evidence.
- After PR7 checks/human review, record its actual result here. Further feature
  candidates in CHANGELOG-SCOPE.md are not an automatic implementation claim.

Reproduce current portable checks from the source root:

    ./contrib/beeper-tools/check-source.sh

Direct `cargo test` inside `harness/` tests its saved snapshot, not current app
sources. See MAC-DMG-TRIAL.md for the local trial-image interface and its limits.

## Historical October 28 checkpoint

The following is retained as historical evidence; its test counts and native
compilation statements are superseded by the dated lane above.

Source-only development trial, not an accepted standalone Mac app. Start with
README-BEEPER.md, then MAC-TRIAL.md for the explicit native build command.

## Checks on this source

- Portable adapter/model harness: 163 tests pass, using real adapter/model sources
  and explicit stubs for unrelated WhatsApp/native integrations.
- Python launcher, read-only diagnostic and Mac assembly helper: 26 tests pass.
- Development GUI library and binary build with real egui/UI sources and unrelated
  transport/native-call stubs. Targeted GUI diagnostics and Linux screenshots have
  been inspected; the full inherited GUI suite remains incomplete.
- Portable harness Clippy completes with warnings, not a clean whole-project lint.
- Current older-history regression checks missing/empty/repeated pagination
  cursors, preserving a partial state rather than claiming completeness.

## Native acceptance not performed

Native Mac build/runtime, OS Keychain restart persistence, live Server/network
connections, Desktop-quit/Server-restart behavior, whole-stack CPU/RAM and signing
are NOT RUN. There is no Server installer, sign-in/lifecycle manager or bundled
Server. Server redistribution terms remain unresolved. Unsupported linked outgoing
media and typing stay blocked. Remote search loads one page with partial labeling.

Use a separate prototype profile and an existing exact target only. The launcher
is read-only by default. This checkpoint does not authorize installing a service,
reconnecting accounts, remote exposure or sending real messages. Preserve existing
Beeper Desktop. See BACKEND.md and HEADLESS-SETUP.md for proposed trials and limits.

No binary, credentials, private conversations or overnight development screenshots are included. Inherited upstream demo screenshots and generic assets remain.
The SHA256 archive digest is an integrity check, not a signature.

## Reproduce portable checks (not native acceptance)

    cd harness
    cargo test --locked
    cargo clippy --locked --all-targets
    cd ../contrib/beeper-tools
    uv run --no-project python -m unittest discover -v

Do not mistake the portable harness stubs for the full native app. The top-level
Cargo.toml is the full native source build, described in MAC-TRIAL.md. The GUI
development binary used separate diagnostic dependency stubs and is not included.

History/single-message responses carrying an explicit different chatID are now
rejected before applying content or using media/reaction/confirmation data.
Sparse responses without chatID remain compatible. Local HTTP regression passes.

Control-worker channel closure now stops polling and invokes the queued-upload
stop guard; inflight requests may still finish. Red-green lifetime check passes.
The read-only diagnostic preserves current known account state/provider enums.
Wider inherited tests remain incomplete: library test compilation exceeded the
build window; presence integration tests need native transport APIs absent from
the diagnostic stubs. Portable/targeted tests do not cover those APIs.

Latest App-event recovery diagnostic checks that a late failed send/caption stays
copyable in the existing error list when newer current/parked drafts prevent
restoration. Empty composer restoration still works. Red-green assertions pass;
GUI library/binary build. This is in-session dismissible retention, not a durable
recovery archive: later errors can evict it. No new pixel/native validation.

## October 30 overnight follow-through (local review)

Combined branch `overnight/beeper-native-followthrough` retains both Server
sign-in and the private SSH target-stdin launcher, plus canonical Messenger
network aliases, optional bridge-type fallback and corrected product/settings
copy. Merge conflicts were resolved preserving both helper capabilities.
43 Python tests and 168 current-source adapter/model tests pass.
Native compile, updated UI pixels, history-404 diagnosis and full network
inventory are not verified. Messenger rendering was confirmed on the earlier
user build; missing Signal/WhatsApp and pagination completeness remain open.
No new branch push, hosted build, upload, merge or release was performed.
Existing trial workflow produces ZIP only, not a fresh DMG. GitHub zero-new-cost
restriction applies; storage liability is unverified, so builds remain held.

## Chat-list breathing room - November 6, 2026

Branch: ui/chat-list-breathing-room, owned by Instinct. Default chat rows
increase 68 -> 80 points (+17.6%), retaining 48-point avatars, two lines,
24-point line spacing, touching click surfaces and existing network badges.
The existing Compact chat list setting stays at 56 points. Both ordinary
and locked-folder virtual lists use the same height as their painted rows;
text is vertically centred at either density. Shared contact/search/action
row heights are unchanged. History review found the earlier spacing changes
were for settings and linked message timelines, not chat-list row height.
Changelog entry feeds What's New when promoted into the next release's
version section; unreleased notes intentionally do not appear automatically.
No migrations; rollback is this PR's revert.
