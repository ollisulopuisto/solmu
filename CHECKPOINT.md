# Fast-beeper coordination and development checkpoint

## Overflow text: truncation, not scrolling - November 14, 2026

Branch `fix/overflow-text-truncation`, based on main 645b11f, version 0.24.23
(0.24.22 is open PR fix/repaint-pacing, so expect a version/CHANGELOG conflict
in whichever merges later). widgets.rs `scrolling_text` panned an overflowing
line end to end forever - decorative motion that forced repaints the whole
time a long title was visible, the clearest "sugar" in the design-principles
review. Replaced by `truncated_text`: the line ends in an ellipsis (the
existing `line()` helper, RTL-aware), shows in full on hover, and never
schedules a repaint; a test asserts a wide line in a narrow room asks for no
further frames. Only caller was the image viewer's title. The unused fade
helpers stay, they are shared pub API. Not compiled locally (no Rust
toolchain in this checkout), CI is the check. No merge by the author. Devin round: the no-repaint test now also
asserts the ellipsis truncation and the hover tooltip itself.


## Automatic chat categories - November 10, 2026

Branch `feat/chat-auto-categories`, based on main c4e24cc, version 0.24.18. First slice of option 4: Filters > Automatic category (People, Quiet people 7+ days, Long quiet 30+ days, Notifications) from plain rules in `src/categories.rs`, plus Move to... / Sort automatically on a one-to-one chat's menu, stored in `beeper.category_overrides`. Quiet tiers fade in search. Tests in `src/categories.rs`, `src/ui/chats.rs` and `tests/chat_scope.rs`. Not compiled locally (no libpipewire headers here); CI is the first compile. No demo captures yet. Details and gaps: docs/chat-categories.md. No merge by the author.

## Beeper attachment replies - November 10, 2026

Branch `feat/beeper-attachment-replies`, based on main 0486c64, version 0.24.12 (open PRs #264 and #265 are on 0.24.11 without bumps, so no version conflict expected). The Beeper backend refused or kept any file, document or pasted picture sent as a reply: SendFiles and the pasted-image gate treated a quote like the genuinely unsupported mentions and view-once. The Beeper message API takes an attachment and replyToMessageID in one body (voice notes already send that way), so the upload batch now carries the quote, the worker sets it on the first file only, and a failed upload returns the quote to the composer with the kept files, like a failed voice note does. The kept-files toast now names only mentions and view-once. Tests: a quoted pasted image passes the unsupported gate, and a quoted document's request body carries the attachment and replyToMessageID together. Not run locally (no Rust toolchain in this checkout), CI is the check. Unverified: a live bridge accepting reply plus attachment on every network. No merge by the author.

## Encoded-control decode cap fails closed - November 10, 2026

Branch `sentinel-fix-nested-encoded-control-13799924842503889086`, rebased on main (version 0.24.11). Sentinel's 16-layer nested percent-decode in has_encoded_control returned false when decoding was still changing at the bound, so a 17+ layer encoded control (e.g. 17x nested %00) passed external_url/preview_url validation. The loop now returns true when it exhausts its layer budget mid-decode; a boundary test builds a 17-layer nested control and a 16-layer control that must still be caught. Merged main in; the branch's older duplicate of #259's merged.rs change disappears against main. Not run locally (no Rust toolchain in this checkout), CI is the check. No merge by the author.

## Command palette polish - November 9, 2026

Branch `feat/palette-polish`, based on main, version 0.24.11 (0.24.9 and 0.24.10 are open PRs #259 and #260, so expect a version/CHANGELOG conflict in whichever merges later). From the unsung article on command palettes: rows show their key, Jump to open chat (Ctrl+K) is listed, and a filter row keeps the palette open. Ctrl+K stays the chat switcher, so the palette stays on Ctrl+P. Devin round: dialog-opening commands no longer queue CloseDialog (it ran second and closed the new dialog), filters re-open the palette after Open(Chats) when not already on the list, Search chats shows Ctrl+Shift+F. Action-dispatch tests in src/ui/palette.rs assert app.dialog after the queue runs, from Chats and Settings; not run locally (no Rust toolchain in this checkout), CI is the check. No merge by the author.
## Quoted and media heads, receipts in a run - November 9, 2026

Branch `fix/reply-bubble-stacking`, based on main ba51fa1, version 0.24.10 (0.24.9 is PR #259). stacks_with_limit let only plain
text stack and required identical delivery status, so a reply, picture, GIF or video head and a run whose
receipts differed (Read, Delivered, Sent) each broke the stack. Plain text, quoted replies, pictures and videos
now join a run in any position; own Sent/Delivered/Read rows stack. Cadence limits, album members, Unknown,
Pending and Failed remain boundaries. Messages bubble style still draws no stacks.
Not compiled locally (no Rust toolchain); CI is the check. Author does not merge.

## Linked reply arrow focuses the composer - November 9, 2026

Branch `fix/linked-reply-arrow-focus`, based on main ba51fa1, version 0.24.9. In a linked-person
view the bubble reply arrow, double-click and Reply to latest match started the quote but left the
composer unfocused; Cmd-R focused it. All of these now share one helper that also requests composer
focus. Test added in src/ui/merged.rs; not run locally (no Rust toolchain in this checkout), CI is the
check. Context-menu and other non-linked Reply entry points already set focus through Action::Reply.
No merge by the author.

## v0.24.7 release preparation - November 8, 2026

Separate version/changelog PR rebased on merged main 0ff5f22 (includes #248 and #249). Move the existing Unreleased notes into v0.24.7 and
include Cmd-R native replies and the merged stepped-width behavior so the existing automatic What's New
and Help menu display the new features. Cargo.toml and Cargo.lock agree.
No new UI, backend operation or packaging path. Maintainer merges this PR.

After merge, re-read latest main and use the existing Release workflow with
macos_arch=arm64, standard hosted runners on the public repository and direct
release assets only. No Actions artifacts or multi-platform tag run. Signing,
notarization and mounted-DMG verification remain workflow gates. No release
is claimed before those gates and release-asset verification finish.

## Cmd-R replies on the latest incoming bridge - November 8, 2026

Separate branch `feat/reply-latest-incoming-bridge`, based on main 3cea59b.
Cmd-R (Ctrl-R elsewhere) in a valid linked-person view selects the newest
loaded incoming, non-revoked message across valid member chats, quotes it on
its exact source account/chat and focuses the composer. Newer outgoing rows,
search text and source filters do not select a different message. Timestamp
ties use the timeline's chat/history-order/message-id ordering. No incoming
loaded means no route change. The saved primary destination is unchanged.
Existing reply, edit or staged attachment must be finished/cancelled first;
modal dialogs, previews, recording and popups do not retarget the composer.
No send, remote fetch, new settings or backend operation is added.

Plain Cmd-R previously had no binding. The existing Cmd-Option-Shift-R
filtered-search reply and selected-message reply actions keep their semantics.
This is a keyboard behavior change using the existing composer and quote UI,
not a layout redesign. Synthetic actual keyboard captures were inspected in
both themes: Ada's incoming Signal message is quoted despite newer outgoing
messages. No real conversation screenshot or review media is committed.

Validation: focused native target runs production code unchanged with unrelated
test modules omitted. Full linked-view and keyboard modules pass (66 tests,
1 ignored); strict all-feature,
all-target clippy, fmt and demo build pass. Full runtime CI remains required.
AGENTS.md and dedicated TODO/BUGS/ROADMAP ledgers are absent; existing fork
README/routing docs take precedence over stale inherited WhatsApp-only text.

Review repair: egui shortcut matching accepts extra modifiers. Consume only
key-press events whose modifiers exactly match plain Cmd/Ctrl-R, leaving
Cmd-Option-Shift-R for the existing filtered handler. UI regression runs both
handlers with a matching WhatsApp message and a newer unrelated incoming
Signal message; filtered reply keeps WhatsApp. Cmd-Shift-R and Cmd-Option-R
also leave routing unchanged. 66 focused native tests pass, 1 ignored.

To do: full CI and review on the PR head; maintainer merge only. No merge,
release or DMG dispatched. Revert the focused PR to remove the shortcut.

## Speech-tail streaks and speaker-change shadows - November 8, 2026

Branch `feat/notched-streaks-group-identity`, based on main 3fe1cb0 including
PR244-245. WhatsApp-style bubbles keep a triangular top sender-side tail on
every message. Eligible same-source/sender/minute text intersects by 6 points;
the older lower edge and shadow paint in front. Quotes, forwarded/edited rows,
status changes, pending/failed rows, reactions on the earlier row, pins/stars
and selection break overlap. The Messages skin keeps its own geometry.

Group speaker changes use a coloured body-and-tail drop shadow, with no
accent rail, seam outline or raised top-edge border. Names and avatars stay.
Hue is assigned to a complete loaded sender streak before viewport culling:
12 candidates at 30-degree steps, at least 120 degrees from the previous streak,
then closest to the sender-ID preference. Same-speaker rows keep the hue;
returning speakers can change hue, and loading earlier history can recolour
later streaks. Speaker transition contrast takes priority over identity colour.

Reactions and time/status flow in one wrapping below-bubble row, preserving
individual message targets. Repeated same-state ticks collapse within eligible
streaks. Linked timelines use the shared renderer and reserve painted tail
bounds outside the network-chip lane. Covered padding is not the next message's
input target; current-pass geometry prevents a stale scroll position from
creating a false overlap.

Verification: strict all-feature/all-target clippy, fmt and actual demo build
pass. Focused native harness, with unrelated test modules omitted and changed
production renderers unchanged: 36 tests pass, including 11 demo regressions
for 7 exact 6px intersections, both themes at 620/1180, tails, footer/reaction
bounds, body/tail shadow colour and no extra border shapes, colliding sender
hashes plus 1000 speaker changes, scroll/wrapped reactions, seam right-click,
selection, time/tick dialogs, failed-draft recovery and linked chip geometry.
The old reaction regression now checks noncollision instead of forcing two
rows, as required by the approved compact-footer design. Full native runtime
suite remains a CI gate, not a claimed local pass.

Actual synthetic demo captures inspected in both themes, including 3x seam
zoom after removing the last shadow-spill line. Existing narrow chat-header
crowding is outside this patch. No screenshot/media files committed. Review
media delivered separately; PR media-upload availability must be reported.
No merge, release or DMG dispatch. Revert this PR to roll back.

PR246 review repair: compute one adjacent-pair boundary table before rendering,
including pin-notice times and the following row's unread divider. Both
receipt collapsing and physical overlap consume that table. Runtime regression
uses outgoing18:40:01, pin18:40:10, outgoing18:40:15 and verifies both receipts
retain slots. The8pt triangular-tail inset is WhatsApp-only; Messages retains
its original17pt sender edge in both directions.38 focused native tests pass,
strict all-feature/all-target clippy passes. GitHub image upload retry: three
PR navigation attempts reset the browser connection before loading; neutral
page loads. No image URL produced. Maintainer can drag the synthetic PNGs into
the PR-description editor; no media committed. CI must run on the repair head.

Full runtime CI on544bc78 found11 failures (1905 passed,8 ignored), so that
head is not merge-ready. The compact-footer scope inherited vertical gaps and
horizontal minimum row height even without reactions. Set the scope's vertical
spacing to zero, retain the explicit3pt footer gap, and use a direct15pt
metadata slot when no reaction row exists. Use wrapping layout only for actual
reactions. Forwarded text keeps its original top padding. These production
changes preserve the older media, call, album-height, selection/right-click,
keyboard-link and initial-scroll contracts instead of changing their tests.
Expanded focused harness includes every failed test unchanged, its helpers
and the full bubble-detail test module:55 tests pass. Changed production code
is the PR's renderer. Strict all-feature/all-target clippy, fmt and demo build
pass. Fresh dark/wide and light/narrow synthetic captures inspected. Full CI
still needs to validate the next repair head; do not merge based on local tests.

Maintainer waived PR screenshot embedding ("Okay nvm screenshot"); stop upload
work, keep media out of the repository. New request: add headroom in overlapped
bubbles. Continuations now add the6pt overlap to their normal9pt top padding,
leaving9pt visible between the older edge and the next text. Runtime geometry
checks this in both themes and widths. Restrict enlarged padding to actual
streak members so ordinary text retains its original6/5pt vertical margins.

Full CI one9968fb:1913 passed,3 failed,8 ignored. Expanded the local harness
again with the full long-chat module and Ctrl+End/new-arrival regression.
ScrollArea applies offset after layout: when new row measurements had zero
net height correction, no discard occurred and the frame painted at the old
offset. Request a discard whenever the output offset changed, even with zero
height correction.60 focused tests pass, plus12 repeated keyboard phone-link
runs and4 Ctrl+End runs. The phone-link CI failure is intermittent and is not
claimed resolved until full CI verifies the new head. No test weakened.
Strict all-feature/all-target clippy, fmt and demo build pass. Fresh synthetic
dark/wide, light/narrow and3x headroom crop inspected. Separate stepped-card
width request is approved for its own PR after this CI/headroom work, not here.

Full CI on2c9a493 confirms both scroll regressions fixed. Remaining failures:
wrapped-phone reverse Tab and two tour sender-name clicks (1913 pass,3 fail).
Reproduced the sender helper selecting y502 while wheel easing moved the name
to421/371 during press/release; the helper now waits for three stable frames
before choosing the same click point. All original panel assertions remain.
Reproduced the exact phone failure using DejaVu-only fontconfig: one phone hit
region in the initial narrow frame became two after layout settled, invalidating
the saved last-region ID. Finish five narrow frames before recording IDs and
expand the unchanged focus/popup/copy/NewContact contract to360/400/520pt.
All78 focused native tests, including the full tour module, pass; the phone
contract also passes all three widths with DejaVu-only fonts. Strict clippy
and fmt pass. No additional production changes in this test-input repair.
Full new-head CI remains required. Actual timestamp/streak combination captures
(one/two/three minutes, both directions, reactions, sender change) inspected and
delivered privately; no behaviour changes or temporary fixtures retained.

## Stepped card widths - November 8, 2026

Instinct, branch `feat/stepped-card-widths`, based on main3fe1cb0. Separate
from PR246: common card content width320pt; wider body text rounds up to
400/480/560/...pt, capped by the available viewport. Plain text uses smaller80/160/240pt steps, joining the same320/400/... grid. Preserve location/image/carousel-specific sizing.
The offline `card-widths` page compares file, voice, reply, preview, two
wider cards with different body lengths, and ordinary text. Tests cover exact
bucket boundaries and actual bubble bounds in both themes at620/1180pt.
Focused native harness omits unrelated test modules, keeps production sources
unchanged:95 tests pass, including both new width tests. Strict all-feature/
all-target clippy and fmt pass. Actual demo rebuild and dark/light before/
after crops inspected; narrow-window viewport caps also visually inspected.
No review media committed. The maintainer explicitly waived PR248 screenshot
embedding; no more upload retries. Fresh captures delivered privately.
Review repair: the width fixture derives its body link from the inherited
preview URL, keeping both targets at spotifast.rocks. Regression verifies
matching targets.96 focused native tests and strict clippy/fmt pass.
Rebased onto main3cea59b after the maintainer merged246. Preserved both
PRs' checkpoint and demo sections; renderer/demo source merged cleanly.
Plain text now also uses80/160/240/320/400/...pt steps per maintainer approval.
97 focused tests pass before rebase, including text bucket boundaries and
actual text/card geometry in both themes. Fresh combined actual-app captures
inspected and privately delivered. Rebased verification:97 focused native tests and strict clippy/fmt pass.
No merge or release dispatch. Full new-head CI still required.
Rollback: revert this focused change. PR246 remains independent.

## Linked network chip lane - November 8, 2026

Instinct, branch `fix/linked-network-chip-overlap`, based on main 2498c7b
including merged PR241-243. The linked renderer reserved room for an outgoing
network chip, then passed the full timeline edge into the shared bubble
renderer. That edge widened the bubble back into the chip lane. Pass the
reserved area's right edge instead. Incoming layout and grouped footers stay
unchanged; no protocol, read-state, send or storage changes.

Add the `linked-chip-lanes` offline fixture and a runtime geometry regression
for outgoing/incoming marks at 620 and 1180 points in both themes. The focused
native harness (unrelated test modules omitted, production renderer unchanged)
passes; restoring the old edge fails with 12 points of outgoing overlap.
Actual demo binary captures at wide/dark and narrow/light were inspected:
marks stay outside bubbles, wrapped text and grouped footers remain readable.
Fixture review: rebase Signal timestamps after the final WhatsApp fixture row,
not launch time. Order is always outgoing WhatsApp first, incoming Signal next.
A separate fixed-clock regression compares all row timestamps at 00:00, 12:00,
18:00, 19:00 and 23:00 UTC; it and the geometry test pass in the focused native
harness. Updated wide/dark and narrow/light captures inspected after this fix.
Adversarial check: rebasing preserves Signal sender direction, exact chat IDs,
minute spacing and both chip directions; other demo fixtures are untouched.
CI on c4c3fc8 failed in mr-boxington setup after cache restore with repeated
HTTP504 responses, before either clippy or project tests ran. Local strict
clippy passes; the next head still needs real runtime CI.
Review media stays outside the repository. Full native runtime suite is a CI
gate, not a claimed local pass. No merge or new DMG; revert this PR to roll back.

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

## README refresh (2026-11-09)

Review, owner Instinct, branch `docs/readme-refresh-20261109`. No root AGENTS/BOARD/TODO file exists at the surveyed main. Rewrite README against the Beeper adapter, Mac launcher, current packaging and scheduler implementation. Separate current code from live/native acceptance; keep upstream credit. No new screenshots or illustrations. Source checked against current adapter, launcher and release scripts. All relative links pass; Markdown rendered in Chromium and actual top/install/build/privacy sections visually inspected. No screenshots added to the repository. CI and PR review pending.

README review follow-up: corrected linked macOS scheduler removal guide to actual zapfast paths, both installed binary names and control/API-token copies; preserve queue/logs by default and require service stop/reconciliation. Documented existing launcher binary/plist mismatch without changing runtime code or deploying a service.

## Adaptive sender cadence - November 9, 2026

In progress, owner Instinct; separate branch `feat/adaptive-streak-cadence`,
worktree `/downloads/solmu-adaptive-streak`, based on main f652135. User asks
for recent cadence rather than a fixed-minute stacking boundary. The synthetic
acceptance case is one sender at 07:54, 08:52, 08:54, 08:55: the last three
belong together, but the first does not. No private screenshot in the PR.

Implemented rough rule: compute a sender/source's completed
positive gaps in the preceding ten minutes, excluding the candidate gap; allow
1.5 times their arithmetic mean, clamped between two and five minutes. No prior
gaps uses the two-minute bootstrap. A gap over five minutes resets cadence.
Keep adjacency, exact source/direction/sender, content/metadata/control,
selection, pin and notice boundaries. Both ordinary and linked renderers must
use the same precomputed decision, not a viewport-limited lookback.

Two synthetic cadence regressions and the existing metadata boundary table are
updated. Native test checking and clippy pass. Native before/after pixels verified: the 18:40/18:41 fixture joins and hides
the intermediate clock. Added rendered-footer regression after visual checking
caught the clock seam. Draft PR256; full runtime CI is pending (cache HTTP504
blocked its quality job before compilation). Focused native runtime tests pass for burst grouping, source/bounds/zero
gaps and the rendered minute-clock seam, in a local build omitting unrelated
tests to fit memory. Removing the seam fix makes its rendered regression fail
(31.5pt intermediate clock), restoring it passes. Both ordinary and linked
native before/after pixels inspected. No full-suite pass claimed. Separate
from reply cancellation PR255. No merge, release or README changes.

CI executed 1,929 passing tests with one old RTL geometry fixture failing: its
one-minute gaps now intentionally collapse clocks. Spaced only that test's rows
six minutes apart, keeping all RTL, emoji and clock-position assertions intact;
its focused native execution and all-feature clippy now pass. Full CI rerun pending.


PR256 review follow-up: full applicable CI passed at c536c0e and the PR is ready,
not merged. Fixed the reviewer-found linked Messages-style clock regression:
that style does not overlap bubbles, so it must keep its independent clocks.
The new rendered-footer regression fails without the guard and passes with it
in the memory-bounded native validation build; all-feature clippy passes.
Published synthetic dark/light native before-and-after captures plus the
Messages-style guard capture at
https://files.instinct.com/xlkcf5cm0tyf-solmu-cadence-review
The public page and image viewers were visually inspected. Full CI and renewed
review for the follow-up commit remain pending. No release build or merge.

## Cancel linked replies - November 9, 2026

In progress, owner Instinct; branch `fix/cancel-reply-quote`, isolated worktree
`/downloads/solmu-cancel-reply`, based on main f652135. The reply X/Esc action
clears the app target but the linked-person view restores its own cached target
next frame. Clear both through the same cancellation action, preserve draft
text and explicit destination choices, and verify a later send has no quote.
Keep unrelated linked people untouched. Native `cargo check --locked --tests`
and formatting pass. Real composer regression and state-isolation tests are
written. Focused native runtime regression passes for X/Esc/redraw/unquoted
send and state isolation, using a local reduced-test build because full local
code generation was resource-killed. Removing the production fix makes the
new regression fail on quote restoration, then restoring it passes. Native
demo pixels confirm X/Esc remove the strip and preserve the draft. Full CI
is blocked before test execution by repeated cache HTTP504, not marked pass.
No README scope changes, merge, release or deployment.


PR256 integration follow-up: full applicable CI passed at 343f39a. Both posted
review findings were marked resolved. Main then advanced to 1021909 with the
reply-cancellation and DOS-device-name fixes; the resulting conflict was only
in the changelog and this planning log. Preserve both entries and all upstream
code with a branch-only merge from main. No feature behavior changed in this
resolution. Fresh integrated CI/review pending; no merge to main or release.


Hidden review finding follow-up: exact outgoing sender IDs are now also a hard
cadence boundary. The shared visual run helper intentionally treats all outgoing
messages as one side, but cadence history is keyed by exact sender; stacking
must use that same identity. Native regression reproduced the bug with distinct
outgoing IDs, then passed with the explicit sender check. Same-sender and incoming
boundaries are asserted too. Focused runtime uses the memory-bounded validation
build; full current-head CI/review remain pending. No merge or release.


## Linked-chat attachments - November 9, 2026

In progress, owner Instinct, branch `fix/linked-default-attachments`, based on
main 86ccc73. Olli reported the linked-chat clipboard refusal. Allow clipboard,
drop and file/document-picker staging in the linked composer. The visible chat
owns the draft, but sending resolves the exact selected/default source using
the existing network/account picker. Missing/stale/locked/ambiguous sources
refuse without fallback. A queued send whose route changed keeps its draft.
Keep destination-chat drafts untouched, and keep account/owner guards. Returned
attachments cannot silently retry on a changed source; choosing a destination
explicitly allows moving an unquoted returned draft. Quoted returns stay tied to
their source until Cancel reply. Both kinds restore to the reachable person draft,
including delayed Beeper upload failures after navigating away. Expanded below to include
voice, GIFs and stickers under the owner's correction. No real attachments sent.

Version 0.24.8, What's New/changelog updated. Native all-feature clippy passes;
five focused native regressions pass in a reduced-test validation build, including refusal recovery and changed-source retry protection.
Actual synthetic native light/dark before/after and source-picker pixels
inspected. Final all-target/all-feature clippy passes. Full CI pending. Local RED attempt did not finish due to disk/codegen limits, so no failing-before claim. No main merge or release.
The Release workflow rejects branch-only commits before building a DMG; draft
arm64 DMG remains held under the standing build rules, not worked around.

Synthetic before/after and picker evidence: https://files.instinct.com/nioka8nka98f-solmu-linked-attachments (public, no real conversations). Page inspected at 320/390/1280px and live preview photo viewer checked.


PR257 review follow-up: fixed four findings. BeeperFilesUncertain and quoted
attachment refusals restore files plus captions under the same canonical member
key as open_chat, not an inaccessible nonrepresentative draft. Per-attachment
source/quote identities survive parking and account switches; conflicting quotes
refuse until Cancel reply. Send Later refuses staged media at both dialog and
execution boundaries, retaining caption and files. Route errors use direct
gettext literals. Eight targeted native tests plus an existing linked demo test
pass in the reduced validation build; all-target/all-feature clippy and fmt pass.
RED proof: restored the published 8570625 recovery and scheduling functions in
the validation checkout with the new tests unchanged; all three new recovery/
scheduling regressions fail. Restored fixed production code, all nine pass.

Dependency advisory CI fails on unchanged libsqlite3-sys0.38.2, RUSTSEC-2026-0459.
A separate dependency PR is assigned; do not fold it into attachment scope.


PR257 media scope correction (2026-11-09): voice, GIFs and stickers are included.
Linked voice sends resolve the exact default/chosen source; returned recordings
keep their source identity, samples and reply until an explicit safe retry.
The destination picker remains available on the recording bar. GIFs and stickers
join the owning staged draft and use native API `gif`/`sticker` attachment hints,
not document disguises. A bounded separate worker keeps typed upload failures
with their original source, reply, media type and caption. Accepted IDs enter the
existing pending-message reconciliation path. GIF search uses the user's GIPHY
key and bounded HTTPS media retrieval without Beeper credentials or redirects.
Linked sticker creation prepares cropped WebP locally before staging. Picker
favorites include locally saved/cached WebP; remote sticker-pack import remains
its existing separate workflow. No real message/media calls used for tests.

Local validation: all-target/all-feature clippy with warnings denied passes.
21 focused reduced-build regressions pass, including native HTTP fixture payloads,
source routing, asynchronous ownership, retained media type/caption, voice route
refusal, URL boundaries and sticker preparation. Actual native dark/light GIF/sticker staging, voice recording, voice source-picker
and restored media tabs were inspected. The updated public evidence page was
checked at 320/390/1280px, with all images loaded and its viewer opened/closed.
A wider 475-test reduced-harness run hit the execution timeout before completion;
no full-suite pass is claimed. Current-head CI follows the commit. No merge;
DMG remains held.


## PR #257 follow-up: portable staging and media ownership

The portable adapter now stages the production linked_media module. Its 239 Rust tests and all 102 Python tool/packaging tests pass locally. The current-source reduced native build passes 27 focused linked-media regressions, including draft removal, parked-draft retention, ownership transfer to upload, orphan preparation, stale thumbnails, and unrelated failed-voice scheduling. Full all-target/all-feature clippy with warnings denied and fmt pass. No full native-suite pass is claimed locally.

Prepared GIF/sticker paths are owned by staged attachment IDs, transferred to the upload worker before send, and reclaimed only when a draft drops them. User-owned files are never removed. Attachment kinds are pruned across all account drafts. Search thumbnails are reclaimed on replacement, stale results, account change, and shutdown. Search uses one worker with a replaceable pending query and a 300ms quiet period, limiting network fan-out while typing. Send Later uses the same current-owner media predicate in the toolbar, dialog, and deferred action.

No real uploads or sends, no merge, and no release build. SQLite draft #258 remains intentionally blocked on the vulnerable bundled SQLCipher dependency.


### Lifecycle review follow-up

The next automatic review confirmed the previous three findings resolved and identified account-removal and shutdown edges. Cleanup now runs after an account and its parked drafts are removed. Backend shutdown joins GIF search, closes its old event receiver, and reclaims queued media; other late producers reclaim rejected media events. A single deletion worker performs filesystem cleanup away from the UI thread, with a shutdown barrier. Added hidden-account removal and unpolled/rejected media regressions. The unchanged-production reduced native build passes 29 focused tests, the portable adapter passes 239 tests, and full all-target/all-feature clippy plus fmt pass locally.


### Native CI shutdown-test contract

Native CI at b041035 passed 1959 tests and failed one old combined draft test because it injected a logout event after `App::shutdown`. Shutdown now deliberately closes event delivery to reclaim unpolled media. Split that test into shutdown saving/rejection and live-account logout clearing. Both exact production tests pass in the reduced build, alongside all 29 linked regressions; full all-target/all-feature clippy and fmt pass. No production shutdown cleanup was weakened.

### Link suggestions, first slice

Added `src/suggest.rs` (pure matcher with tests), `Chat.handles` filled from the other participant's phone number and username, per-pair dismissals in `beeper.dismissed_links`, a Contact info card, a collapsed Filters row and an Undo toast (`ToastKind::Undo`). The portable adapter harness passes with suggest.rs staged; the UI is first compiled in CI.

## Export alternate-stream names - November 10, 2026

PR #272 follow-up: normalize colon to underscore before reserved-DOS-stem replacement so `CON.pdf:payload` becomes `attachment.pdf_payload` and ordinary filenames cannot select NTFS streams either. Four new regression cases; the old code failed on the first case, 242 other portable tests passed. Linux portable adapter tests rerun after repair; real Windows/NTFS not exercised. No merge or release build.



## Linked-chat avatar source - November 11, 2026

Owner Instinct, branch `fix/linked-avatar-default`, isolated from #267/#269.
Use the saved primary picture, then saved member order. Resolve exact identities
in the active app account and reject locked, missing, replaced, duplicate-ID,
group and foreign-origin sources. Thumbnail and Contact info full-size lookups
share that order. Never write fallback paths over member caches or change send
routing. Re-evaluate each lookup so late pictures/default changes take effect.

Portable current-source regressions and Python checks run locally; native App
regressions added for fallback, late primary arrival, changed default, source
cache preservation, full-size/thumbnail agreement and request order. Native GUI
and real Mac/connected-network avatar behavior remain unverified locally.
Custom contact pictures are a separate requested feature awaiting UI approval.
No merge, main push or release build.
## Final-ID send confirmation - November 11, 2026

Owner Instinct, branch `fix/final-id-send-confirmation`, based on main c1ff292.
Read-only investigation of the 60-second toast found that pending-message GET
may return the final ID per Beeper's current retrieve/send API docs, but the
poller required returned ID == pending ID. Accept that resolution while keeping
exact chat, outgoing-sender and explicit SUCCESS/delivery evidence. Timeout copy
now says not confirmed, since missing/unknown status is not necessarily PENDING.
No retry, resend, send payload or routing changes. Text/media share this poller.

Regression first failed against unchanged production code for final-ID SUCCESS;
then passed with the repair. Fixture also covers foreign chat, incoming SUCCESS,
pending and absent status. The current-source portable suite runs locally. No
live message was sent or private message data retrieved. This proves a possible
false warning, not which bridge/status caused the owner's particular toast.
Source behavior predates Nov 9; #274 changes inbound download queuing, not send
confirmation. Real network send confirmations remain unverified. No merge/build.

API references:
https://developers.beeper.com/desktop-api-reference/resources/messages/methods/retrieve
https://developers.beeper.com/desktop-api-reference/resources/messages/methods/send

Review follow-up: route the new static confirmation-timeout key through gettext
at the Event::Info UI boundary. Other dynamic Info messages are unchanged. No
translation catalogs regenerated; the maintainer owns that step.

## Media download queue backpressure - November 11, 2026

Owner Instinct, branch `fix/media-queue-busy`, separate from avatar work.
Applied the supplied media-backlog patch to main c1ff292. Requests rejected by
the eight-slot worker queue wait in a bounded 256-entry backlog and retry on
control-loop ticks; explicit clicks go before automatic work. Review repair:
a click upgrades a queued automatic duplicate (including its manual-download
permission), clicks keep FIFO order, and new automatic overflow cannot evict
older accepted work. A full backlog may still reject work explicitly; a dead
worker reports failures. No unbounded queue or automatic resend is introduced.
Portable production-source regressions run locally; real Telegram/WhatsApp
media downloads and native GUI remain unverified. No merge or release build.

Review scope clarification: the current UI disables controls for items already
marked Downloading, including waiting automatic requests. They cannot be clicked
to promote them. Priority applies to new explicit requests reaching the backend;
manual upgrades are a backend safeguard, not a new user-visible promotion flow.
The PR description/changelog now make this limitation explicit. A waiting versus
active UI model is outside this queue-full repair.


## Reply/forward shortcut gap - November 11, 2026

Owner Instinct, branch `fix/reply-forward-shortcuts`, based on main after #274.
Ordinary chats had no Cmd-R handler: only merged::show consumed that key. Add
exact-modifier Cmd-R handling for ordinary chats via existing Action::Reply,
which preserves an unedited draft and focuses the composer. Latest incoming
selection ignores later outgoing/revoked rows, independently of unread divider.
Linked Cmd-R remains in merged::show and retains exact-source routing.

Cmd-L was free (Cmd-Option-Shift-L is app lock). It now opens the existing
Forward dialog for the selection or latest forwardable loaded message, including
an exact source chat in linked views. No send is performed by the shortcut.
Existing Beeper Command::Forward remains unsupported: this is dialog wiring,
not the separately proposed composed-forward implementation. Overlay guards and
exact modifiers preserve other shortcuts. Added native key-dispatch regressions
and updated the shortcut registry/help. Fmt and diff checks pass locally; native
execution/real Mac UI remain unverified pending CI. No merge or build.

PR #276 follow-up review: exclude synthetic call records from ordinary replies,
cancel selection before Reply so its composer is visible, and share the exact
linked-view ownership predicate with merged::show (duplicate IDs/group-kind
fallbacks keep ordinary Cmd-R). Regressions include those edge cases. Local
native test compilation was attempted but exceeded the available command window
while building dependencies; no passing native result claimed.
Own adversarial pass also keeps the new message shortcuts out of the image
viewer and uses the linked view's exact valid-members filter for Forward,
including direct-chat, unlocked, same-origin and unique-ID constraints.
CI caught one needless-unwrap Clippy lint and a regression fixture borrow
conflict. Use if-let dispatch and construct the member before mutating settings.
No production behavior change in this follow-up.

## 2026-11-11 - Beeper chat media album (Instinct, draft)

User request: browse chat-wide media with Left/Right. Existing key handlers and
ViewerStep were already in the delivered manual.29 build, but Beeper silently
ignored LoadChatMedia. Added a dedicated bounded history worker, exact-chat
validation, fresh/continuation requests, and older-edge continuation. Plain arrows
now browse photos and playable videos; Alt+arrows retain video seeking. Does not
create an audio/document/GIF viewer or combine linked sources into one album.
Portable adapter tests cover paging/cursor/source boundaries. Full native tests
and macOS interaction remain unverified; do not describe this as a shipped fix.

## 2026-11-11 - Favorites list sections, option B (Instinct, draft)

Approved sketch B: small Pinned/Other chats labels, with a subtle rule only at
the group boundary. Headers appear only when both favorite and ordinary chats
remain visible; search/empty/Favorites-only/all-unpinned views remain ungrouped.
Variable-height virtual rows include headers in keyboard reveal geometry.
Native tests and actual pixels still require verification. No release build.

