# Local flat groups

Latest and Groups are two views of the same chats. Cmd-D switches views, using
shared-keymap PR #281 (which moves chat info to Cmd-Option-I).

Create a group in Groups. Right-click a chat and choose Assign groups, select
any number of groups, then Apply. Cancel changes nothing. Drag a chat onto a
group header to add it without changing its other memberships. The Undo button
removes only that added membership. Right-click a group header to delete the
group. This deletes no chat, history or remote data.

Membership is saved in local settings, keyed by local account, backend origin,
bridge account and chat. It is not a Beeper label, folder or remote group.
There are no parents or subgroups. Aliases open the original chat, so read state
is shared. The total unread-chat count counts each chat once even when it appears
in several groups. Existing search, queue, network and account filters still
apply; archived and locked chats stay behind their existing views.

The old label commands are not used: the Beeper adapter does not implement them.
This feature shares the local-only label concept, but uses the existing settings
save path so bridge refreshes do not erase memberships. No migration from remote
labels is implied. At most 100 groups are kept, with names up to 80 characters.

Validation: standalone membership tests, native cargo check and library clippy.
Native visual capture, full suite and Mac keyboard/drag acceptance are pending.
