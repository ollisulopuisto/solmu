# Send Later presets

The clock beside Send expands into Send later with four choices. In the upward
menu, 8am is nearest the button, then a time from the conversation, a custom
date/time, and recurring farthest away. Opening the menu sends nothing.

- Cmd+Shift+Enter on Mac, Ctrl+Shift+Enter elsewhere opens the menu from the composer.
- Up moves away from the button, Down toward it. Enter chooses the highlighted row.
- 1 chooses the next 8am, 2 the parsed time, 3 custom date/time, 4 recurring.
- Esc closes the menu and returns to the unchanged draft. Tab is also available.
- Numbers are intercepted only while the menu is open. Normal typing is unchanged.

The first two choices show their actual resolved local date and time before they
are chosen. The draft stays until the scheduler acknowledges acceptance. Custom
and recurring open a calendar, editable date/time fields, and the existing repeat
and end controls. Each schedule belongs to the exact composer destination,
including the selected source in a linked view; it never picks a different chat.
The schedule queue clock remains a separate control.

Time parsing is deliberately limited: HH:MM, Finnish `klo HH.MM`, am/pm, today /
tomorrow / tänään / huomenna and ISO dates (YYYY-MM-DD). Relative dates are tied to
the source message's day, not the day the menu opens. The most recent matching
text message in the destination chat wins. Text with multiple times, ambiguous
bare hours, unsupported dates or explicit zones is not guessed. A stale explicit
date or a daylight-saving gap/overlap disables the preset instead of rolling it
forward or skipping to an older unrelated message. Choose the calendar when the
preset is unavailable. Parsed choices are frozen while the menu is open, so an
incoming message cannot change the action under the pointer.
