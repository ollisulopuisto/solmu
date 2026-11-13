# Work log

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
