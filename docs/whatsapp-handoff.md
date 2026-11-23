# WhatsApp link handoff on macOS

The app bundle registers `whatsapp:` as an optional URL handler. macOS decides which installed app handles it. Solmu does not change the user's default handler. Browser `wa.me` links are web links, not a universal-link association with Solmu. The client can also take a WhatsApp link as its explicit launch argument.

Incoming text is an unsent composer template. It never sends automatically or replaces a text/attachment draft, edit, recording or armed reply. An unchanged template is not stored as a user draft. App-lock requests wait for unlock.

Routing uses an exact phone handle in a loaded direct WhatsApp chat with a real Beeper origin/account/chat identity. It does not turn a phone number into a synthetic Beeper chat ID. Multiple matches, including another locked match, are ambiguous and do not open a destination. Missing chats wait up to 30 seconds for inventory then stop. Solmu does not create a remote chat, search other local profiles or pick an account from list order. Linked-person composition explicitly selects the resolved WhatsApp source only after accepting the text.

The Beeper bundle launcher receives cold/running-app URL opens and forwards one argument to the existing client, without a shell. The native client registers a separate Apple-event handler before its first event pump so delegate/window changes do not replace it. Incoming requests are bounded, never logged, and contain no credentials.

## Mac acceptance checks still needed

- Build/package both native and Beeper bundles; check Swift launcher compilation.
- Select Solmu as the handler and open a WhatsApp send link with encoded Unicode, ampersand, newline and a URL. Verify the exact real WhatsApp account/chat and unsent text.
- Repeat cold, running, hidden and app-locked. Unlock before checking the destination.
- Existing text and attachment drafts, edits, recordings and armed replies must remain intact.
- Same number on two WhatsApp accounts must not choose either. An iMessage chat with that number must never match.
- In a linked timeline, verify the send destination is WhatsApp, including after a refused prefill.

These are incoming URL links, not a native share extension. No iMessage deep-link support is added.

## Finnish domestic numbers

A domestic `040 123 4567` becomes `+358401234567`, including inside a send link. Bare `40 123 4567` does too. Explicit `+46…`, `0046…` and `358…` keep their country code. Nonzero digit-only WhatsApp URL targets retain their international meaning. For compatibility, bare digit-only numbers longer than ten digits also retain the existing international interpretation; use the Finnish leading zero for an unambiguous domestic number. This normalization does not broaden exact chat matching or create missing chats.
