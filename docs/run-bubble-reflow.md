# Run bubble sizing and reflow

Received text hugs its contents without a minimum. Sent bubbles align right,
with a 120-point outer minimum unless the pane itself is narrower. Within each
same-source, same-sender, same-day sent run, natural widths round up to a third,
two thirds, or all of the widest natural bubble. Quotes and cards stay inside.
All skins use outer top-corner tails on every bubble, 4-point row gaps and
14-point run gaps. A run gets its timestamp and normal receipt on its last row;
per-message reactions, failure actions and marks remain attached to that row.

When a new message changes a run, it appears at its final width first. After
87.5ms, changed older visible bubbles resize nearest-to-new first, sequentially,
210ms each. The bounce goes to 128% of the width change, back to 89.5%, then
settles. New arrivals retarget from currently drawn widths, not queued motions.
History loads, viewport/font changes, hidden rows and Reduce motion snap.
Appearance has a Reduce motion switch; active accessibility clients also skip
reflow. The existing transcript anchoring retains the newest position only for
readers already at the bottom and compensates changes above scrolled-back views.

Validation: five pure Rust width/timing tests pass locally, including append,
stagger, overshoot, retarget, history/cap changes and pruning. Native geometry,
footer, card-containment and both-theme tests updated for the approved design.
Native app compilation, full test suite, and visual capture await CI. Local
native compilation is blocked by missing PipeWire development headers.
