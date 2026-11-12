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
