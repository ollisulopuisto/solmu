# Draft recovery controls

Normal automatic saving is silent. Clearing meaningful text offers only Save
deleted text as draft, Restore text and Dismiss. No explanatory label. The
first new nonblank text starts a10second countdown, shown as a thin shrinking
edge without numbers. Further typing does not reset it. At zero the offer and
its temporary snapshot disappear; nothing is automatically added to Drafts.
Normal same-editor Cmd-Z history is independent and may still restore text.

Attachments do not start, reset or dismiss recovery. Navigation pauses the
timer and keeps its remaining time in memory, resuming when that chat returns.
Quit/restart does not preserve this unsaved deleted-text snapshot. This does
not change persistence of ordinary text still in the composer.

Cmd-Shift-S on macOS, Ctrl-Shift-S elsewhere, saves the deleted snapshot while
the offer exists, otherwise current plain composer text. Save as draft waits
for durable acknowledgement. Failures stay visible, no false saved claim.
Restore text is available only while composer is empty; never overwrite new
text. Shelf layout stays in its separate queued change.
