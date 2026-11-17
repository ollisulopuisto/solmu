# Shared items

The chat info panel separates Photos, Videos, Voice, Docs and Links. Voice includes audio files as well as voice notes. Download is explicit; after it finishes Play toggles playback, and Go to chat opens the originating message. The clip keeps its own chat speed even if a member info panel is open over a group.

Beeper metadata is read on a separate bounded worker, not on the typing/send worker. One Refresh or opening checks at most ten history pages and 20,000 rows, with a newest-items note if history continues. This is not a promise that every old attachment was found. Each kind has its own display cap. No attachment body is downloaded by listing metadata. Refresh is explicit, not polling. Failed or busy requests show Retry rather than leave a spinner forever. No database schema or saved-setting change.

Validation: model categorization/caps, backend pagination/bounds/scope/errors, audio download state without a loaded transcript, existing info-pane request correlation and demo scenes. Local library clippy passes; native suites and synthetic pixel review are required before merge. Live private messages are not sent, transcribed or downloaded during tests.

Native synthetic before/after, Voice, Videos, dark and narrow-panel captures:
https://files.instinct.com/6nkicc93b5c9-solmu-shared-voice-and-video

If a refresh fails, existing cached items remain usable and a visible error
with Try again is shown above them. A failed refresh never implies an empty chat.

## Saved voice transcripts

Voice has an Expand transcript/Hide transcript control for saved text and a
search field limited to the panel's account and chat. Search results include
a matching text line; audio without a saved transcript does not match.
Saved matches outside the bounded media listing still offer Expand and Go to chat, without invented audio metadata. Searching never downloads audio or queues transcription. The stored text
remains local and is searchable even when transcription is disabled. Closing or changing the panel clears its query and expansion.

In-chat search merges saved local transcript matches with backend matches,
labels the text Voice transcript, and deduplicates by chat/message ID.
Results jump to the original voice message. Day filters use a known message
date; an unloaded message whose date is unknown is excluded when filtering
by day, rather than using the transcript file's modification time. Hidden
locked chats and another account's transcripts are excluded.
