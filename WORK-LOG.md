# Work log

## 2026-11-20: Thread navigation and visible right-click target

Latest Mac CI passed 2461 tests, failed ordinary OpenMessage thread-exit and the
existing right-click-beside-document fixture. The normal OpenMessage action now
leaves the thread; the earlier insertion only covered its locked-app deferral.
The right-click fixture now scrolls to its tested row before reading the target
rect, rather than assuming both old rows remain visible after loaded-reply
controls add transcript height. No production context-menu behavior was relaxed.
This fixture explanation still needs the new CI run; it is not labeled flaky.
Local full-test compilation again exceeded the execution window, so these tests
are not claimed passed locally. The previous album/texture failures are absent
from the latest Mac result. No screenshot or release change.

## 2026-11-20: Combine approved one-tail groups with restored overlap

This branch combines one tail per grouped bubbles with restored overlap.
Integrated the four overlap-restoration commits from PR407 on the avatar-tail
branch, retaining current main and its newer hidden-message handling. The tail
rule now applies to ordinary outgoing/direct runs and linked runs too. Incoming
Messages avatars remain bottom-aligned with their existing tail geometry; the
WhatsApp-style head has the approved one-third mouth-height rule.

Own review of407's failed CI found global bottom-padding inflation changing
isolated voice/image geometry and a seam command-click without a dedicated
owner. Non-text padding stays unchanged; replay is clipped to each older row's
actual safe bottom inset. Added stable older-seam primary gesture ownership
(command-click selection and double-click reply); newer body targets remain
clipped below the covered seam. Updated the old every-bubble-tail assertion to
the requested one-tail-per-run behavior. These repaired interaction regressions
still require full CI execution; they are not claimed passed locally.

Native all-target/all-feature Clippy and demo build pass. Inspected actual dark
wide, light narrow, group-stacks WhatsApp and Messages captures. Overlapping
steps are visible; text is readable; followers have no separate tail. Existing
geometry unit helper ran1pass earlier. New screenshots remain private; no
waiver assumed, PR not opened.407 is not closed until the combined PR supersedes
it with validated CI and review. No release.

## 2026-11-20: Avatar-aligned incoming tail (feat/avatar-mouth-tail)

Build option1 from the approved synthetic sketch: retain the existing downward
nub, with tip y = avatar bottom minus avatar height/3. Incoming ordinary group
heads use the actual top-aligned avatar allocation; same-sender followers have no
tail. Outgoing and direct-chat tails are unchanged. Messages skin retains its
existing head-tail geometry because it bottom-aligns avatars; moving this
same downward nub there would extend below short bubbles. Linked-view avatars
are not changed by this ordinary-conversation patch.

Added a geometry unit test with translated/resized avatar rectangles. Executed
the exact helper and exact unit test against the built egui dependency:1 pass.
Full native test compilation exceeded the command window; no full-suite pass
claimed. Native demo build and all-target/all-feature Clippy pass. Added an
avatar-tail synthetic demo page with short and tall heads plus two followers.
Inspected actual dark wide and light narrow captures: heads meet the avatar at
the requested height; followers remain tailless. No external image hosting or
screenshot waiver assumed. Parent has captures and the waiver/audience question.
## Quiet hidden placeholder,20 November

Implement approved artboard4: no permanent visible label, compact32px seam/ellipsis target with Show message on hover and keyboard focus, click restores. Accessible button name remains explicit; custom Tab/reveal logic retained. Added no-label layout regression; existing keyboard restore/reveal regressions retained. Full native test execution and both-theme pixels await CI. No network delete or hide semantics changed.

Keyboard review follow-up: focused restoration text now replaces the ellipsis inside the seam, never outside the row where outgoing alignment could clip it. Narrow incoming/outgoing focus-label coverage added. Native test-target compilation passes. Actual Linux keyboard navigation exercised from the composer through Tab stops; Show message is visible within the focused seam in dark and light captures. Mac execution remains unverified.

Regression follow-up: inspect both accessibility label and value, with a negative control that recreates a separately painted static-text announcement and proves detection.

Reviewer follow-up: the seam paints its visual hint without a second accessibility label. The focused regression rejects elided galleys, checks painted glyphs, and asserts one accessible restore name. Formatting passes; local Clippy exceeded the bounded run without diagnostics, so execution and compile validation await fresh CI.

## 2026-11-20: One receipt per automation request, bounded receipt history

The follow-on review found HTTP and the shared write handler both recording a
receipt, and denied traffic exhausting the receipt cap. HTTP now calls an
internal already-recorded handler; direct callers keep the audited entry point.
The metadata receipt history retains the latest 10,000 receipts plus a saturating
lifetime count. It does not evict durable idempotency claims, whose separate
10,000-entry cap still fails closed. This prevents unauthenticated receipt traffic
from consuming claim capacity. Rotation is explicit here, not an automatic purge
of send outcomes. Existing old journals load with a default receipt counter.

Executed 114 production-module acceptance tests, including a full receipt window,
an unauthorized attempt, an authorized send, and an idempotent retry: exactly
one receipt per request, one retained claim and one synthetic upstream send.
Native all-target/all-feature Clippy passed. No real credentials or messaging.

## 2026-11-20: Maximum audience counted-send regression

Added a full-view regression that selects 50 synthetic distinct recipients,
checks no queue exists from selection alone, clicks Send to50chats, then checks
all50 exact destinations are queued in order. This preserves the existing
explicit counted one-click send design, not an extra confirmation screen.
All-target/all-feature Clippy passes; test execution awaits native CI, no pass
claimed locally. No live account or send used.

## Distinct editing target,20 November

Implement approved artboard3 on top of shortcut-target fix #408. Edited message gets a3px contrast-adjusted outline and reserved EDITING header line (does not paint over a neighbor). Composer shows original preview, named Cancel and Save edit controls. Add visible-tag regression and both-theme edit-state Mac captures to CI. cargo check --lib --tests --features demo -j1 passes; test execution and app-pixel inspection still pending. No send behaviour or network action performed.

## 2026-11-19: Beeper bridge messages show every attachment (fix/beeper-multi-attachment-messages)

Olli reported bridge messages with several attachments showing only the
first. The Beeper API delivers a multi-attachment send as ONE wire message
with an attachments array, and attachment_content() mapped only
attachments.first(). map_messages() now splits such an event into one row
per usable attachment sharing the bridge id as their album, so the existing
album row draws them together; the first part keeps the bridge id, caption,
quote, reactions, mentions and edit mark, later parts get "{wire}#att<n>"
ids (model::attachment_part / attachment_part_id) that resolve back to the
bridge message and their own attachment index. Boundaries resolve the part
id: get_message (edit, react, delete identity checks, forward planning,
quote resolution), the reaction/edit/delete URLs and emitted MessageDeleted
(delete of any part removes the whole bridge message; app-side the delete
event removes every part and moves the cursor past the album), the media
worker (a part downloads its own attachment via download_media_at's
existing index parameter), and replyToMessageID on send. Poll notifications
fire once per bridge message, not per part. Viewer gallery and the
shared-items panel list every part. No local compile on this box (no GUI
toolchain); fmt/clippy/tests run in CI, plus an evidence workflow with
failing-first test proof and before/after demo captures (bridge-album page
through the real mapper, synthetic content only). No merge by the author.

Devin review round one: forwarding a chosen part now validates against the
bridge id and sends only that attachment instead of failing the identity
check or expanding to the whole set; a jump to an unloaded later part
compares bridge ids so it stops on the right page; selecting several parts
of one bridge message for delete coalesces to success per part when the
first delete's tombstone answers the rest; poll notifications pick the
first displayed row per bridge message (the first usable attachment is not
always attachment 0) and quote jumps land on it through
Conversation::message_or_first_part; the viewer gallery orders a message's
parts by attachment index instead of id text ("#att10" after "#att2").
Regression tests cover each, including a mock-server tombstone delete and a
mock-server forward of "album-1#att2" through planning and submission.

## 2026-11-19: Avatar crop editor with macOS face centring (feat/avatar-crop-reframe)

Branch feat/avatar-crop-reframe off main 7d4b87a, version 26.11.2. The
customize-avatar modal is now Photo | Initials tabs (sketch D, approved
2026-11-19): the Photo tab has a 240pt crop canvas (drag to move, scroll or
slider to zoom, circular outline), an "Auto: face" chip and "Re-centre on
face" button when detection found a face, live previews at 48/32/24 pt, and
Reset left / Cancel+Save right in the footer. Appearance::Image gains
crop {x, y, zoom} and source_aspect, both serde-defaulted so entries saved
before cropping keep their exact old look (old files are the pre-cropped
512x512 squares; default crop renders them identically). The picker now
stores the bounded full source (max side 1024) instead of a centre-cropped
square; the square window is a texture UV rect at paint time, so the #377
thin-image blowup stays impossible (no upscale-to-square anywhere; tests
cover 8192x1 both ways). Face detection (src/face_detect.rs) is macOS-only
via native Vision (VNDetectFaceRectanglesRequest through the objc2 runtime,
no new crates, Vision framework linked); every other platform returns None
and keeps the centre crop. Detection runs in the picker thread. Not compiled
locally (no full Rust toolchain deps on this box: PipeWire); the pure crop
math and serde tests were verified in a scratch crate against the real
image/serde versions, fmt via rustfmt; clippy and the full suite run in CI.
macOS detection itself is unverified (Linux box): the macOS CI leg compiles
it, runtime behaviour needs a Mac. No merge by the author.
## Sidebar person names,20 November

Implement approved sidebar sketch: remove linked suffix from chat title and network-count subtitle. Existing source-network avatar badges and combined timeline remain unchanged. Add title/subtitle regression. No new replacement icon. Native execution/pixel review pending.

Pixel review caught a residual network-count suffix after removing linked. Removed that suffix from the merged-person preview; source badges stay intact. Both-theme synthetic Linux captures inspected with the full person name visible. Native Mac execution and review evidence publication remain separate gates.

## 2026-11-19: Captioned media bubbles settle to the picture (fix/captioned-media-bubble-width)

Olli reported sent bubbles going "all funny" with a narrow media attached:
the bubble settled to max(picture frame, caption measure at the cap) while
the caption drew wrapped to the drawn picture width, so a long caption
inflated the bubble past the picture (dead strip beside it) and a slim
portrait shot squeezed its caption into a one-word column. Fix in
src/ui/conversation.rs: natural_content_width settles a captioned picture
or video to max(frame, min(caption measure, max(frame, CAPTION_MIN_WRAP)))
with CAPTION_MIN_WRAP = 200pt, and the new caption_wrap helper gives the
drawn caption the same floor (quoted messages still span the settled
width). Also asked: whether a neighbour message resizes an earlier bubble.
From the code: only plain-text own bubbles share a run's width
(planned_widths shares_width, #282 tolerances); a media bubble repaints at
a new width only when its own inputs change, and WidthBook::plan plays the
retarget animation only when a row is appended, so a late width correction
lands on the next message's arrival and looks caused by it. Failing-first
tests (commit before the fix): caption measure vs picture width, portrait
floor, caption_wrap bounds. Not run locally: no Rust toolchain on this box
(PipeWire headers too old); fmt, clippy and tests run in CI.

## 2026-11-19: Code panels mark content rows only (fix/code-panel-row-kinds)

#382 merged with five failing monospace tests (its CI was still queued when it
landed). row_kinds hit-tested glyph clusters against per-span content ranges
correctly, but then marked EVERY glyph-less row with the previous row's kind.
Parse drops fence marker lines, so each marker leaves a blank row; those blank
rows inherited Code, stretching panels past their blocks and merging adjacent
fences' marks (a_blank_line_inside_a_fence_stays_code,
adjacent_fences_stay_two_code_runs, multiline_fences_keep_one_block_index_each).
Now blank rows are placed by the character position of their newline against
per-block content bounds (end inclusive, since an empty content line is its
span's empty range): in-fence blanks stay code, dropped-marker rows stay
plain. Two test expectations were wrong, not the code: the bridge escapes only
an exact fence line, so a native backslashed line keeps its bytes in the copy
text; and the width test now tolerates the pixel or two the galley's
pixel-snapped glyphs add over the planner's summed advances while still
rejecting an unwrapped line. Verified by rustc parse-check only; fmt/clippy/
tests run in CI (this Ubuntu box cannot build the crate: PipeWire headers too
old).

## Hide unsupported message pin controls,20 November

Hide message-pin menu, chord/help, and production banner while Beeper adapter has no pin read/write support. Favorites unchanged. Pins browse sketch parked, not cancelled: pins still meaningful for groups, owner uses favorites instead. No Matrix state route exists in Solmu; SDK endpoint list/schema inspected with no message-pin route found. Not a live Server capability probe. Revisit only with a verified source and owner instruction. Regression covers Cmd-Alt-P dispatch being absent; native execution/pixels pending CI.

## 2026-11-18: Repair #379 fallout that left main red

Branch fix/context-menu-shortcut-fallout off main 2192fd3. #379 (context-menu
shortcut hints, head d54bb0a) merged red: quality clippy failed on
route_native_actions (dead outside macOS builds), and four tests failed
(chat-menu width floor, key help-rows literal, keyless-menu catalog, and the
native-command demo regression). Repairs: route_native_actions is now cfg-gated
to macOS or test+demo builds, the chat context menu keeps the compact 190pt
width floor in both places #379 had widened it (hinted labels already
truncate), the key-catalog tests learn that context menus own the
message-star/pin/save chords and the three new help rows, and the demo
regression holds the chat menu open through the native-action frame the way
egui memory does in production. Not run locally: no Rust toolchain on this
box (libspa-sys needs libpipewire); fmt, clippy and tests run in CI.
## 2026-11-18: Sidebar falls back to the full-size avatar (feat/avatar-full-fallback)

Cause: App::avatar (src/app.rs:4508) served non-linked chats from the
thumbnail cache only, while Contact info (src/ui/info.rs:300) already falls
back to the full-size picture. A failed thumbnail fetch caches its miss
(Event::Avatar full:false path None, src/app.rs:5711) and only the Beeper
account path rechecks after 15 minutes (src/account.rs:397-408), so the
sidebar could keep showing bare initials for a contact whose full-size
picture had in fact arrived. Fix: avatar() now falls back to the
cached full-size picture, the way the info panel shows it; it never starts a
new fetch, so the avatar worker's small queue sees no extra pressure.
Failing-first test covers miss-then-fallback and a real thumbnail winning.
Verified by rustc parse-check only; fmt/clippy/tests run in CI (this Ubuntu
box cannot build the crate: PipeWire headers too old).
## Emoji avatar initials,20 November

Retain existing three-symbol limit, count extended grapheme clusters rather than scalar chars. Accept emoji (family/ZWJ, skin tone, flags, keycaps) and letters with combining marks. Remove the TextEdit scalar12 cap that could cut a third long cluster before normalization. Shared color-emoji line renderer explicitly paints the avatar, with width fit unchanged. Tests cover clusters, limit, punctuation/control filtering and JSON persistence; native pixel verification pending.

## 2026-11-18: Code and quote panels with Copy code (feat/code-quote-panels)

Styling PR, second half (markup groundwork was commit 0c1d1b9): markup.rs now
also paints the panels (paint_row_panels: low-alpha fill across the body
width, pale accent bar on the leading edge, code panels returned in source
order) and keeps a blank row inside a fence marked Code so the panel stays
one piece. conversation.rs rich_body_full paints the panels under the text
and puts a frameless Copy code button at each code panel's top right; it
copies markup::code_blocks(text)[k] exactly (fences, prose, timestamps and
link annotations excluded, collapsed lines included) and swaps to Copied for
about a second via temp data + request_repaint_after. Selection drags are
untouched. The long-code-line overflow is covered failing-first by
fenced_code_rows_fit_the_layout_width (wrap.rs already force-breaks overlong
tokens); verified only by rustc parse-check locally, since this Ubuntu box
cannot build the crate (PipeWire headers too old) - fmt/clippy/tests and any
visual check run in CI.
## 2026-11-21: Linked sidebar title hover follow-up (#411)

Shared-boundary follow-up: reserve a one-point neutral gap between title and preview hover targets. The native handoff regression requires a strict positive gap; negative controls reject touching, overlapping and reversed regions. Click/drag still belongs to the full row.

Hover arbitration follow-up: title tooltip uses its own non-overlapping top-line hover target, leaving the full row click/drag target unchanged. Extended preview regression opens a networkless name tooltip before moving directly into its message preview.

Reviewer follow-up: retain the full-name hover tooltip without a network subtitle. Focused helper regression covers name-only and unchanged network cases. Formatting passes; runtime validation awaits CI.

## 2026-11-18: Failed link previews stay quiet (fix/preview-fail-quiet)

local_link_previews (src/ui/conversation.rs) painted a prominent "Preview
unavailable" + "Retry preview" row when a fetch failed. Now a Failed status
is skipped before any card space is reserved: no row, no button, no 5pt gap.
The match arm stays as a documented no-op. preview_worker.rs gains a
test/demo seed_failure next to seed_fixture. Failing-first test
a_failed_local_preview_paints_no_row drives local_link_previews with a
seeded Failed worker through the shared still-view fixture (generalized to
with_still_view_and) and asserts drawn == false and no actions. Verified by
rustc parse-check only; fmt/clippy/tests run in CI (this Ubuntu box cannot
build the crate: PipeWire headers too old).



## Larger reaction emoji,20 November

Initial13px label inherited from initial commit0cd79a8, no documented size constraint found. Use17px emoji, keep count13px, same chip padding/click semantics. Measurement and painting share the exact mixed-size line. Footer metadata receives26px reacted row height so it does not constrain larger pills. Added font regression; native pixels/execution pending.

## 2026-11-18: Final helper pins from the published whisper-helper-v1.9.5 release

Olli approved publication 14:49. #375 merged (matrix now uploads unsigned
Linux/Windows archives after installer verification). Release
whisper-helper-v1.9.5 published with six assets + metadata JSONs, marked
not-latest: macOS from signed run 44301781725, Linux/Windows from matrix run
44320859629. Every asset anonymously re-downloaded; size and SHA-256 match
the in-job metadata exactly, executable hashes re-verified. artifact() now
carries the real release URLs and recomputed pins; Linux executables are
byte-reproducible across runs, Windows ones are not (pins taken from the
published bytes). Docs updated. Not run locally: no Rust toolchain here.

## 2026-11-18: Pin the six prebuilt Whisper runtimes (publication pending)

Branch feat/local-whisper-pins off main 40ea68e (after #359 and #370 merged).
Wired whisper_runtime::artifact() for all six shipped targets from the
packaging handoff: macOS arm64/Intel from the signed + notarized main-branch
run 44301781725, Linux/Windows from the unsigned matrix build. Verified the
macOS pins independently against the run's metadata JSON and by hashing the
downloaded archives and their whisper-cli executables; the Linux/Windows
values could not be re-verified yet because the main-branch matrix run
44301777978 was still queued and the PR-branch run's artifacts are gone -
they must be confirmed against that run or the published assets. Download
URLs are placeholders (https://invalid.invalid/pending-helper-publication/...)
by design: the helper release is not published, publication needs the owner's
explicit approval, and every size/hash must be recomputed from the published
assets before merge. Docs and changelog updated to say so. Not run locally:
no Rust toolchain on this box; fmt/check/clippy/tests run in CI.

## Restore saved voice and video transcripts,20 November

The private on-disk store already persisted transcripts. Bubble status only read hot memory, so after restart saved text could be searchable while the same bubble offered Transcribe again. Restore exact account/chat/message text asynchronously with bounded concurrent reads, versioned hit/miss cache, and deletion/drop checks. Use the archive reader for bubble text even with the speech engine off. Save/drop/reopen voice/video display and search regression failed against the old getter and passes after the fix. All focused worker tests pass in an isolated production-module harness; native test-target compilation passes. Full app/Mac runtime unverified. No new GUI or release.

## 2026-11-18: Recent grouped sections: activity order, unread filter, dim-on-read

Branch feat/recent-unread-sections. Read README/WORK-LOG; no root agent/board
file present. Approved design (user picked option 4A of the sketches, with the
4C read transition as the default): in the Recent view's grouped list, sections
sort by their freshest chat (stable; Other pinned last), a section with unread
rows lists only those rows plus a "Show all N in {group}" row that opens the
full member list (and folds back), quiet sections list every member, and a
chat opened from a filtered section stays in the filtered list - dimmed via
unread_kept - until the view changes. Unread and Needs me modes keep the saved
drag order and full member lists; the flat Recent list is unchanged. Drag
reorder of headings is disabled in Recent (order is computed); member drops,
folding and keyboard navigation follow the drawn plan, and revealing a chat
lifts a section's filter when the chat sits outside the filtered rows.
controls() untouched (owned by another branch). Tests: section activity order
with Other last, saved order in other modes, unread filter plus expander
toggle, dim-on-read until SetListMode, flat list keeps nothing, keyboard walks
shown rows only. Verified locally: cargo fmt only - this Linux box lacks the
libxkbcommon/alsa/pipewire dev packages, so cargo check/test/clippy run in CI.
No DMG, no release.



## 2026-11-17: Wyoming transcript read as empty (data_length framing)

Branch fix/wyoming-data-length. Read README/WORK-LOG; no root agent/board file
present. Root cause of "wyoming returned an empty transcript" against a working
Parakeet-MLX Wyoming server: the reference rhasspy/wyoming library writes event
data out-of-line (header line carries `data_length`, raw JSON data bytes follow
the header with no newline of their own); the reader only handled inline `data`
and binary `payload_length`, so `data.text` was always empty. Fix consumes and
merges `data_length` bytes for every event before matching, mirroring
async_read_event. Tests: exact-framing repro (fails on main with the observed
error string), framed non-transcript events skipped, framed empty text still an
error; existing inline-data tests kept. Verified in a standalone crate against
the real module sources (this Linux box cannot link the full egui/pipewire lib);
full suite runs in CI. No DMG, no release.


## 2026-11-11: scoped archive/delete batches

Separate feature branch from main. Read README/CONTRIBUTING; no root agent/board/
roadmap/todo/bugs file was present. See docs/archive-delete-batches.md.

Added live capability mapping; all-item UI preflight; explicit scope and final
confirmation; per-item request-correlated results; no automatic retry or silent
local fallback. Removed optimistic Beeper message revocation and archive hiding.
A local Silence promise is recorded only after archive confirmation, not on a
refused/uncertain request. Existing non-Beeper handling remains separate.

Checks passed: cargo check --locked --lib, library clippy -D warnings, formatting,
isolated unchanged capability source test. Full tests, actual UI screenshots,
real bridges and macOS are unverified. New/updated tests need native/adapter CI.
No merge, DMG or live chat mutation. beeper.rs edits limited to capabilities,
map_chat, archive/delete handlers and their fixtures/tests. Pending-media owns
uploads/sends; rebase after it lands. Draft until all validation gates pass.

## 2026-11-11: local flat groups

Branch feat/local-flat-groups, stacked on keymap #281. Implemented one-level
local membership, additive drag with Undo, assignment Apply/Cancel, original-chat
aliases and deduplicated unread totals. Group deletion cannot issue a backend
command. Read README and CONTRIBUTING; no AGENTS/BOARD/CODEX/roadmap/todo/bugs file
exists at the checkout root. See docs/flat-groups.md for behavior and limits.

Passed: membership tests in a standalone crate using the unchanged model source,
cargo check --locked --lib, cargo clippy --locked --lib -- -D warnings, formatting.
Native check used local current PipeWire headers because host headers are old;
no repository dependency or lockfile change. Full tests, Mac runtime and pixels
remain unverified. No merge or release build. Shared-keymap const compile repair
reported to its owner and repeated in this stack.

## 2026-11-11 - Beeper view-once composer visibility

Approved scope: hide the misleading photo View once toggle on Beeper chats. No send/upload changes and no merge order.

- Composer visibility now checks Beeper origin/account identity before showing the existing photo-only control, including restored selected drafts.
- No draft state is cleared or downgraded. The existing backend refusal is unchanged.
- Red-green: compiled the unchanged production visibility leaf as a standalone Rust test module. The Beeper test failed with the old predicate and both tests passed after the guard. This does not claim full native test-suite execution.
- Native demo build passed on Linux. Full native test compilation exceeds this environment's 2 GB memory; final CI and macOS/live-network interaction remain unverified.


## 2026-11-13 - Failure-only media send status

Branch ui/send-status, separate from group redesign. Sending/Accepted/Confirmed
media records paint no pending-status text. NotSubmitted paints an alert with
detail and Retry. Unknown paints an alert with Check history and the detail,
without a blind Retry because the original POST may have been accepted. Normal
sent/delivered/read receipts are unchanged. No send transport changes.

Red-green: a standalone test compiling the unchanged production State failed
for the absent failure presentation method; with the method it passes all five
states. cargo check --locked --lib passes. Full native tests and screenshot are
pending; no macOS/live-network verification claimed. Self-review checked that
Unknown cannot gain a duplicate-send path and that #322's media preview hunk
and #323/#324's contrast/line-wrap hunks are untouched.

Send-status native CI follow-up: one baseline Beeper fixture test failed with
ConnectionReset while 2,149 tests passed. The stub polls a nonblocking listener
but assumed accepted streams are blocking when using read timeouts. Explicitly
set accepted fixture sockets blocking so a preconnected or newly accepted
socket waits for request data rather than being closed on WouldBlock. This
changes test infrastructure only, not Beeper transport or send-status behavior.
## 2026-11-13 - Final group redesign

Branch feat/group-modes implements the approved consolidated sheet: Recent,
Focus, Needs me; static group sections; one group per chat; Other last; merged
manual later mark; next-local-08:00/forever mute; widening find and names-only
switcher. Beeper timed mute is local because the bridge has no duration API.
Old multi-memberships use the first saved section, with single-home assignment
on edits. No hide/collapse/group-focus controls remain.

Self-review caught retained old group toggle shortcuts, stale keyboard tests,
global-search network filtering, mode restore retaining the old queue, and
missing expiry repaint scheduling; addressed before push. Existing #322/#323
conversation changes and line-wrap work are untouched. #286 overlaps app/model/
keys/chats and may need a later rebase.

Validation: cargo check --locked --lib and --tests passed. Standalone assignment
probe fails on the unchanged baseline and passes on this branch; production
next-08:00 probe passes midnight/exact-08:00/DST cases. Initial production edits
preceded tests, so strict red-first is not claimed for the whole feature.
Full test linking/native demo code generation exceeds the local 120-second
execution limit; no test-suite or pixel-verification pass claimed. CI and
runtime capture remain required. No merge or release claimed.


Follow-up: maintainer removed the pre-existing Filters row after the first
capture. It exposed type/favorite/unreplied/category/network/account/label scopes
and link suggestions. Shared palette actions and predicates remain, while
mode selection clears extra scope and startup no longer restores hidden
type/favorite filters. Captures refreshed and visually inspected in both themes.
Updated obsolete favorite/draft ordering tests to the approved latest-first
Recent semantics, preserving legacy favorites-filter ordering. Native demo
build now passes after local shared-library setup; full tests remain on CI.


Maintainer picked sketch A for group headers: static 16-point thin dividers
with 11-point labels. Implemented the same treatment for Other, preserving
single-home row order and drop targets, with no click or collapse action.
Added a geometry/non-click regression (compile-red before helper, check-green
after). Refreshed native dark/light captures; no B/C/D sketch code shipped.

Native-suite follow-up: the grouped path now shares the list's persisted scroll
ID, slide-copy offset, gesture routing, near-top send behavior, and row geometry
instrumentation. Keyboard reveal accounts for static headers before drawing,
including virtualized rows. Recent/Focus/Needs me join the deliberate Tab order.
Migrated retired-chip/mute-duration/favorite-priority assertions to the approved
modes, 08:00 mute, and activity ordering; kept interaction and viewport coverage.
The retired saved scope is cleared from account settings as well as session state.

A standalone headless input probe also reproduced ordinary Tab getting stuck
at the grouped viewport edge on Settings. Rows now retain neighboring geometry
only while a list row holds focus outside the main chat cycle; group-creation
controls reveal their focus too. The same probe now reaches previously hidden
rows with fully visible focus rectangles. The resize-centering test waits for
sidebar/scrollbar settling rather than sampling a transient layout.

The same test-only Beeper fixture socket-mode fix as send-status PR #327 is
included here to keep the independent native CI legs deterministic. Production
Beeper transport is unchanged.

Devin correctly flagged the focused Settings-row fallback as unbounded. Replaced
it with three overscan rows on each viewport edge. The standalone forward Tab
probe still reaches hidden rows; the 1,000-chat benchmark now explicitly focuses
a row on Settings and checks four more frames stay below 20 widgets (13 built).

Second native suite: 2,153 passed and two failed. The resize test grabbed a row
edge, which varies with scrollbar width, instead of the panel resize boundary;
it now reads the panel edge directly. The mute test's retired Always label now
matches Mute forever, and the timed choice explicitly says Until 08:00 rather
than tomorrow (which was inaccurate before today's 08:00). Both new labels use
literal gettext calls.

Third native suite: 2,156 passed, only left-resize failed. A standalone real-input
probe reproduced actual width growth, so the previous test-coordinate explanation
was incomplete. Content minimums were widening egui's resizable Panel during a
left drag. The existing wider pointer strip now exclusively owns resizing, with
the panel fixed to its current saved width (260-520 bounds). Probe: starting
panel edge352, then352/347/342/337/332/327 as the pointer moves left, no widening.
Rebased onto merged send-status main, preserving both work-log entries and code.

Fourth native suite: 2,157 passed; resize passes. Fixed the final focused-row
visibility failure: the scroll area's content rectangle can be wider than its
clip while its scrollbar changes width. Grouped row widgets now cap their
right edge to the actual viewport clip, preserving fully visible keyboard focus.
The standalone forward-Tab probe passes after this change.

Fifth native run: all 2,158 library tests pass. The subsequent portable source
contract still required the retired Filters-menu suggestion list verbatim.
Chat-level Same person? cards remain in merged.rs, capped at three with accept
and dismiss actions. Migrated the contract to assert that shipped bounded path
and that the removed menu stays removed. Standalone rustc execution passes.


## 2026-11-17 About build identity

About reads the package-owned Resources/build-version.txt, written by bundle.sh before signing. Initialization runs before the event loop; About rendering reads memory only. The numeric Info.plist version and update ordering remain unchanged. Non-bundled binaries fall back to Cargo version. Package regression was red before the metadata writer, green after; all 8 package tests and standalone version-selection Rust test pass. Native CI and full packaged UI capture are still required.

## 2026-11-17: Cmd+B linked header

The linked-person renderer did not apply the traffic-light clearance used by ordinary chat headers. With only the 52-point icon rail left after Cmd+B, the green button overlapped the linked name. Added real-input coverage for repeated hide/show at three zoom levels in both themes. Capture and test results are recorded in the PR; never use private chat data as review media.

About build identity is tracked in a separate PR.


## 2026-11-17 - Local Whisper setup (approved sketch C, in progress)

Separate branch feat/local-whisper-setup based on main18416af. Explicit Build
approval follows the first-run sketch; no automatic model downloads. Current
work is a pinned, hash-verified, bounded atomic model installer using the
existing HTTP/hash crates and tempfile already in the lockfile. Move tempfile
from dev-only to regular dependency for private audio and atomic installation.

Planned: engine selection preserving Wyoming, model controls and progress,
signed bundled runtime from upstreamv1.9.5 commitd1be6fde11ac6e0407606b4e42fe72d34add8037,
explicit-auto language, portable bounded process output, shared decoded audio,
real-recording actual-app verification. Source policy/packaging, licensing
notices and all native validation remain to be completed. Not a working feature
yet; no PR or DMG claimed. #357/#358 stay on their separate release track.

Installer checkpoint: production installer test passed in an isolated crate.
Covers bad hash, truncation, oversize, cancellation, atomic success and cleanup.
No network/model download performed. Full library/tests compile was killed for
memory here; no whole-app compile pass claimed. Use serial jobs next time.
Model revision and exact size verified against the live distributor API.

Whisper source-independent checkpoint: explicit Off/Local/Wyoming selection
with legacy Wyoming migration, opt-in settings controls, progress/cancel and
remove actions, background download completion, shared decoded PCM to private
WAV, CLI auto-language, portable bounded output and timeout kill/reap.
Disk-space checks use existing libc on Unix and the OS API on Windows; no new
locked dependencies. Switched download transport to existing ureq for bounded
reads. Local cargo check/clippy --lib --features demo pass with one job.
15 installer/engine production-module isolated tests pass. Full test compile
is too large for this worker and was interrupted after 120 seconds. Native
CI, visual evidence, runtime source decision/bundling and real audio in actual
app remain open. No model download or bundled upstream code yet.

Owner correction 2026-11-17 18:41: download platform-specific prebuilt runtime
and model only on user action; no bundled runtime or Whisper build-time
dependencies. Original phonemsg-01M7N65ZM205EJ3XQEZM659VCK recovered.
Runtime path moved outside signed app bundle into app state. No executable
provider selected yet: official b5454 has Apple dynamic-library XCFramework,
not a macOS CLI. Archive inspected and published SHA256 matched. Linux/Windows
archives exist. Main has reported choices; do not silently choose a third-party
Mac CLI or add a helper distribution. No xattr bypass or weakened validation.

Separate optional helper prep: manual-only workflow fetches pinned official
source, builds standalone helper app, signs/notarizes/staples, checks quarantine
without stripping it, and emits exact archive metadata. No Solmu build invokes
it and no release asset is published automatically. Workflow and build script
are only prepared, not run. Publishing helper plus pinning output URL/size/hash
and implementing verified runtime installation still pending. Existing app
engine controls remain disabled until that runtime exists.

Helper arm64 run44130161632 passed build/sign/notarize/staple/Gatekeeper with
quarantine present and --help execution. Intel still pending. Archive metadata
/downloads/whisper-helper-arm64 captured; no release published. Runtime module
now stages archive, checks exact hash/size, signature+team requirement and
Gatekeeper, then atomically installs outside app bundle. Provider remains None
until a real published URL is available. Download button intentionally disabled.
Model worker now downloads/verifies runtime first, then model. No model fetched.
Actual-app real-recording test still unrun. Public helper download needs a
publish decision separately from unpublished DMG; parent notified.

Adversarial review follow-up: restrict helper job AND caller to public default
branch before signing secrets are used. No more branch dispatch signing.
Arm64 artifact passed; Intel build succeeded but notary credential setup failed
without exposing credentials; remains unverified. Runtime retry replaces invalid
version directory only after fresh verified archive is staged. Runtime signature
and hash rechecked for every transcription, model hash once per worker. A fully
malicious same-user process can still race file paths; no stronger claim made.
ProviderNone and actual-app/mac-specific installer tests remain merge blockers.


## 2026-11-17 - Local Whisper settings presence review

Moved the model-presence metadata probe out of settings drawing to a background
cache with generation-checked results and a repaint notification. Refresh on
startup, engine selection, completed/cancelled/failed install, and model removal.
The view reads only cached state and shows checking while a probe is pending.
A model can be removed even while the runtime provider is unavailable.

Failing-first: the blocked-probe regression failed against synchronous refresh
(refresh waited for a blocked filesystem probe), then passed after background
execution. Three standalone Rust tests pass: blocked probe plus repeated cached
reads, stale blocked result cannot overwrite newer removal, and real file
create/remove refresh with wake. Native rendered-settings regression checks that
file deletion cannot change a repaint until explicit background refresh. Local
library/tests demo clippy and formatting pass. Native execution and new synthetic
light/dark before/after captures are pending. This PR remains unmerged and is not
part of the current DMG; no helper build or publication was resumed.
## 2026-11-17 - Readable sidebar, approved sketch A

Recent replaces All, with no unread/age/mute restriction. Removed the seven-day
unread mode and migrate its saved value and legacy native command to Recent.
Three named buttons and their descriptions replace icon-only tabs. Local group
creation moves to a + beside Groups, with an on-demand, cancellable name popup;
all changes are local and preserve original chats. New native synthetic capture
scenes cover both themes. Existing Unread/Needs me logic and grouping stay intact.

Validation: cargo fmt, cargo check --lib --tests --features demo and library
clippy -D warnings passed locally. Full native tests and pixels await CI.
Regression coverage exercises named labels, popup creation/cancel in both
layouts, old saved-mode migration, mode cycling and no date/unread/mute cutoff.
No sidebar release build yet; combine with About #357 after review and CI.

Sidebar review follow-up: localized the + tooltip/accessibility label and moved
its focus stop after Needs me to match visual and tested order. Both library
and test-target clippy passed locally before this small correction. Screenshot
evidence still awaits native CI; the review gate remains open.

Native #358 gate found popup focus requested during an invisible sizing pass,
a tour cue still seeking the retired All mode label, and a tooltip fixture
whose target moved below the viewport with the fuller header. Focus now waits
for the visible popup pass; tour targets the semantic Recent stop; tooltip
fixture explicitly uses compact rows, matching its existing row-stride math.
Rerun native gate before treating these corrections as validated.

Second native gate: tour and hover fixed; group popup focus/input/create pass.
Escape remained consumed by TextEdit before Popup saw it. Capture Escape
before drawing the field, clear the draft and close explicitly; regression
also checks popup state rather than only a previous-frame widget response.

## 2026-11-17 - Voice and video shared-item sections

Separate Photos/Videos/Voice tabs, bounded metadata-only Beeper listing worker (the old command was ignored), explicit refresh, and audio download/play/jump rows. Per-kind caps cannot let photos crowd out video or audio. Download results update panel rows even outside the loaded transcript. No schema/settings migration. Local library clippy passes; native tests, before/after synthetic screenshots and actual Devin gate remain pending. No private screenshot reused, no release built.

## 2026-11-18 - Optional runtime all-platform follow-up

Merged current main without reverting its transcript, sidebar or workflow
changes. Runtime remains unavailable until approved published URLs and measured
hashes exist; no release publication is part of this checkpoint. Cross-platform
installer now accepts the separately built bin/whisper-cli or bin/whisper-cli.exe
layout, with pinned archive/executable hashes and staged repair. macOS keeps
Developer ID and Gatekeeper checks. Runtime tree fingerprints protect subsequent
jobs without rerunning OS assessment each time.

Review follow-up: reuse valid models, disable unavailable Local selection, clean
owned crash leftovers under an advisory lock, remove runtime and model off the UI
thread, retain the selected engine on removal failure, and avoid creating install
directories for users who never downloaded. Bounded transcription preserves Ogg
fallback and downsamples incrementally before the five-minute PCM cap. Changelog
states that this limit applies to Wyoming too. Local demo library/test Clippy
passes; 27 production-module engine/installer tests and ten workflow tests pass.
Native regressions and all-platform real application verification remain pending.
## 2026-11-18 - Whisper helper packaging matrix

Six-platform packaging pipeline for the optional Whisper helper (#359 option A), separate from the signed default-branch macOS workflow: `packaging/whisper-helper/` (pins, fetch, Linux/Windows builders, smoke tests, metadata emitters, runtime README template) plus `whisper-helper-matrix.yml`. Linux builds in a debian:12 container (Debian 12 baseline, ldd allowlist guard, OpenMP off so no libgomp), Windows links the MSVC runtime statically and packs .tar.gz with system tar.exe, macOS reuses the existing unsigned bundle script. Every leg runs an inference smoke test (pinned tiny model + JFK sample) and prints per-target metadata JSON; Linux and Windows also run the #359 installer's real-app smoke test (pinned installer commit) with the produced archive, verified Base model and public JFK sample; unsigned macOS legs skip it because Gatekeeper verification must not be bypassed. Nothing is uploaded, cached or published, and the workflow requests no secrets. The linux/x86_64 path was run end-to-end locally (fetch, build, archive, smoke, metadata); the other five legs are CI-verified on the PR. Final pins for `src/whisper_runtime.rs` are computed from the published archives (signed/notarized on macOS) when the maintainer publishes.

CI round 1 fixes: PowerShell pin parsing now strips pins.env's bash single quotes (shared read-pins.ps1; the phrase check had failed on a correct transcript); Windows ARM64 builds with GGML_NATIVE unset (pinned GGML requires MSVC ARM native; MSVC targets the ARMv8.0 baseline anyway); Linux metadata step chowns the root-owned docker output first. macOS legs were already green.

## 2026-11-18 - Group by section header switch

Approved option A: removed mode explainer text and replaced the passive Grouped/Flat list label with the shared switch widget. Existing Cmd/Ctrl+G and the switch use ToggleListLayout and keep per-mode settings. Shortcut appears only on hover; + remains independent. Focus order and pointer/keyboard behaviour have regression coverage. Tests, synthetic before/after pixels and reviews pending. Incoming-message section sorting is separately coordinated.

## 2026-11-18 - Long bubble collapse, approved label A

Separate feature branch. Bodies over 25 rendered rows are clipped after row 25,
including blank lines, with Show more (+N lines) and Show less. Expansion is
session-only and scoped to local account, source and text revision. The complete
message/markup/transcript stays intact for copy, reply and forward; link targets
are clipped with the body. Hidden count is recomputed at the actual wrap width.
Failing-first row-limit test observed, then fixed. Native pointer toggle, resize,
blank-line and full-transcript coverage plus real dark/light collapsed/expanded
capture scenes added; native results pending. No avatar, menus, #359 or DMG work.
## 2026-11-18 - Local avatar customization (Contact info entry)

Approved option A: Customize avatar in Contact/Group info opens a local editor
with image or initials/colour, preview, Save/Cancel and Reset to default. No
right-click entry added. A shared avatar-painter override covers sidebar,
favourites, chat rows, headers and info without changing source profile photos.
Keys include the local account and exact source/account/chat identity; linked
members share the override. Image import is bounded, decoded and re-encoded as
a metadata-free square PNG off the UI thread. Prepared-image write must succeed
before settings adopt it. Files stay outside the disposable source-photo cache.

Failing first: production image import tests rejected the pass-through stub on
invalid image data. The fixed image/identity module passes three isolated Rust
tests. Native UI tests cover opening from info, cancellation, save/reset,
pending-picker cancellation, image persistence and linked identity handling.
Native compile/tests and real dark/light demo shots are running through the
existing shots-only workflow; no native result or visual verification claimed
yet. No #359, helper packaging or release/DMG work included.
## 2026-11-18 - Context menu key hints, approved sketches

Separate feature branch. Message menu owns exact Reply Cmd-R, Star Opt-Cmd-S,
Pin Opt-Cmd-P, Save Opt-Cmd-B and existing Forward Cmd-L chords against its
message. Chat menu owns Archive Cmd-E, Deal with later Cmd-I and Info Opt-Cmd-I
against its chat, not the open background chat. Global shortcuts defer while a
popup is open. Hints use the shared shortcut registry/platform notation. No
avatar right-click entry. Native target/collision tests and dark/light capture
scenes added; results pending. No release, #359 or DMG work.

## 2026-11-19 - Code panel row kinds repair (#388), hyphen-in-code root fix

PR #388 fixes the five code-panel test failures #382 merged with. Row kinds
now place blank rows by their newline's character position inside a block's
content bounds instead of inheriting the previous row's kind, so the dropped
fence markers' blank rows stay plain while a blank line inside a fence stays
code. Code tokens no longer hyphenate, since a hyphen in code shows a
character that was never typed. That did not fix the width failure: CI on
4ea9c98 still measured 22-char rows at 201.3125pt in the 200pt panel. The
real cause sits in the balancing planner. Every mid-word split charged
FORCED_SPLIT_PENALTY (1M), so packing an unbreakable string into more lines
cost millions per extra line, while overflowing the cap by 1.4375pt cost
slack^2 x OVERFLOW_PENALTY, about 2000, per row. The planner saved lines
and overran the cap on every row. Now only a word's first forced split
pays the penalty; splits of an already-split word are free, so a fitting
plan always beats an overflowing one. A short word wider than the cap
still overflows in one piece rather than splitting. Reproduced against the
real fonts and egui layout in a local lab (rows now 192.1875pt) and pinned
by a failing-first planner test at the exact CI metrics. The width test is
back to a 0.5 px bound, and a new test fails on any hyphen glyph in a
laid-out code block. No release, #359
or DMG work.

## 2026-11-19 - Copy code button floats above the panel (overlap repair)

The Copy code button painted at the panel's top right inside it, over the
code text whenever a row reached the right edge (visible on narrow panels
like two short shell commands). It now hangs off the panel's top right, in
the blank row the opening fence's dropped marker leaves, so it can never
cover code. A first version hung it below the panel, but a collapse clip
cuts a long block mid-panel and the clamped panel bottom sits at the clip,
which left the button outside the visible body and unreachable (Devin
finding on d0034ec). Top placement stays visible whenever any of the
block is. The panel geometry is shared (code_panel_rects/
copy_button_rect); one test asserts the button rect intersects no code
row, another that it stays above a mid-block clip. A third pins the
button's actual response rect: Ui::put expands a themed Button to the
28pt minimum interact height, which leaked nine points into the first
code row and intercepted selection drags (Devin finding on e4286bc), so
the label was first painted by hand over an exact allocation in the
reserved band, but that control had no accessible label (Devin finding):
it is now a real Button in a child UI whose interact height is capped at
the band, pinned by an accesskit tree test. No release, #359 or DMG
work.

## 2026-11-19 - Outgoing collapsed text alignment

Owner screenshot shows long outgoing text left of its bubble. The collapsed
body painted a child at the cursor minimum before the right-aligned parent
allocated the row. Allocate first and use that exact rect for painting and
clipping. Incoming and outgoing containment regression added, plus synthetic
outgoing demo fixture and an explicit before/after evidence workflow. No local
Rust toolchain; CI and evidence run must validate before merge. No DMG dispatch.
## 2026-11-19 - Local message display masks

Distinct Hide in Solmu action, not bridge Delete for me. A small placeholder
keeps Unhide available after restart. Only exact source/message identities are
persisted beside local bookmarks; original messages are not deleted. Hidden
chat albums use individual rows so masks cannot hide sibling attachments.
Tests cover identity isolation, serde round trip and no backend command.
No local Rust toolchain; CI and synthetic before/after evidence still pending.
No DMG dispatch.

## 2026-11-19 - Reveal focused local Unhide controls

The hidden-message early return skipped the conversation focus reveal path.
Call reveal_focus on Unhide before custom Tab registration and preserve
keyboard-navigation mode, so offscreen placeholders scroll into view. Added
a small-scroll-area regression that tabs to a placeholder below the viewport.
No appearance change; prior inspected synthetic captures remain representative.
Local Rust execution unavailable; CI must validate this regression.


## 2026-11-19: Fix PR #402 CI (clippy + album viewer regression)

- Clippy: the `Api::download_media` convenience wrapper is only called by
  unit tests, so `cargo clippy` flagged it as dead code. Marked it
  `#[cfg(test)]`.
- `the_album_follows_a_chat_while_the_viewer_is_open`: the rewritten
  `MessageDeleted` handler collected removed ids only from the loaded
  conversation, so a message present only in the viewer's album (the
  fixture's case, and real chats whose page is not loaded) stayed in the
  viewer. The handler now also collects matching ids from `viewer_media`
  before forgetting them.

## 2026-11-19: Keep the selected attachment across forward retries (Devin security finding)

Devin's re-review found that a failed plan dropped the attachment selection:
`plan_forwards` canonicalized the source id and only then fetched the message,
so a fetch failure left the retried item with neither its suffix nor
`item.attachment`, and the retry expanded to every attachment. The selection
is now recorded on the item before the fallible fetch, and a regression test
drives a failed plan followed by a retry through the mock server.

## 2026-11-20: Remove the bridge album evidence workflow before merge

Owner decision: the per-PR evidence workflows are not kept. The
`bridge-album-evidence.yml` workflow (failing-first test plus before/after
captures in both themes) served its purpose on this PR and is removed before
merge; the mapping and regression tests it proved remain in normal CI.

## 2026-11-20 - Reject false-green server inventory diagnostics

During the existing-server reliability audit, the read-only endpoint checker
accepted an error-shaped chat or history response as an empty successful page.
Validate account and chat identities, items arrays, pagination flag and cursor
types, and sampled history structure before printing a summary. Invalid server
JSON yields a nonzero exit with no private payload. Failing-first regressions
cover these cases, valid empty inventory, and the CLI exit/output boundary.
No real accounts queried, no native Mac run, no release or DMG dispatch.

## 2026-11-20 - Keep state when Server pages omit items

The reliability audit found the app adapter also accepted error-shaped HTTP200
JSON as an empty final page because items defaulted to an empty vector. Require
the items array so existing error paths preserve the last good chat list,
history and search continuation. Tests cover disconnected inventory without a
Chats replacement event, legitimate empty inventory, malformed history, and
search retaining results/cursor across refusal and successful retry.
No real Server query or UI-layout changes; native/live acceptance unverified.
## 2026-11-20 - Unknown reconciliation on malformed Server search

## Attachment-pencil handler,20 November

Production previously neither compiled the picture-edit module nor routed InspectPicture/ApplyPictureEdit. Both now run through the existing local attachment-action worker, independently of Server/offline status, emitting targeted results. Output goes to uniquely named disposable beeper-edited-* files at the account media cache root, registered for ownership, trimming and clearing protection, never over the original. Thread-start failures answer the waiting action. Existing attachment-ID checks still reject stale results.

Native library/test-target compile passes.418 portable adapter tests pass, including real routing that inspects an8x6 PNG, applies3x4 crop, verifies output dimensions and untouched source, and receives a missing-file error while disconnected. Harness stages real picture_edit/sticker_store sources. No live Mac pencil/crop/save roundtrip or new UI design claimed. A local disk-cache-full interruption was recovered by clearing disposable build caches before rerunning the suite.
The Server reliability audit found that the scheduler interpreted a missing
search items array as NotFound. That is not evidence that an uncertain send
has no existing copy. Require items so malformed search leaves reconciliation
Unknown. A failing-first real-HTTP regression reproduced NotFound before the
fix; valid-empty pages still report NotFound. No automatic resend policy is
changed, and no real messages or Server configuration were touched.


## Shortcut edit targets,20 November

Owner reports Up selected an older message. Source confirms the shortcut deliberately skips non-text rows (Beeper attachment edits are unsupported), not an index decrement. Conversation merge sorts by timestamp/history order. A separate real gap reproduced: hidden, Pending and Failed own text rows could become shortcut edit targets. Reject unsent rows in can_edit and hidden rows in the shortcut. Failing-first extracted production-method fixture:3fail/1pass before,4pass after. Added native App regression for hidden/pending/failed, latest sent/read and captioned album-member boundaries. The extraction uses simplified model scaffolding, not the full App suite. Full native CI remains required. Original older-target screenshot cause not proven; no cache/off-by-one claim. No editing-highlight UI changed.
## Code block breathing room,20 November

Fenced code has10pt horizontal inset, extra bottom space,1.4x line height and a reserved20pt accessible Copy code header inside the panel. Layout row positions, not copyable indentation/blank lines, supply the spacing, in cached and fresh paths. Actual synthetic before/after Linux screenshots inspected in dark/light: header separates from code, left/bottom no longer crammed, prose remains outside. New geometry/copy regression plus updated response-height assertion compile; execution/Mac/PR-hosted evidence remain gates. No release.
## Transcript reading alignment,20 November

Ready voice/video transcript text now forces left alignment independently of outgoing bubble placement and has plain double quotation marks. New layout regression checks quoted multiline text has a common left edge under an outgoing layout. Both-theme actual synthetic Linux outgoing voice captures inspected. Native test-target compile passes; native test execution and Mac pixels remain gates. Richer transcript styling/collapse and moving the action outside the bubble remain sketch-only.
## Finder media-tool resolution,20 November

Video transcription invoked ffmpeg by bare name; Finder-launched macOS apps may lack Homebrew in PATH even when the tool is installed. Resolve executable absolute paths from PATH then macOS Homebrew Intel/ARM and MacPorts bin directories. Use the same resolver for animation ffmpeg and ffprobe. Missing transcription tool errors list searched directories. No existing ffmpeg override was found. Focused resolver tests pass with fake install directories, precedence, missing-tool diagnostics and relative-directory rejection; the fallback discovery test failed before resolution was implemented. Native library/test-target compilation passes. Real attached MP4 audio extraction passed with the same ffmpeg arguments separately; packaged Mac acceptance is unverified. No new UI or package publication.
## Readable batch-result banner,20 November

Batch archive/delete result notices use resolved chat titles and explicit Archive chat/Delete message for me/Delete message for everyone labels, not raw room/message IDs. Accepted is explicitly not completion confirmation; waiting and failed/uncertain states say not to blindly retry. Technical error details collapse, and dismiss clearly says action results. Previous-connection history never resolves an old identity against the current session. Missing chats say Unavailable chat.

Native test-target compile passes, regression covers labels/states/no raw IDs; execution pending native CI. Actual synthetic Linux dark/light screenshots inspected: wrapped label and dismiss button are readable within the sidebar. No Mac pass.

## Group-member chat search,20 November

Cmd-K, global/sidebar chat matching and palette include full loaded member display names and bridge-provided names for named groups, without changing the visible group title or fetching metadata from paint. Locked groups stay excluded from switcher/palette. Unknown or unloaded membership cannot match yet. Contact alias editing remains sketch-only.

Production helper regression fails with the previous title-only terms and passes after including members. Native test-target compile passes with app-level regression covering Cmd-K/global/palette/title/locked exclusion. Full native test execution was attempted before the fix but exceeded local build timeout, so no claim that the actual end-to-end regression has run red/green; CI must execute it. No Mac pass.

## Loaded reply threads and routed connectors,20 November

Original messages show their known loaded reply count, opening an in-place thread with Back to full chat. The full-chat scroll state is retained separately. Reply graph identities include source chat and message; hidden/revoked/unloaded sources do not contribute. Shared rendering covers ordinary and linked views. Hover/focus connectors use exact inner-facing bubble-border ports and an orthogonal visibility grid with bend/length costs and padded obstacles; corners are rounded within clearance. No safe route or offscreen endpoints produces no line; Original jump remains available.

Three production graph tests and three production router tests executed directly, all pass. Native library/test-target Clippy passes with warnings denied and actual Linux binary builds. Full native app test execution did not finish within local compilation timeout. Actual synthetic app count/filter/hover/back pixels inspected; no live account use. Review media remains private; maintainer waived the screenshot requirement for this PR only at14:27. No merge or release.

## Authenticated automation product decision,20 November

Updated PRODUCT.md and CONTRIBUTING.md to permit scoped authenticated API, CLI and MCP read/send access including WhatsApp, replacing the previous blanket exclusion. Read grants do not authorize writes. Revocation, explicit destination limits, loopback default, request validation, metadata-only auditing and remote-transport decisions remain requirements. Documentation only; no server, grant, token, installation or send created.

## Thread CI follow-up,20 November

Mac CI executed the thread changes and found new test texture-delta cleanup
mistakes plus album accessibility/navigation regressions: using any reply link
to force every message into a separate row split unrelated media albums. Keep
the normal album grouping and register each member's exact thread identity on
the album's shared painted bounds. Clear texture deltas in the new seeded
fixtures. Native all-target/all-feature Clippy passes; local targeted application
test execution exceeded120 seconds during compilation and has not run. CI must
recheck the three existing demo failures, including right-click-beside-message.

## Restore overlapping consecutive bubbles,20 November

Regression traced to #355, which replaced #246 seam overlap with separated rows. Restore6px overlap using current-pass predecessor geometry, older-edge repaint and covered hit-test clipping. Keep current width bands and every-body top tails. Selection and hard-boundary rows remain separate. Native tests updated for geometry, scroll and covered menus, and both-theme group-stack app captures added to the existing Mac CI smoke path. No real chat sent, no package published. Local native build/check execution remains pending after missing Linux development dependencies and bounded compile-time attempts.

Review follow-up: clip enlarged text-selection targets below the overlap cover, including command-click and double-click gestures. Reserve the full nine-point replay strip as bottom padding in both skins so background repaint cannot erase older content. Added seam gesture and padding regressions. Actual Linux synthetic both-theme group-stack captures inspected: final glyph rows and colored shadows remain readable. Native test linking exceeded the local execution bound; test-target compilation and fresh CI are distinct checks, not a claimed runtime pass.

### 2026-11-20: combined tail and overlap regression fixtures

The macOS failures exposed two obsolete per-message tail assertions and a
synthetic command-click that omitted egui's held-modifier event. Assert tails
only on run heads, require tail-less follower shadows, and emit
ModifiersChanged before the seam click. No production behavior changed.
All-target, all-feature Clippy passes. Native targeted test compilation timed
out while rebuilding dependencies; CI must execute the tests before merge.
## 2026-11-20: Reconstruct Markdown when editing rich messages

Owner requested retaining editable Markdown, then explicitly chose reverse
conversion rather than duplicate source storage. Prepare edit reads the current
exact own message, reconstructs Markdown from rich HTML, and verifies a local
CommonMark semantic round-trip. Unsupported or lossy constructs refuse before
changing the composer. A request generation, local account, chat, target and
unchanged draft guard reject late replies. No write occurs until explicit send.
Both fresh sends and edits pass Markdown in the API text field. Beeper's current
docs say both use the same rich-text conversion. Retrieval exposes rich text,
not original marker spelling. No source store or schema migration was added.

412 portable production-source tests pass, including rich reconstruction and
exact message/account/ownership refusals; native all-target/all-feature Clippy
passes. Four converter tests include13 markdown-rich-markdown-rich cases with
byte-identical CommonMark rich output. This is not live Beeper/WhatsApp proof.
Full native UI tests unrun. Demo compilation was killed by the environment;
visual capture remains unverified, not waived. No real message/account touched.

Review follow-up: validate every rich element and attribute before conversion,
including code wrappers and list items. Refuse nested code-block breaks/links,
styled code/list items and reversed lists. Preparation errors are structured
Unsupported/Unavailable values, translated with literal gettext at the UI
boundary; backend detail strings do not reach the toast. 412 portable tests pass
again and native all-target/all-feature Clippy passes. Private actual app
before-dark, after-dark and narrow-light pixels now inspected after a reduced
debug binary build. Composer shows Markdown hyphens and bold markers rather than
rendered bullets. No screenshots published; per-PR media gate remains held.

Added an executed local HTTP wire regression: fresh POST and edit PUT both
submit the same Markdown text byte-for-byte in the JSON text field, never
display bullets/renderer markers. 413 portable tests pass; Clippy passes.
This verifies Solmu's transport contract, not Beeper's downstream conversion.
## Scoped message automation,20 November

Implemented exact account/chat messages:send grants, old-read-token refusal,
bounded authenticated loopback POST, metadata-only durable claim/outcome audit,
revocation rechecks after identity lookup and disk claim, and no automatic retry.
Added solmu-control text-file CLI and stdio solmu-mcp using the same API. Modern
MCP2026-07-28 per-request metadata/discovery plus2025-11-25 compatibility; no
remote MCP listener. Clients disable redirects, proxy routing and transport retries.
Current app code constructs only the Beeper backend, so this serves the configured
network connections including WhatsApp through Beeper. Earlier inherited native
WhatsApp-mode notes do not describe a second live backend in this fork. Multiple
origins and direct/standalone protocol login remain out of scope for this slice.

Executed109 tests in an isolated build using actual production modules (only
merged Person/Member data shapes stubbed):85 unit tests,16 existing readHTTP,
4 writeHTTP/actualCLI,4 MCP/CLI protocol tests. Native lib/bins/test-target Clippy
passed. Own review found and fixed POST pagination leakage, unbounded total socket
read time, stale-grant check after disk claim, invalid token options, and Windows
directory-sync incompatibility. Final all-target/all-feature native Clippy, formatting and diff checks pass;407 existing portable adapter tests also pass. Own adversarial review found no remaining issue in these paths.
No real token, service, account query, live send, media upload or release created.
Full application tests and native Mac execution not run locally.

## Automation independent-review fixes,20 November

Devin found five real issues on8b46654: stale PID-lock recovery could delete a
new owner's lock and lose send claims, Windows PID locks never recover after a
crash, empty relative parent broke Unix directory sync, local people paths were
rewritten to the upstream root, and early denied HTTP sends missed receipt audit.
Journal transactions now hold a cross-platform OS file lock, never unlink it,
and release automatically on process exit. Local people paths stay local, empty
parents mean current directory, HTTP POST receipts precede denial paths and
unclassifiable malformed heads get anonymous receipts.113 production-module tests
pass, including a killed lock-holder subprocess followed by eight concurrent
receipt/claim writers proving one retained claim, relative-path restart, denied
HTTP receipts, and actual-client people/history routes. Native all-target/all-
feature Clippy passes. Windows runtime and fresh independent CI/review pending.

## New-message Mac CI follow-up,20 November

Mac CI executed2466 passing tests and found two failures: the background composer
still inserted Enter while audience results owned shortcuts, and a new viewport
fixture dropped texture deltas. Disable the background composer UI and surrender
its text focus while results are visible, rather than merely blocking Send.
Clear the fixture's deltas. Native all-target/all-feature Clippy passes; these
actual app regressions await CI rerun, not claimed executed locally.

## Compact list layout,20 November

Claim: Instinct, branch `feat/compact-list-layout`, worktree
`/home/sandbox/solmu-compact-lists`. Owner chose sketch1. Wrapped list rows now
hang under their content rather than the bullet. Marker width is measured once
per prepared body and retained with its cached metadata. The balanced and
greedy paths reserve the indent; inserted break bytes still map to unchanged
copy text. Ordinary/RTL text is unchanged; monospaced rows are not shifted.
The separate Markdown fidelity PR owns source HTML's doubled list newlines,
not this branch. No invented spaces inserted into the message or bold spans.

Four helper tests executed against actual egui galleys: prefix identification,
non-list refusals, greedy continuation rows, balanced inserted-newline handling.
Native library and test-target compilation passed; new complete markup tests
compiled but not executed locally. Private before/after pixels from a smaller
synthetic egui renderer fixture were inspected: continuation rows align with
content. These are not native app captures; whole-app layout, interactions and
PR-media requirement remain unverified. No screenshot waiver assumed for this
new PR; no committed or externally hosted review images. No native link retry.

#436 first macOS run: 2496 tests passed, one new fixture failed before its
assertions because it used the default context without installing the app's
named Inter bold font. Both new fixtures now call `theme::install`, matching
the existing prepared-layout tests. No production fallback or assertion was
removed. All selected CI jobs and Devin must pass again on the new head.
## Markdown fidelity audit follow-up,20 November

Claim: Instinct, branch `fix/markdown-fidelity-audit`, worktree
`/home/sandbox/solmu-markdown-fidelity`. Owner approved fixing the audited
losses as one separate PR. This branch is based on #434 pending its merge;
not part of #434, #435, or the separately approved compact list layout.

First failing-first regression reproduces hard-break and mixed nested-list
edit refusal. Source signatures were counting CommonMark serialization
newlines after br and before nested lists as inline spaces. Normalize those
boundaries only, retaining actual inline spaces. Both fixtures now pass,
with all six reverse-converter tests passing in the production-source
portable harness, including the original13 CommonMark round-trip cases and
unsafe-markup refusals. No display-adapter audit fixes yet. Full native
CI/review and broader audit regression suite still required.

Display-audit progress: failing-first tests reproduce heading-level loss and
pretty-list blank lines. Preserve heading level as explicit # prefixes,
paragraphs as blank-line-separated blocks versus a single br, nested br in
pre as code newlines, table cells as a readable delimiter-separated fallback
without leaked HTML tags, and ignore whitespace-only HTML list source runs.
Updated three existing paragraph fixtures for the intentional separation.
429 portable production-source tests pass with this in-progress branch.
Escaped-literal markers and reversed ordered lists are still unresolved; no
claim that the full audit is fixed and no PR opened before that work is ready.
## Preserve ordered rich-text lists, 20 November

Claim: Instinct, branch `fix/preserve-ordered-rich-lists`, worktree
`/home/sandbox/solmu-ordered-lists`. Reported numbered sends appeared as bullets.
Root cause is the display adapter: `plain_rich_text` ignored ol/ul context and
forced every li to a bullet, with an old fixture explicitly blessing that loss.
Outbound `send_text` already posts the original Markdown without this converter.
Track nested list counters and numeric start/value attributes, preserve ul
bullets, and share the existing conservative attribute scanner with href.
Indentation is capped and counter overflow saturates. No bullet spacing or bold
layout implementation; those remain sketches awaiting the owner's choice.

Failing-first extracted production-function reproduction: nested ol start=3
returned bullets before the fix, then passed with numbering. Full portable
adapter suite passed 422 tests, including a local HTTP wire check that numbered
Markdown is identical on send POST and edit PUT. Applying this patch to #434's
current source passed 427 portable tests, including the reverse converter and
its CommonMark round trips. Full native Clippy hit the execution time limit
twice, without a diagnostic; not claimed passed. Actual downstream WhatsApp
receipt was not inspected and no real message was sent. No screenshot waiver
for this PR assumed; separate ready PR with its missing review media noted.

## Actor-aware reaction previews,20 November

Preserve original ownership separately from aggregate reaction actors. Incoming reactions to own message say to your message, our reaction to theirs says You reacted ... to the message. Mixed/unknown identities use neutral Reactions wording, never infer latest from array order. Chat inventory and history/poll mapper use account identity; own originals can identify self from their sender. No delivery tick or original group-sender prefix on reaction previews. Owner approved English-only reaction wording now and full i18n as future roadmap work, not implementation. Native compilation passes; tests and app pixels pending CI.

Owner confirmed English-only reaction wording and later full i18n on20November.
Recorded future i18n in ROADMAP.md only, no locale implementation or catalog edits.
Each English string has its own literal gettext call for later extraction.
Exact portable staging passes429 tests, including actor/ownership, unknown/mixed
identity and reaction-order regressions. Native test-target compilation passes.
No native app captures are available and no per-PR screenshot waiver assumed.
No claims of latest reactor from an unordered aggregate, no real messages or release.

#438 review follow-up: both sidebar search predicates now match reaction
emoji metadata separately from the original summary. Added a native regression
for emoji-only queries, scoped outside-match counts and locked-chat exclusion.
Native test-target compilation passes; native execution awaits fresh CI.
The media-only finding remains open, no waiver assumed.
## Markdown fidelity audit completion,20 November

The audit fixes each reported readback loss separately: heading prefixes,
paragraph separation versus a hard break, breaks within preformatted text,
table cells/rows without leaked HTML tags, source-whitespace-only list runs,
reversed ordered counters (including direct-item-count defaults and minimum
integer saturation), and literal HTML style markers. Literal characters carry
a nonprinting renderer hint distinct from emitted semantic styles; inline
parsing removes the hint for rendering, copy and link detection. Native
backslashes and the shrug's slash-adjacent underscores stay untouched. Link
label comparison strips only those marker hints before comparing destinations.

Nested mixed lists and single hard breaks now reconstruct safely for editing.
Each refusal change has a failing-first regression and safe/unsafe round-trip
checks. Unsupported semantics still fail closed. Exact source spelling is not
recovered; original HTML remains the source, not a second stored Markdown copy.

Validation: 437 portable adapter/reverse-edit tests pass. Replacing the harness's
markup leaf stub with extracted, unchanged production text-parser functions and
actual mention-token helpers, plus one mapper-to-parser acceptance test, passes
463 tests, including 25 text-parser tests. This is not a whole-app test run.
The reverse-list and literal-marker new tests failed before their fixes. Native
`cargo check --locked --tests -j1` passes with incremental disabled; native tests
are compiled, not run. A preliminary check exhausted disk space; old local
build caches were removed and the check completed. No native linking retry.
Review screenshots and actual Beeper/WhatsApp bridge round trips remain absent;
no new per-PR visual waiver is assumed. Existing #434 and #435 waivers do not
cover this follow-up. No review media committed/hosted and no real sends.

#437 review follow-up: literal marker hints are now normalized at loaded
history search, archive SQL matching and result-snippet boundaries, not only
at painting/copy. A shared borrowed normalizer keeps unhinted loaded search's
no-allocation path. SQL removes only marker-plus-hint pairs, preserving native
separators elsewhere. Added mapper-to-loaded-search/snippet regressions and
native archive global/scoped-search tests. Actual production SQL was executed
against text, caption, URL and literal-marker rows. Native test-target check
passes; portable suite with actual parser/search functions passes. Full native
archive-test execution still awaits CI. Rendering representation is unchanged.

#437 second review follow-up: text_matching now normalizes only its matching
predicate and retains the protected source line for preview_line's one parse.
Added a native application regression through text_matching and preview_line
for literal stars, standalone backticks and filenames. The shared normalizer
is now a small real module staged by the portable checker instead of a helper
hidden behind its util stub. The repository's exact portable staging command
passes438 tests; native test-target compilation passes. Retargeted to main
after #434 merged; retained all follow-up fixes while reconciling its squash.
#433 Linux all-features run passed2514 tests and failed the linked-minute
fixture's first-clock assertion. Its WhatsApp times are fixed at today's18:40,
while Signal was still relative to wall time; near18:40 those samples
interleaved and correctly split the source run. The linked-minute fixture now
places Signal after the replaced WhatsApp run, preserving both sources. The
regression freezes18:40UTC, the problematic period, without loosening any clock
assertion. Native test-target compilation passes; full execution awaits CI.
Production bubble/footer logic is unchanged.

The same linked-minute fixture failure affected #437 Ubuntu (2523 passed, one failed). Reused the identical fixture-only correction from #433; no audit or production code changed. All exact-head CI/review gates restart.

#424 review follow-up: natural width planning previously omitted the new code inset, then rendering subtracted it from the already settled text width. Fenced bodies now use the same cached padded layout for both timeline planning and per-bubble fallback. Added a14-line conversation width regression keeping the snippet below the collapse threshold. Native all-feature check passes; runtime remains CI-gated.

#424 Mac runtime:2546 passed,2 failed. The compact-list fixture still expected the pre-inset180pt code cap; it now expects160pt while retaining zero list-indent assertion. New narrow-code regression caught shaped overflow after balancing disabled wrapping. Fenced prepared jobs now retain their hard inner shaping cap, including trailing whitespace. No assertions removed; width/copy regressions rerun in CI.

#424 zoom review: prepared and break-plan caches now key the current pixel scale as well as point width and font generation. Regression cycles five scales at unchanged widths and compares cached row classifications/geometry with freshly prepared layout. This prevents scale rounding from reshaping a finite-width code job against old row metadata.

## Terminal read synchronization404,20 November

Linked-person reads fan out to each exact source. The read command used the generic Info toast for every HTTP failure, unlike history/poll terminal404 handling. A missing source now becomes unavailable for read synchronization until reconnect, logged once without identifiers. Local read clearing remains independent; no remote receipt is claimed. Other sources still synchronize;403/500 and transport errors stay visible. Read-only never posts. Three focused production-module tests pass, with the404 test first failing against the previous generic-failure behavior. Native test-target compilation passes; no live Server or Mac validation. No package publication.
## 2026-11-20 - Send Later uses the chat bridge identity

The app sent its local profile number as account_id, so the scheduler correctly
rejected real bridged chats before queuing. Resolve the account from the
selected chat instead. Unknown or blank identities stop locally without
clearing the draft, and linked-person preferences cannot change the account.
App regressions cover routing, quote/replacement preservation, and refusal.
A real-daemon fixture test verifies a separate enqueue client exits before
due time and the daemon still delivers once and persists Sent. This does not
verify the user's live Studio installation or real-network delivery.


## Scheduler persistence failure,20 November

Daemon now exits nonzero on any queue.save failure, before dispatch proceeds or control HTTP success is sent. Durable Sending recovers as Uncertain, no retry. Subprocess regression verifies save failure cannot return to caller. Existing closed-client acceptance verifies direct queue.json and restart/no-duplicate. Runtime test execution pending focused harness.

## Favorite marker placement,20 November

Implement approved sketch5: paint-only hollow heart in avatar left gutter; existing network badges stay right. No independent click target or favorite toggle. Pure geometry regression checks compact/regular rows and no avatar overlap. Executed native tests/pixels pending CI.

## Restore saved voice and video transcripts,20 November

The private on-disk store already persisted transcripts. Bubble status only read hot memory, so after restart saved text could be searchable while the same bubble offered Transcribe again. Restore exact account/chat/message text asynchronously with bounded concurrent reads, versioned hit/miss cache, and deletion/drop checks. Use the archive reader for bubble text even with the speech engine off. Save/drop/reopen voice/video display and search regression failed against the old getter and passes after the fix. All focused worker tests pass in an isolated production-module harness; native test-target compilation passes. Full app/Mac runtime unverified. No new GUI or release.

## Encrypted derived transcripts,20 November

Production transcript readers/writers now use a local_transcripts table in each account's existing SQLCipher archive, with the archive's existing OS-keyring key and no plaintext fallback. Legacy text migrates by account in a transaction, commit with synchronous FULL, exact text readback, then plaintext-file unlink. Failed unlock, conflict, verification or purge prevents migration from silently counting as done. Existing backups/snapshots and storage-level deleted plaintext are not retroactively erased. OS-keyring access by apps already running as the user, admins and compromised logged-in sessions are not prevented by encryption at rest.

Account unlink/logout purges only that account, invalidates queued results and closes our DB handle before account removal. Chat deletion/clear/search/restart retain their worker paths. Global text byte cap stays32MiB across accounts. Tests: three real SQLCipher module tests pass for wrong-key fail-closed/unkeyed read refusal/no plaintext markers, voice+video restart/search/account scope/removal, and verified migration/purge/conflict/no-account preservation.20 production worker tests pass including account logout late-result suppression. Native test-target compile passes; native app test execution and Mac Keychain/migration remain unrun. Existing source fixture harness uses test keys, not a real OS keyring.

Review follow-up: fresh Beeper profiles may have only beeper-drafts/drafts.db, not archive.db. Transcript storage now safely initializes the encrypted archive.db only inside an existing non-symlink account directory, retaining removal refusal. Quota and insert now share one account transaction, avoiding a committed save becoming failed/stale because another account cannot unlock. Tradeoff: quota is32MiB per account, not a global shared quota. This isolates accounts. Four SQLCipher tests pass including fresh Beeper directory selection/initialization with injected test key and an unavailable other account; actual OS-keyring constructor remains a Mac gate. Removal warning translated at UI boundary. Native test-target compile passes.
## Encrypted transcript store review fixes,21 November

Devin review of #420: account removal no longer purges transcripts before the backend's unconfirmed removal command (a confirmed logout still purges; a completed removal drops the account archive folder with its archive.db). Delete-all now attempts every account archive and surfaces the first failure in Settings instead of hiding partial deletions. Legacy plaintext migration moved off the settings thread onto the worker's store-serialized migration thread. Encrypted search matches the file store: an unscoped empty query returns nothing, and hits carry full transcript text without the display snapshot's clipping (the Voice panel's scoped empty query still lists transcripts). The account-deletion test now proves row removal by reopening the keyed fixture database instead of relying on a failed real-keyring reopen. Over-quota eviction stays silent by design, matching the plaintext store's cap eviction. Follow-up: Delete-all also removes unmigrated legacy plaintext so a pending migration cannot resurrect cleared transcripts. Per-message, per-chat and account removals now delete pending legacy plaintext too, for the same reason.

## Empty migration no longer wakes views,21 November

macOS CI caught two worker tests asserting exact scan counts failing after legacy migration moved onto a worker thread: `migrate_legacy_on_thread` woke the view unconditionally on success, so an empty archive_with_wake store still produced a third wake and raced the stale-completion test's wake counting. `Encrypted::migrate` and `Store::migrate_legacy` now return the number of transcripts moved; the migration thread wakes the view only when something actually migrated, and `Store::migrate_legacy` bumps the cache generation only then. The two exact-count tests (already in the tree) cover the regression: they failed before this fix on macOS.

## Per-account legacy migration isolation,21 November

Reviewer follow-up on #420: `Encrypted::migrate` returned on the first failing legacy account directory (missing archive folder or conflicting archive row), so every later account's plaintext stayed on disk and the migration failed identically on each launch. Migration now runs per account: a failing account keeps its plaintext for the next attempt and its first error is reported, while later healthy accounts still migrate and purge. `Encrypted::migrate`/`Store::migrate_legacy` return the migrated count plus the first per-account error; the worker wakes the view when anything moved and logs an incomplete migration. Regression: a bad account sorted before a healthy one no longer blocks the healthy account, and the conflict case now proves the archive row survives.

## Partial-migration view coverage,21 November

Devin follow-up on #420: the per-account migration regression verified the archive and the source files but not the view invalidation path. New regression drives a partial migration through the worker's migration thread: with one failing account and one healthy account the view wake still fires and the store version bumps, so cached results are invalidated and migrated text becomes searchable; the failing account's plaintext stays. Negative control: a retry with nothing left to migrate reports the failing account but does not bump the version again.

## Clear partial-failure coverage,21 November

Adversarial pass over the #420 diff (state transitions across files: select/edit/clear/retry/partial failure/ordering/restore). Held up: queued transcriptions die only on their own generation mismatch, so remove_account's repaint signal cannot drop other accounts' jobs; migration and clear/remove serialize on store_io in either order with final-correct results; clear_legacy still purges pending plaintext when an archive removal fails. New coverage: clear() reports a failing account but still clears the healthy account, purges pending legacy plaintext, and bumps the store version so views invalidate.

## Deterministic clear order,21 November

Devin follow-up on #420: the clear partial-failure test depended on filesystem directory order - a healthy account sorted first would pass even if the account loop stopped at the first error. Store::clear now attempts accounts in sorted order, and the test covers both orders: failing account first proves the loop continues past the error (healthy account still cleared, legacy plaintext still purged, version still bumped); failing account last proves the error still surfaces after successes.

## 2026-11-21: Free-only security pilot

Review: Instinct, branch `agent/security-pilot-free`, isolated security-pilot worktree. Adds report-only, pinned scanners and weekly staggered public-hosted runs. See `docs/security-pilot.md` for coverage, output and limits. No app behavior/version change, fixes, releases, private CI, or merges.

Security pilot review follow-up: include merge-resolution patches, tolerate null RustSec advisory warnings, fix native Go scan tags, and conform to repository lint. Added two regressions (both failed against the old implementation). CodeQL pin resolves in hosted CI; no change based solely on an external 404 claim.


## 2026-11-22: Preserve history jump stop reasons

Branch `fix/search-history-stop-reason`, based on main 91f596a. Owner screenshot
showed a partial-timeline warning, without logs. No private conversation content
was used in fixtures or copied here. The 400-page budget belongs to Solmu's
search/quote jump, not a WhatsApp/bridge quota. The old loop replaced every
unsuccessful exit with one generic possible-budget warning, including exhausted
history, unavailable/nonadvancing/repeating cursors and HTTP failures. Preserve
one real terminal cause, mentioning the local cap only after 400 pages. Keep
loaded pages and retry cursor, no budget increase or new network behavior.
The screenshot cannot reveal which underlying stop actually occurred, and this
patch does not claim to restore history the upstream service does not return.

Added synthetic HTTP fixtures for error causes, missing cursor/exhaustion,
cycles, the real budget and retained pages. An isolated Rust harness extracted
the exact production loop with stubbed transport; three meaningful test groups
failed before the change and pass after it. This is not the native app suite.
Native Rust 1.98 compilation reached libspa-sys and failed because the local
libpipewire-0.3 development package is absent. Installing prerequisites needs
unavailable sudo access. Native suite, Clippy and docs are not claimed passed.
Rustfmt and diff check pass. Public CI must run the complete native checks.
Version bumped to26.11.4. No UI layout change, real-account reproduction, release
or merge. Olli's explicit word is required before merging this PR.

## 2026-11-22: Bound large-message layout work

Branch fix/long-message-layout from main91f596a, independent of diagnostics PR442.
A reported startup/constant beachball was tied to a newly pasted ~377KB message;
the owner reports deleting it restored responsiveness. Current and yesterday's
manual69 sources share the same quadratic balancer. Manual70 only adds441, not
a new text shaping path. No commit regression is proven. The supplied macOS
sample shows sustained main-thread work but stripped frames prevent naming its
Rust function. No real message or sample is included here.

A synthetic420KB multiline log took~3.1s for layout in an isolated optimized
Linux harness using current production markup/wrap and pinned egui, with
unrelated emoji/theme/bidi leaves stubbed. The deterministic failing-first
assertion verified the expensive path was taken. The fixed same layout took
~81ms. A 504KB single paragraph exceeded a40s bound before paragraph partitioning;
afterward~68ms. A400KB single word laid out in~45ms. These are isolated numbers,
not macOS acceptance or a complete native app benchmark.

Above8KiB, use cached greedy layout for painting AND natural-width planning.
Only for these large bodies, partition shaping paragraphs every1024graphemes
in the display job using existing inserted-break/copy/link remapping; full
stored text, copy text, markup and actions remain intact. Borrow inline URL
parser suffixes rather than allocating the remainder at every word. Normal
messages keep balanced wrapping. Regression groups cover complete text/copy,
links, cache reuse, Unicode and long paragraphs. Two source regression groups
pass in the isolated harness. Native suite/Clippy/docs still require public CI
because this environment lacks PipeWire development files and sudo access.
Version26.11.5 avoids colliding with26.11.4 on the independent diagnostics PR.
No merge without explicit owner word.


2026-11-22 18:20: Rebased443 after442 merged at4f9cfe5 under the owner's
explicit merge-order instruction. Kept sequential26.11.5 and preserved26.11.4
release notes without duplication. Previous full native CI failed only the new
cache pointer assertion: parallel font reinstalls invalidate cached bodies by
global generation. Default Mac native tests passed; all-features Mac/Ubuntu/Nix
reached the pointer assertion. Match existing cache tests by checking reuse only
in an unchanged epoch, with bounded retries and a failure if none stays stable.
No production test expectation or performance path is weakened. Added an exact
1024-grapheme boundary/combining-mark/hard-newline regression. Fresh CI required.
