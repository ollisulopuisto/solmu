# Quiet thread entry

Remove the always-visible loaded-reply count below a message. Keep thread navigation in a reserved slot immediately before its timestamp. Reveal the label on message/footer hover or keyboard focus; reserving the same slot every frame prevents hover reflow. Pointer click or Enter opens the same exact parent thread as before. Loaded-history availability remains an internal condition, not visible count text.

This is separate from #453's connected reply tails and #452's ordinary last-message tail ownership. It adds no history requests, API changes or sending behavior. Albums retain each member's exact thread identity rather than routing all controls to the last member. Long/many-member albums and narrow viewports still require native verification before readiness.

Local validation: production entry helper pointer/keyboard/rect regression passes. Mutation replacing click sense with hover fails; restored source passes. Full application regression added for timestamp stability and exact thread scope. Native CI and visual proof pending.
