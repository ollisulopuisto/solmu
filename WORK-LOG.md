# Work log

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
