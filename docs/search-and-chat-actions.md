# Search and open-chat actions

Cmd/Ctrl+F always searches inside the current chat, independent of keyboard
focus. No chat view means no action. Cmd/Ctrl+Shift+F searches all messages and
chats. Both scopes are explicit in macOS Edit and shortcut help. Settings search
is reached by clicking its field.

Cmd/Ctrl+K shows recent chats with no query. With a query, chat names come first,
including never-opened chats, followed by message text. Message candidates reuse
the normal backend search and currently loaded conversations, not a second stored
index. Matching loaded text accepts word subsequences of at least three characters;
exact matches rank first. The remote Beeper index does literal-word search, so
fuzzy/typo matches outside loaded history are not guaranteed. Locked chats remain
excluded. Selecting a message uses the same exact chat/message jump as search.

Global and in-chat Beeper search initially request one page. Load more re-reads a
cumulative window one page larger, preserving the exact query/chat/day parameters,
deduplicating chat+message IDs. Re-reading costs additional local API requests and
latency, not a new paid service. This is capped at 50 pages per query. Missing,
repeated or failed continuation cursors are errors, not proof the result set is
complete. Previous displayed results stay available on failure.

Clicking a message hit from either search requests older history until the target
is loaded, using the last good history cursor. Up to 50 older pages are loaded per
click; a failure or bound is shown, and clicking again retries. A pending jump is
not replaced by the latest page's scroll-to-bottom. The target is highlighted
when laid out. This does not repair separate Beeper live-update cursor errors.

The toolbar above an ordinary open chat duplicates Read later (Cmd/Ctrl+U), Reply
later (Cmd/Ctrl+I), Favorite and Archive. Active states offer the reverse action.
These use existing chat IDs/actions even when the chat was opened from search.
Later flags are local; Archive/Unarchive keeps its existing backend semantics.
Channels do not get Favorite. No messages are sent by these controls.
