# Work log

## 2026-11-11 - Beeper view-once composer visibility

Approved scope: hide the misleading photo View once toggle on Beeper chats. No send/upload changes and no merge order.

- Composer visibility now checks Beeper origin/account identity before showing the existing photo-only control, including restored selected drafts.
- No draft state is cleared or downgraded. The existing backend refusal is unchanged.
- Red-green: compiled the unchanged production visibility leaf as a standalone Rust test module. The Beeper test failed with the old predicate and both tests passed after the guard. This does not claim full native test-suite execution.
- Native demo build passed on Linux. Full native test compilation exceeds this environment's 2 GB memory; final CI and macOS/live-network interaction remain unverified.
