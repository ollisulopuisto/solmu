# Link suggestions

First slice of proactive account linking. Solmu looks for one-to-one chats on
different networks that are very likely the same person and offers to link
them. It never links on its own.

## What matches

Two direct chats on the same Beeper server, on different networks, match when:

- their names are the same full name (two or more words, accents and case
  ignored), or
- they share an identical phone number or username. Only the other person's
  handles count; your own are skipped.

Close names, single first names, numbers used as names and placeholder titles
never match. Fuzzy matching stays in the manual "Link person..." picker.
Groups, locked chats and chats already linked together are skipped. Two people
you have already linked are never offered for merging, and a person holds at
most 8 chats.

## Where it shows

- Contact info: a "Same person?" card with **Link** and **Not the same person**.
- Filters menu: a collapsed "Link suggestions (N)" row listing every pending
  pair, with the same two buttons.
- Nothing appears on chat rows.

**Link** creates the person (or adds the chat to the person one of them
already belongs to) and shows an Undo toast for 8 seconds. **Not the same
person** is remembered per pair in `beeper.dismissed_links` and never comes
back.

## Local only

Matching runs on names and handles already loaded. It makes no request, and
nothing is sent to Beeper or any network.
