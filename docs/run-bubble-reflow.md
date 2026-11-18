# Run bubble sizing and reflow

Received text hugs its contents without a minimum. Sent bubbles align right.
Within each same-source, same-sender, same-day sent run, plain text bubbles
join the run's widest natural text width when they are near-equal to it
(within 16 points AND 15% of the larger, #282's tolerance); every other
bubble keeps its natural width, and media, cards, quotes and forwards never
take part in the shared width at all, so one wide message cannot inflate the
short replies around it. All skins use outer top-corner tails on every
bubble, 4-point row gaps and 14-point run gaps. A run gets its timestamp and
normal receipt on its last row; per-message reactions, failure actions and
marks remain attached to that row.

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
Full native library/test-target compilation and strict clippy passed locally.
Two additional extracted-function regressions failed before the follow-up fix
and passed afterward: poster bytes do not invalidate geometry, and received
albums use their natural 320-point width capped by the pane. The full native
suite and final-head pixel capture are CI gates, not local execution claims.

Review follow-up: linked width signatures cover preview/card/poster and quote
inputs, and the linked planner reserves the same network-chip lane as rendering.
A same-text no-preview/preview regression and narrow-lane cap assertion cover
those paths. Pin notices keep both separated bodies and their receipt slots.


Width signatures hash text and layout shape without formatting payloads or
traversing thumbnail/waveform bytes. Received albums retain their natural card
width; sent albums use card-based band planning rather than a viewport fallback.

CI follow-up: photo widths start from their actual frame ratio, call rows from
their title/detail fonts, and wide carousels use the renderer's distinct cap.
Old seam-overlap, last-tail-only and per-minute-clock assertions now test the
approved separated rows, every-body tails and one footer per sender/day run.

Adversarial review follow-up: resolved mention names participate in the width
signature, so unchanged mention rows reuse their measurement. Stable plans reuse
their order and target maps instead of rebuilding those collections. Explicit
invalidation builds its suffix once, not per cached row. Linked rendering no
longer computes or passes the obsolete cadence/neighbor arguments. Padding-click
coverage now tests both separated neighboring bubbles. Full-timeline signatures
still have linear cost, and pane/font changes deliberately remeasure; large
history resize performance has not been benchmarked. History inserts snap, and
bounce overshoot is bounded by the pane cap.
