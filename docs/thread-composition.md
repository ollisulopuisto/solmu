# Sending in a thread

While a loaded reply thread is open, a new send without an explicit reply choice
replies to its latest loaded visible member. This keeps the new message in that
thread. It applies to text, location, attachments, GIFs, stickers and new voice
recordings. An explicit reply target wins; returned attachment/voice retries
keep their original target rather than being silently retargeted. Edits and
scheduled drafts keep their existing behavior.

There is intentionally no unquoted-send mode inside the thread. Clearing an
explicit reply restores the thread default; it does not opt out. To send without
a quote, close the thread first. No old message links are changed. The default
is resolved when sending from currently loaded messages, not unseen history.

The thread view has no connecting lines, including replies to non-root members.
Normal chat chain glyphs, resting stubs and hover/focus routes are unchanged.
Thread membership, order, default reply targeting and scrolling are unchanged.
