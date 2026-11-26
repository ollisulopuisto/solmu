# Ordinary composer text persistence

Unsent composer text is stored locally in the encrypted draft database. Draft
writes work while the Beeper server is initially disconnected. Normal typing
queues a save after 650ms without an edit, or at most two seconds of continuous
typing. A blocked backend network request can delay processing the queued save
by its existing30second timeout; local disk/keyring availability and an in-flight
save can also delay it. Chat
and account switches save immediately; normal quit queues a final save before
joining the worker. Draft database commits use synchronous FULL in WAL mode.

Abrupt termination can still lose the newest unsaved interval. No application
can promise persistence if storage/keyring is unavailable or a write fails;
errors stay visible, with no plaintext fallback. This does not change the
separate cleared-text recovery offer, its lifetime or its persistence.
