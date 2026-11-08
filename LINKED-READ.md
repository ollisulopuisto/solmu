# Restored linked-source read state

The state is transient, per account and source conversation. Each restoration
has the exact origin/account/chat identity and a fresh opaque request token.
Cached messages and ordinary paging state do not authorize a read.

| Current state | Input | Next state and effect |
| --- | --- | --- |
| Absent or any cached state | New exact linked source arrives unread | Restored-pending with new token; keep badge, no receipt |
| Restored-pending | Linked view requests history | History-requested; send one tagged initial fetch |
| History-requested | Matching successful initial fetch completion | Loaded-for-this-restoration; no receipt yet |
| History-requested | Matching failed fetch completion | Failed; preserve badge; no automatic retry |
| Any state | Generic Messages, live page, old token, duplicate completion | No readiness transition |
| Pending/requested/loaded/failed | Reader chooses Retry history | Restored-pending with a new token, then fresh tagged fetch |
| Loaded-for-this-restoration | Source bubble drawn in viewport and all current gates pass | Read-sent; clear badge and send read command once |
| Loaded-for-this-restoration | Offscreen/filtered, closed, unfocused, hidden, incognito, app/source lock, invalid linked scope | Keep loaded; no receipt |
| Any tracked state | Source removed or exact identity changes in inventory/update | Absent; old completions cannot authorize anything |
| Any pending/failed state | User explicitly chooses Mark as read | Read-sent; clear pending state and issue read command without requiring history |
| Read-sent | Old completion/repaint | Remain read-sent; no duplicate deferred command |
| Read-sent | Later removal and restoration | New pending generation, never cached success |

Automatic open/focus reads cannot bypass pending/failed restoration state.
Explicit Mark as read intentionally can. Receipt preference and backend read-only
mode keep their existing semantics. An empty failed-history placeholder never
counts as success. A successful empty history alone does not count as seeing an
unread message: without a displayed source bubble, automatic read stays pending.

## Verification map

- linked_read transition-table tests: all phases, success/failure, explicit read,
  retry token rollover, cached restoration, stale/duplicate completions,
  inventory removal and identity replacement.
- App tests: focus/privacy/lock/open-chat/identity matrix; generic failure
  placeholder; fresh load despite cached success; explicit read after failure;
  loaded source excluded from displayed sources; inventory removal.
- Beeper worker test: tagged success and failure completions arrive after their
  message payload/placeholder, never from an unrelated live update.
- Existing linked unread/Instagram tests remain in place.

## Review outcome (November 8)

Adversarial checks covered inventory-only return, removed and replaced identities,
duplicate IDs, stale completion after removal, retry token rollover, offscreen or
filtered sources, post-load history errors, private/unfocused/locked views and
explicit read after failure. Partial page errors do not signal successful load.
The worker test exercises HTTP 500 as well as successful and invalid responses;
an ordinary initial fetch emits no restoration completion.

Local results: three pure state tests, 15 focused native app tests and 231
portable adapter tests passed. Strict all-feature/all-target clippy passed. The
focused native build kept production code unchanged and omitted unrelated test
modules. The complete native test build exceeded local resources; full CI is
still required. No network account was used for these regression tests.

## Unlink and independent paging outcomes

| Current state | Input | Next state and effect |
| --- | --- | --- |
| Any tracked state | Link removed, exact identity no longer matches, or source no longer belongs to valid linked scope | Absent; retire token and reset requested so individual history can fetch |
| Failed | Older/live history succeeds and clears generic paging error | Remain Failed; restoration Retry history remains visible |
| Failed | Retry history | Fresh pending/requested token; only its tagged initial completion can load it |
| Absent after unlink | Old tagged completion | Stay Absent; cannot recreate readiness |

Both Unlink and Remove local link use one invalidation method. Fetch, automatic
read, tagged completion and displayed-source paths also reconcile scope, covering
settings changes outside those two controls. Individual reopening uses its
ordinary fresh fetch/read behavior rather than inheriting a linked-read guard.

Generic-error audit: the only production reads/writes are in app.rs and the
linked retry UI in ui/merged.rs. BeeperHistoryState sets/clears ordinary paging
errors; new restoration, Retry history and scope invalidation clear them.
ensure_person_loaded consults them to avoid repeated failed requests, and the
visible-source finish path blocks while an error exists. None of those generic
error clears changes Failed to Loaded. The UI now uses person_history_error:
ordinary errors take priority, with a separate Failed-state fallback. Older
LoadChat bypasses ensure_loaded intentionally and cannot complete restoration.
Reconnection and inventory refresh fetch the open source through ensure_loaded.
Open/refocus automatic reads reconcile scope and retain valid pending guards.

Verification adds all four pending phases through unlink and two individual
reopens, stale completion after unlink, and paging success while Failed through
retry/new token/loaded/read. The latter runs the actual linked UI and checks its
painted Retry history button after the generic error disappears. Local focused
native app tests: 17 passed. Pure state tests exercise scope retention/removal
from all six states. A synthetic runtime capture was inspected: the fallback
warning and Retry history button are visible after the paging error is cleared.
No review media was committed.

## Localization review

The restoration fallback, partial-timeline message template and Retry history
button now all use the active-locale gettext path. Template placeholders keep
source/error values separate from translated prose. The PR production diff was
swept for other new user-facing literals; remaining English additions are test
fixtures/assertions or internal invariant messages. Existing broader linked-view
copy outside this PR is unchanged. No speculative catalog translations were
added: missing catalog entries retain the existing English fallback.

The fallback test covers every offered locale. All 18 focused native app tests,
strict all-feature/all-target clippy, format and diff checks pass. Refreshed
synthetic capture inspected: partial-history notice and Retry history remain
readable. Full current-head CI remains the final gate.
