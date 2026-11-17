# Chat list modes and local groups

All is the full list, latest activity first. Unread lists every chat with
something unread, muted chats included. Needs me includes unread chats and the
Deal with later mark, excluding every muted chat. Cmd+D cycles these three.
Bookmarks is not a mode: the bookmark icon in the left rail, Go > Bookmarks in
the Mac menu bar, or Cmd+Shift+B opens the Saved list, with a Starred and Saved
switch under its header. Existing read/reply-later marks count as that same
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

The pre-existing Filters row is removed. It previously exposed chat type,
favorites, Unreplied, auto-category, network/account, labels and link suggestions.
Shared filter actions remain for palette/keyboard and legacy views. Starting the
app or picking a primary mode does not restore hidden saved type/favorite scope;
picking a mode clears all extra scope. These controls are no longer in the list.

## Layout and kept chats

Each mode shows the group sections or one flat list (Cmd+G switches the current
mode; the choice is kept per mode in the account settings, and no entry means
sections). A flat list has no headings (no Pinned or Other chats labels either; Archived keeps its own): each row of a grouped chat ends in
` · Group name`, and the keyboard steps through the list in its on-screen order.
Search results and Archived are unchanged.

A chat opened from the list, or stepped to with the keyboard, in Unread, Unread
7d or Needs me stays listed after it is read, in its place. Changing the view
(a mode, a filter, Archived) forgets it. Chats opened from a notification or a
search result are not kept.

Native macOS before/after captures are available in the
[sidebar review](https://files.instinct.com/1nsz2jzyeew9-solmu-sidebar-before-and-after)
and linked in PR #358. They show the old and new sidebar in light and dark,
using synthetic demo chats only. The final named modes, description line and
Groups + control were checked visually. Default/all-feature Mac tests and the
full Linux/Nix CI gate passed for bcefc08.


The chat-list modes are **Recent**, **Unread**, and **Needs me**. Recent contains
all chats in the current scope, newest activity first within each group (or
globally in flat layout), with no age limit. Unread includes muted chats; Needs
me includes unread or manually marked-for-later chats but excludes muted ones.
Old Unread 7d settings migrate to Recent. Cmd+D cycles these three modes.

The **+** beside the Groups heading opens **New local group**. Enter a name and
press Create or Enter; Cancel, Escape, or clicking outside dismisses it without
creating a group. The field is not part of the scrolling list and works in both
grouped and flat layouts. This creates a local sidebar section, never a new
backend group conversation.
