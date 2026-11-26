# Screenshot selection

Screenshots stay enabled. Startup and the four before/after About/settings captures always run. Only reviewed unaffected gallery scenes may be omitted. The eight-second capture delay is unchanged, and Rust tests are never filtered.

## Map and safety gates

`.github/scripts/scene-map.json` lists all 43 canonical gallery scenes (both workflow jobs load this one list) and classifies all 186 Rust files. `full` is an explicit safe classification, not a missing entry. Only four isolated runtime modules currently have reviewed scene subsets: reply-router (9), message-info (4), settings UI (2 plus the unconditional About/settings captures), and transcript text (4). Four test-only modules and four separate binary targets cannot be linked into the screenshot executable, so keep only baseline captures. Other source changes keep the full gallery. Known safe mixed changes take the union of their scene sets. Any full or unknown path wins.

The selection tests reject a new/deleted Rust module, an unknown scene reference, unknown scene references, missing review reasons, and an incomplete reachability contract inventory. A new file therefore cannot silently lose coverage. Non-Rust implementation, assets/fonts, localization, dependencies, workflow/selector/map, global state, input and shared layout changes retain full coverage.

The reachability proof is deliberately bounded to the reviewed source revision: SHA-256 contracts cover every full-coverage module and Cargo.toml. If any such context changes, including outside the current PR after a base update, selection becomes `full` until the map has been reviewed and refreshed. Contract drift is not a CI failure, because full coverage is safe. Completeness drift is a CI failure. Refresh hashes only after reviewing call sites, fixtures and parent cfg(test) declarations, never as an automatic repair. Integration after #487-#489 needs that review before claiming any screenshot savings.

An import graph alone cannot prove pixels are unaffected. The map follows dispatch and fixture state: MessageInfo renders only its dialog; settings renders only its page; transcript text has one production voice-transcript caller; router draws only reply connector geometry. Shared info-panel code was considered and rejected because docking and thumbnail cleanup run outside the panel and affect shared conversation layout. Shared hover/input and reply-tail geometry remain full.

## Historical applicability, 26 November 2026

Replayed the file lists of the 40 most recently merged PRs among the 200 most recently updated closed PRs, sorted by merge time. Sample spans 2026-11-21T11:17:11Z to 2026-11-26T07:39:08Z. File lists came from the GitHub PR files API; current map applied to historical filenames, not reconstructed historical runtime/call graphs. This is a counterfactual applicability check, not a timing benchmark.

**0/40 PRs would newly skip gallery captures; 0/39 native PRs.** One documentation-only PR already omitted native work under the previous selector and is not a new saving. Most recent fixes combine demo fixtures, app state, conversation or shared widget changes. Those correctly remain full. No claimed historical speedup from scene selection. Targeted future PRs may benefit; pure router saves 34 gallery launches, nominal 272 seconds of fixed waits, settings-only saves 41, nominal 328 seconds. Startup/About and process overhead remain. Actual timing is pending, and the workflow PR itself runs full coverage.

## Sample sources

- #486 https://github.com/ollisulopuisto/solmu/pull/486 | already no native
- #460 https://github.com/ollisulopuisto/solmu/pull/460 | native/full
- #485 https://github.com/ollisulopuisto/solmu/pull/485 | native/full
- #482 https://github.com/ollisulopuisto/solmu/pull/482 | native/full
- #483 https://github.com/ollisulopuisto/solmu/pull/483 | native/full
- #484 https://github.com/ollisulopuisto/solmu/pull/484 | native/full
- #479 https://github.com/ollisulopuisto/solmu/pull/479 | native/full
- #456 https://github.com/ollisulopuisto/solmu/pull/456 | native/full
- #464 https://github.com/ollisulopuisto/solmu/pull/464 | native/full
- #481 https://github.com/ollisulopuisto/solmu/pull/481 | native/full
- #477 https://github.com/ollisulopuisto/solmu/pull/477 | native/full
- #478 https://github.com/ollisulopuisto/solmu/pull/478 | native/full
- #469 https://github.com/ollisulopuisto/solmu/pull/469 | native/full
- #468 https://github.com/ollisulopuisto/solmu/pull/468 | native/full
- #467 https://github.com/ollisulopuisto/solmu/pull/467 | native/full
- #466 https://github.com/ollisulopuisto/solmu/pull/466 | native/full
- #462 https://github.com/ollisulopuisto/solmu/pull/462 | native/full
- #465 https://github.com/ollisulopuisto/solmu/pull/465 | native/full
- #459 https://github.com/ollisulopuisto/solmu/pull/459 | native/full
- #461 https://github.com/ollisulopuisto/solmu/pull/461 | native/full
- #457 https://github.com/ollisulopuisto/solmu/pull/457 | native/full
- #458 https://github.com/ollisulopuisto/solmu/pull/458 | native/full
- #452 https://github.com/ollisulopuisto/solmu/pull/452 | native/full
- #455 https://github.com/ollisulopuisto/solmu/pull/455 | native/full
- #449 https://github.com/ollisulopuisto/solmu/pull/449 | native/full
- #454 https://github.com/ollisulopuisto/solmu/pull/454 | native/full
- #453 https://github.com/ollisulopuisto/solmu/pull/453 | native/full
- #451 https://github.com/ollisulopuisto/solmu/pull/451 | native/full
- #450 https://github.com/ollisulopuisto/solmu/pull/450 | native/full
- #448 https://github.com/ollisulopuisto/solmu/pull/448 | native/full
- #447 https://github.com/ollisulopuisto/solmu/pull/447 | native/full
- #445 https://github.com/ollisulopuisto/solmu/pull/445 | native/full
- #446 https://github.com/ollisulopuisto/solmu/pull/446 | native/full
- #444 https://github.com/ollisulopuisto/solmu/pull/444 | native/full
- #443 https://github.com/ollisulopuisto/solmu/pull/443 | native/full
- #442 https://github.com/ollisulopuisto/solmu/pull/442 | native/full
- #441 https://github.com/ollisulopuisto/solmu/pull/441 | native/full
- #439 https://github.com/ollisulopuisto/solmu/pull/439 | native/full
- #440 https://github.com/ollisulopuisto/solmu/pull/440 | native/full
- #427 https://github.com/ollisulopuisto/solmu/pull/427 | native/full
