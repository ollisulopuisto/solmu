# Product

## Users and purpose

Solmu serves people reading and sending WhatsApp messages on Linux, macOS,
and Windows. It is a small native companion client built with Rust and egui.
The conversation is the primary workspace, with chats beside it and a composer
below it.

## Product character

Native, familiar, restrained. Preserve the existing interface, fonts, local
themes, and platform behaviour. WhatsApp Web is the supplied reference for
interactive message anatomy: media, readable text, message metadata, then
separated action rows.

## Design principles

- Keep messages readable and the composer immediately available.
- Use existing shared controls and theme colours consistently.
- Make working actions and unavailable actions distinguishable. Reply buttons
  send immediately, lists open dialogs with grouped choices, and copy-code actions stay local.
  Carousels keep independent cards in a horizontal strip. Poll results disclose
  known voters and vote times, preserving incomplete-history notices.
  Disable only unsupported actions or replies blocked by connection, chat
  permissions, or an in-flight send.
- Preserve selection, copying, emoji, links, and narrow-window usability.
- Validate screenshots with synthetic offline content only.

## Boundaries and accessibility

No browser engine, hosted backend, telemetry, or Solmu-operated account system.
WhatsApp only: no other networks or protocols (Matrix, Signal, Telegram) and no bridges to them.
Scripted access, command-line control, APIs and MCP may read or send messages,
including WhatsApp, only with authenticated, revocable, scoped tokens. A read
scope never grants sending. Sending needs a separate scope and explicit
account/destination limits. No unauthenticated message access is allowed.
Loopback-only is the default; remote exposure requires a separate secure
transport and access-control decision. Audit writes without message content or
credentials. Network terms and account restrictions still apply.
Send Later and ghost groups remain visible, user-controlled workflows with
chosen text and recipients and a clear way to stop them.
Several WhatsApp accounts may be linked locally. Current feature limitations
are not permanent exclusions; see the product boundaries in [AGENTS.md](AGENTS.md#product-boundaries).
Avoid decorative dashboards and extra nested cards inside message bubbles.
Retain keyboard operation, zoom, text selection, and light and dark themes.
No additional accessibility certification target is specified by the project.
