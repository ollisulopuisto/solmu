# Draft recovery controls

Normal automatic saving is silent. When meaningful text is cleared, show only
Save as draft, Restore text and Dismiss, no explanatory chip label. New typed text dismisses the old recovery controls. An attachment alone
preserves recovery text and its buttons. Save failures
remain visible with Retry; hiding normal saved/saving status never hides errors.

Cmd-Shift-S on macOS, Ctrl-Shift-S elsewhere, saves current plain composer text
or the just-cleared recovery text. It does not intercept attachments, editing or
modal dialogs. Save as draft shows the same shortcut. Shelf/active persistence
semantics stay transactional; no saved claim or composer clear before success.

This is independent of the queued left-icon/count overlay-shelf redesign.
