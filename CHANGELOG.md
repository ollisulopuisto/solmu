# Changelog

Newest first. One `## <version>` section per release, bullets under it;
the app shows the sections newer than the version the user last
acknowledged, and the latest sections when opened from Help. The
`## Unreleased` section collects notes for the next release and is never
shown automatically; move its bullets under the release version when the
version in Cargo.toml is bumped.

## Unreleased

- Follow-up text bubbles use each sender/source's recent cadence instead of a fixed calendar-minute boundary. The last ten minutes of completed gaps set a 1.5x average allowance, bounded to 2-5 minutes. A long pause starts a new burst; replies, metadata and other existing boundaries still stay separate.

- Keep macOS package-signing and notarization passwords out of process arguments and error output during release builds.

## v0.24.7

- Message widths snap to 80-point steps: short text uses 80/160/240-point content widths, cards start at 320 points, and longer text/cards share the same larger steps within the window.

- Cmd-R (Ctrl-R outside macOS) in a linked chat quotes the latest loaded incoming message, selects its exact bridge/account/chat and focuses the composer. Newer outgoing messages and search filters do not change the target; nothing is sent automatically.

- Text streaks overlap with the older bubble in front and keep a triangular speech tail on every bubble. Group speaker changes use contrasting colored drop shadows, including their tails, while names and avatars remain visible.
- Reactions and message time share a compact row that wraps when needed without covering one another.

- Keep outgoing linked-chat network icons outside message bubbles.

- Opening or marking a linked person read clears the exact unlocked source chats shown together, not only the representative chat. Live updates and returning focus use the same scope; incognito and hidden-account protections remain.

- Save all exports are owner-only on Unix, rather than inheriting a world-readable default mode.
- Save all keeps attachments with colliding sanitized filenames separate and checks the batch before writing.

## v0.24.6

- Message clocks sit below the final bubble of each same-sender/source minute, aligned with that sender's edge. The wide right-hand clock gutter is removed.
- Unknown delivery status no longer paints a standalone dash in the transcript; hover still explains that the status is unavailable. Real pending, sent, delivered/read and failed states remain visible. Failed text can be restored to the composer for review by clicking its failure label; nothing is sent automatically.


## v0.24.5

- Day changes have more room above and below their labels in ordinary and linked timelines.
- Message clocks align in a fixed right-edge gutter instead of following each bubble's width. Consecutive same-sender, same-source messages in one minute show the clock only beside the last bubble. Status ticks, edited/failure notices, pin/star marks and exact-time hovers remain available.

## v0.24.4

- Chat header actions now use outlined icon cells beside Search, with green active states and tooltips. Linked timelines use the same single header row, including at narrow widths.
- Background live-update errors stay discoverable in a compact Connection status control instead of painting raw diagnostics across messages. Details explain that messages may be missing while updates retry.
- Favorite avatars include unread state from other unlocked chats linked to the same person. Network badges use a single muted color so unread counts and the active chat stay easy to see. Muted unread badges keep the attention color, only slightly subdued instead of grey.

## v0.24.3

- Conversation clocks and status marks sit outside the bubble toward the centre. Date separators carry the day; hovering the clock keeps the full local timestamp. Photos and albums no longer draw a timestamp over the image.
- Search results now request missing Beeper history instead of silently dropping the jump command. The target stays pending until reached; history failures and a 50-page safety bound are explicit and retryable.
- Global and in-chat Beeper searches offer Load more when the API reports another page. Each click fetches one next cursor page, keeps earlier pages and is capped at 50 pages with a notice, preserving query/day/chat scope.
- Cmd/Ctrl+K keeps recent chats with an empty query. Typing lists matching chat names first, then message text from the existing index plus fuzzy matching of loaded text. Remote Beeper search is literal-word search, so typo matches in unloaded history are not guaranteed.
- The open-chat toolbar duplicates Read later, Reply later, Favorite and Archive actions, including clear/unfavorite/unarchive states. Later flags remain local and use the same existing shortcuts.

- Search shortcuts now have fixed scopes on macOS and elsewhere: Cmd/Ctrl+F searches within the open chat only, Cmd/Ctrl+Shift+F searches all messages and chats, and Cmd/Ctrl+K stays recent chats. macOS Edit menus now match the keyboard instead of routing Cmd+F to the global search. Settings search is reached by clicking its field, not by changing Cmd+F's meaning.

## v0.24.1

- macOS: Cmd-Tab now restores minimized Solmu windows by default after real-device testing of the opt-in. Set `SOLMU_EXPERIMENTAL_RESTORE_MINIMIZED=0` when launching to opt out. Closed-window activation and Dock reopen keep their existing paths.

- Conversation day separators now show Today, Yesterday and Day before yesterday, then short numeric dates in the system locale's order. Dates from another year include the full year. Finnish regions show Tänään, Eilen and Toissapäivänä. Bubble clocks and chat-row timestamps are unchanged.
- The four-way queue toggle can show chat counts for All, Unread, Reply later and Read later. Settings > Chats > Show queue counts is on by default; turn it off for icons only. Counts follow the current search and filters, count linked people once, and are separate from unread-message counts.

- macOS: explicitly subscribe the late-installed Cmd-Tab activation handler with standalone method type encodings so it reliably requests a new window when running headless, and provide direct minimized-window restoration (now on by default; set `SOLMU_EXPERIMENTAL_RESTORE_MINIMIZED=0` to opt out). Dock reopen stays separate.
- Favorites ordering avoids normalizing ordinary ASCII search text just to test emptiness, while preserving Unicode search behavior.

- Sidebar shortcuts follow the visible queue: Cmd+Option+3 (Ctrl+Alt+3 elsewhere) selects Reply later, and 5 toggles Favorites only. Network tooltips now show Option/Alt without Command/Ctrl. Fixed the off-Mac Ctrl+Shift+1 Chats collision and corrected shortcut help and guide references.

- The active avatar in the collapsed sidebar and favorite bar has a clear ring around its circle instead of a thin line at the left edge. Unread and service badges remain on top.

- Chat-list redesign: Search first, then Chats beside an aligned four-way icon queue (All, Unread, Reply later, Read later). Filters combines chat type, favorites, labels, Unreplied and network/account scope; the bottom New message button stays fixed even with no results. Search keeps the selected scope and offers a count of available matches outside it, without listing them until the scope is cleared. Keyboard filter/network/favorite digits keep their assignments.
- Incoming group messages use a fixed avatar gutter and a straight bubble-body column, including follow-up messages from the same sender.

- Clean URL labels on shared links (including Instagram reels) now hide the duplicate parenthesized destination, like named links. Clicking or Copy URL keeps the complete destination and its query parameters; message copy/export keeps the original text.

- Copy image and Save as now work on downloaded attachments in Beeper mode, even while disconnected. Failed decode, clipboard writes and file saves show an error instead of silently doing nothing.

- Message timestamps, day separators and date labels follow the OS regional locale independently of the app language: a Finnish region reads "eilen 20:13", "toissa päivänä 20:13" and "14.11.2023", a US region "Yesterday 10:13 PM" and "11/14/2023". The 12/24-hour clock keeps following the OS clock setting.
- The Dock/taskbar unread badge and the Unread filter now count the same chats: channels and chats from Beeper accounts hidden from the inbox no longer leave a ghost number on the icon. Muted chats still appear under Unread without raising the badge.
- Settings: clearer spacing between sections, consistent row rhythm, and each Beeper account's technical details (account ID, backend, bridge status) grouped under an Advanced heading inside the account panel instead of crowding its title.
- Cmd+D toggles the right-hand info sidebar. Cmd+B still toggles the chat list; Cmd+I still marks Reply later.
- Links whose separators arrive entity-escaped from Beeper Server (Dropbox share links among them) now render and open with a plain `&` instead of a literal `&amp;`. The repair is scoped to link addresses - one level - and text outside links keeps its entities byte-for-byte. Diagnosed from a live payload probe.
- Named links show their title instead of a long raw URL; hover reveals the full address. The message menu now offers "Copy URL to clipboard" for named and standalone links, preserving the full target including query parameters.
- The chat list has more breathing room: default rows are 80 points tall instead of 68, with the same avatars and two-line previews. Settings > Beeper view > Compact chat list keeps the smaller 56-point rows.

## v0.22.0

- Link a person's chats from Contact info: a searchable picker suggests name matches across networks, and the linked conversation shows who sent each message and on which network.
- The left rail is a single compact icon column: favorites and pinned chats are avatar chips with unread badges, filters are one icon row, and networks sit as plates at the bottom with right-click account scoping.
- Voice notes can be transcribed entirely on your own local speech services (no cloud), with a per-message transcription language override from the message menu.
- The conversation background picks up a light tint of the open chat's network color, and each Beeper account can have its own background and conversation theme, linked view included.
- Pinned chats keep their normal row in the chat list as well as their rail shortcut.
- Shortcuts: Command+1..9 opens the nth favorite, Option+1..n picks a filter.
- Quote cards show the original message's excerpt when it is already loaded, and Signal group mentions render as proper chips with real member names.
- Hide and mute indefinitely now holds locally and tells you when the bridge does not confirm the change.
- Send Later: adding a message no longer kills the scheduler, setup failures name the failing stage, and a resend that could duplicate a message asks for explicit confirmation.
- Clicking the Dock icon brings the window forward again on the packaged app.
- Connection warnings in a chat collapse to one line per distinct problem, with the full text on hover.
- Networking update: connections no longer hang on networks with broken IPv6, plus a TLS verification fix (ureq 3.5).

## v0.21.0

- Command palette (Ctrl+P): find chats, apply list filters and run common actions from one keyboard-first overlay.
- Combined sidebar: network and account filters moved into the filter chip row, pinned chats show as regular rows at the top of the list, and Calls stays reachable in the rail with Beeper accounts.
- The chat list's edge can be dragged to resize it.
- Attachment uploads to WhatsApp no longer send the request shape that was rejected; a failed send keeps the staged files.
- The sidebar restores correctly in narrow windows and with the list hidden, and settings rows no longer cut their titles short.
