# Chat list modes and local groups

Recent is the full list, latest activity first. Focus includes favorites, pins
and Needs me. Needs me includes unread chats and the Deal with later mark,
excluding every muted chat. Existing read/reply-later marks count as that same
mark, and clearing Deal with later clears both old marks.

Every mode uses static section headers. Headers do not collapse or filter the
list. Other is always last. Each chat has one home. Old settings with several
memberships display the first saved group only until reassigned. Assign groups
uses a single choice; dragging to a header moves the chat and all its linked
source identities. Deleting a group deletes no chat or history.

Membership stays in local settings and is keyed by local account, backend
origin, bridge account and chat. It does not change remote labels or folders.
At most 100 groups are kept, with names up to 80 characters.

Mute offers next 08:00 local time or forever. Beeper's bridge supports only a
boolean mute, so timed mute is explicitly local to Solmu. It clears an old
remote forever mute and persists the local deadline over bridge refreshes.
Expiry restores unread chats to Needs me without another user action.

Cmd-F searches the open chat. A second press widens to all chats, carries the
query, leaves the global scope banner and dims the conversation. Cmd-Option-F
and Cmd-Shift-F open global search directly. Cmd-K searches chat names only.
The old Cmd-D Latest/Groups toggle is retired.

Validation: cargo check --lib and --tests pass. Isolated production probes
cover one-home assignment (old implementation fails) and local 08:00 across
midnight and daylight-saving change. Full native tests and screenshots remain
pending because test executable code generation exceeds the local execution
limit. macOS keyboard/drag and live bridge behavior need CI/runtime validation.
