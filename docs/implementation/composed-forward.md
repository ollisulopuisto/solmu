# Composed forwarding in Beeper mode

This is a new send, not a bridge-native forward. The text header includes sender,
network and original timestamp (including local time zone). The source group's
name is an explicit, default-off disclosure. Source text is escaped rather than
carrying source mention identities or rich-text control syntax into another chat.
A zero-width separator after each @ prevents destination-side mention scanning
without changing the visible text.

Each attachment is downloaded through the authenticated Beeper asset endpoint,
re-uploaded, and sent separately. The first attachment carries the source body;
subsequent attachments carry only the provenance header. A missing or unsupported
source refuses the item rather than sending an empty provenance-only message.
Attachments use the existing cache limits and a 32 MiB upload limit.

## Submission and retry

- Preparation failure: no message POST began. Safe to retry this item.
- Sending: a request is in progress.
- Accepted: Beeper returned a pending ID for the exact destination. This does not
  prove network delivery.
- Unknown: the message POST failed or its response could not prove acceptance.
  Check destination history. Never bulk-retry this state.

Retries preserve accepted and unknown rows and re-plan a failed preparation,
including expanding all attachments. This prevents text-only retries of a failed
attachment lookup and prevents duplicate sends of already accepted attachments.
The progress dialog stays open while requests run; it cannot be replaced by a
second forward batch. There is no batch-cancel control in this first version.
Session shutdown stops before the next message POST where possible. In-flight
POST outcomes can remain unknown.

Solmu stores an exact server/account/chat/message local forwarding marker and
adds the resolved final ID when pending-send confirmation arrives. This does not
change a bridge's native metadata.

## Verification and limits

The adapter fixture suite covers metadata/privacy, literal text, pre-POST source
identity failure, ambiguous POST responses, exact acceptance, original filenames,
selected-attachment upload, retry re-planning and accepted-row preservation.
Native App tests cover duplicate confirmation, stable retry indices and keeping
active progress visible. Synthetic screenshots do not prove live delivery.

Real networks and macOS interaction require manual testing. Do not interpret
per-network fixture labels as coverage of actual bridge behavior.

API contracts checked:
- https://developers.beeper.com/desktop-api-reference/resources/messages/methods/send
- https://developers.beeper.com/desktop-api-reference/resources/messages/methods/retrieve
- https://developers.beeper.com/desktop-api-reference/resources/assets/methods/upload_base64

Review follow-up: batch rows and generation belong to the owning app account.
Hidden-account events still update progress and local provenance; switching back
restores the results dialog. Non-text display placeholders (including locations)
are refused against the original wire type/body before a POST. Accepted pending
identities are saved with exact server/account/chat and resumed as read-only
confirmation lookups on startup/reconnection. Confirmed final IDs replace pending
marks; timeout keeps the saved association for another startup, never resends.
