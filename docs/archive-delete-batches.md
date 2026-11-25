# Archive chats and scoped message deletion

Archive is separate from deletion. Cmd-click chats (or use Select for archive
in a chat's context menu), then Archive selected. Review the count and any
unsupported chats before confirming. Messages stay in Archived. Unsupported
items block the complete batch; Solmu does not quietly skip them or change scope.

Message multiselect uses the existing selection UI. Delete opens a scope review.
Only scopes advertised by the selected chat's bridge are shown. A scope change
requires another explicit choice; it is not a fallback. Delete for everyone
shows the bridge age limit when supplied and warns that it cannot be undone.
Unknown capabilities fail closed. Partial global-delete capability is also
withheld until its constraints can be represented reliably. Outgoing messages
only are eligible for global deletion here; no moderation override is inferred.

The worker refreshes the selected chat's capabilities and the message before
deletion. Missing, deleted, hidden, nonfinal and mismatched messages are refused.
Archive is confirmed by rereading the chat's state. Delete success means the
endpoint accepted the request, not independent proof of remote-device removal.
The list header shows per-item accepted, failed/uncertain or pending outcomes.
Batch request IDs prevent old replies or unrelated single-item actions from
being shown as this batch's outcome. No destructive action is automatically
retried. For an uncertain result, inspect the real chat before any manual retry.

Sources for capability semantics:
- https://raw.githubusercontent.com/beeper/desktop-api-python/main/src/beeper_desktop_api/types/shared/chat_capabilities.py
- https://developers.beeper.com/desktop-api-reference/resources/messages/methods/delete/index.md

Read: archive, deleteForMe, delete (only full level 2 enabled), deleteMaxAge.
Missing is not permission. There is no fixed network allowlist and no local
fallback. The existing read-only HTTP guard remains in force.

Validation and limits: native library check/clippy and isolated capability model
test pass. Full native/adapter tests, live bridge behavior, pixels and Mac input
acceptance remain pending. Changes overlap other Beeper work only at capability
mapping and destructive handlers, not uploads/sends. Rebase after that work lands.
