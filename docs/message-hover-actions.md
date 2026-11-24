# Message hover actions

Hover a message to show quick reactions, the full reaction picker, Reply and Star. All controls act on the hovered message, not the neighbouring message below the bar. Clicking your current quick reaction removes it. Star switches to Unstar for a starred message.

The vertical reply/star strip is behind the horizontal reaction bar. The right joins are flush. The bar stays inside the visible transcript. Where the viewport is too small for the full bar, the existing small React/Reply controls remain. Keyboard users retain those focusable controls and the message context menu.

Calls have no toolbar. Selection and pending-media rows retain their existing interaction rules. Reply is unavailable for read-only chats, revoked messages and phone-only placeholders. Star is unavailable for revoked messages. No network or account changes are introduced.
