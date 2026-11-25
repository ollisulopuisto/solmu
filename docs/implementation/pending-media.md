# Beeper pending-media bubbles

Dragged/staged files and pasted pictures enter the transcript at dispatch, before
an upload starts. A local row retains the file path/name or pasted pixels and its
caption. The backend is given an exact server/account/chat identity and validates
it before upload and again immediately before the message POST.

The first item carries the caption and quote. Ordering is the staged order.
Several media items are queued on one bounded worker. Queue refusal means nothing
was submitted for that item.

## Retry boundary

A separate atomic flag marks entry into the message POST, not the asset upload.
A failed upload or preparation can be retried. Once the message POST begins,
errors and malformed/unexpected acceptance are unknown outcomes and never offer a
Retry button. This decision does not depend on matching error-message strings.
The user is told to check history. A new retry changes state immediately so a
second click cannot enqueue the same item again.

Acceptance is queued, not delivered. Pending-ID lookup requires exact account,
chat and outgoing ownership. The returned message replaces the local row in its
existing slot; duplicates already observed in history are removed by exact ID.
Final-ID resolution updates that slot again. Bridge SUCCESS removes the local
submission annotation, leaving ordinary delivery/read indicators. Failure or a
60-second confirmation timeout keeps an unknown-outcome annotation.

Local placeholders do not offer ordinary reply/forward/delete/reaction actions.
They cannot become a quote target for another send. Both ordinary and linked
transcripts display submission state. Sending while viewing older history does
not force a scroll to the newest edge.

## Limits and validation

This first version is for staged files and pasted pictures in Beeper mode. The
existing specialized voice/GIF/sticker paths and WhatsApp backend are unchanged.
The existing 32 MiB upload bound remains. Unsupported view-once/selected mentions
keep attachments in the composer instead of pretending to send them.

Pending submission state is held in memory, per Solmu account. It is not a
persistent outbox across app restarts. Do not promise restart recovery. Unknown
outcomes require checking destination history before any new manual send.

Tests cover pre-POST vs post-POST failures, wrong destination account, exact
pending-to-final identity, sender ownership, timeout ambiguity, dispatch-time
visibility, no optimistic delivery, in-place replacement, stable viewport intent,
and safe-retry single dispatch. The POST-boundary test fails when its atomic
marker is removed and passes when restored.

Synthetic UI captures validate readable Sending, Not submitted/Retry and Check
history states. They do not test real bridge sends, macOS interactions or native
audio. Real networks still need manual verification.

Review follow-up: unresolved media conversations are kept across cache rotation
and trimming, refused options retain view-once, and sparse echoes bind identity
without erasing staged content. Explicit delivery evidence completes submission
even when attachment metadata is sparse. In that case a non-retryable Confirmed
record retains the local preview, without a sending/unknown annotation, until
full metadata arrives or the app closes. Confirmation and metadata availability
are separate facts. This retained preview is still in-memory only.
