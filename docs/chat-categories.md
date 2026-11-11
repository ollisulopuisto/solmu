# Automatic chat categories

Chosen design: option 4 of the grouping sketches, hybrid of automatic
categories and tags. The labels Solmu already has are the tags. This first
slice adds the automatic categories and the per-chat override.

## What it does

Filters > Automatic category shows one of:

- **People**: one-to-one chats active in the last 7 days.
- **Quiet people**: one-to-one chats idle 7 days or more, up to 30
  (`QUIET_AFTER_SECS`).
- **Long quiet**: one-to-one chats idle 30 days or more
  (`LONG_QUIET_AFTER_SECS`). The maintainer suggested "7/30 days"; I read it
  as two tiers. Both limits are constants and easy to change.
- **Notifications**: channels, broadcasts and read-only chats.

Groups have no category; they stay under the Groups chip. A chat with no
recorded activity is never called quiet.

The rules are deterministic, run on this computer from the chat list's own
data, and send nothing anywhere. Choosing a category replaces the queue the
way a label does, so it combines with network, account, favorites and search.
Clear it with Any or Reset view. It is not remembered across restarts.

## Fading in search

While a search is active, a chat's name fades by quiet tier, in the chat rows
and in message search results: Quiet people at 25% toward the background,
Long quiet at 45%. The hue stays the theme's text colour. People and
Notifications keep the normal colour, the selected row stays plain, and the
default list (no search) is untouched. The fade never drops below 4.5:1
against the panel or a hovered row (`quiet_tint` in `src/ui/chats.rs`), so on
light themes the two steps sit closer together. Tests check the ordering and
the floor on both built-in themes and on a custom palette.

## Moving a chat

A one-to-one chat's menu has **Move to…** with the four categories and
**Sort automatically**. A move is stored in settings (`beeper.category_overrides`,
keyed like the Reply later marks), is this chat's alone, and the rules never
override it. Groups cannot be moved.

## Not in this slice

- Quiet and Notifications use only the chat list's fields: last activity, kind,
  read-only. Detecting one-way senders such as bots and business numbers needs
  message history (who writes, who answers) and is not done yet.
- The categories are not segments of the All / Unread / Reply later / Read
  later toggle and have no counts. They are an opt-in filter first, so the
  default list does not change.
- No sound or badge defaults for Notifications; nothing is muted.
- Categories attach to the chat, not the linked person, like the later marks.
- No palette commands or shortcuts yet.

## Roll back

Revert the change. Saved overrides in settings are ignored by older versions.
