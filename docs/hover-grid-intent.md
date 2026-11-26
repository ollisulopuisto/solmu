# Message action grid intent

Pointer opening waits150ms on the target bubble. A quick pass-over does not
open it; leaving before the delay cancels the attempt. Keyboard opening stays
immediate. Placement still prefers the target bubble right side, then left,
without covering its own target or following subsequent pointer movement.

While open,12logical-point invisible padding surrounds the grid, enlarged
to24points on the arrival side facing the bubble. The target bubble and the
existing facing-edge bridge also keep it open. A diagonal path from the
last pointer position inside the bubble toward the grid stays safe while
it remains in the triangle to the grid edge and moves closer. This invisible
area does not capture clicks or cover other message controls.

Leaving those regions starts250ms grace, cancelled on re-entry. Close is
slower than open. Escape, click outside actual grid buttons, scroll, leaving
the window or selecting an action closes immediately. The target highlight
stays while the target is rendered and the grid remains open; virtualization
removes only the unavailable target outline, not the retained grid. No maximum
visible duration, new setting, changed action target or shortcut. Timing and
geometry are constants, not preferences.
