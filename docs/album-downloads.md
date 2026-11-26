# Album downloads and saving

Album rows no longer show a dedicated Save all button; bulk export is in the album context menu. Downloading into the local
cache and exporting a file are different actions. Per-item Save remains in its
menu, and Download all remains for missing/failed/over-limit/opt-out files.

Incoming images in loaded events are prefetched across chats, including albums.
This honors the account automatic-download setting and excludes locked chats.
Automatic files have a64MiB limit and streaming/cache bounds, with at most64
image downloads in flight across conversations. Queue-busy failures can be
replenished; other failures do not loop automatically. Outgoing images and
unloaded history are not promised prefetch. Other media keeps its existing
policy. Moving bulk export must not remove any manual fetch route. Save all has no existing keyboard binding, so no shortcut is invented.
