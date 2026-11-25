# Recorded voice MIME

Solmu records mono 48 kHz and encodes Ogg/Opus. Recorded clips now declare
`audio/ogg; codecs=opus` on both the base64 upload and message attachment.
The existing `voice-note` hint and duration in seconds remain unchanged.
Ordinary audio files still use Beeper's MIME detection; arbitrary Ogg is not
relabelled as Opus.

This removes MIME ambiguity for a known codec. It is not a confirmed diagnosis
of the reported silent WhatsApp send failure. There is no captured upload
response MIME, outgoing sendStatus/error, installed Beeper API schema/version,
or recipient delivery test from the affected installation.

Current API docs permit this MIME override and spell the type `voice-note`:
https://developers.beeper.com/desktop-api-reference/resources/messages/methods/send
https://developers.beeper.com/desktop-api-reference/resources/assets/methods/upload_base64

The upstream WhatsApp bridge advertises `audio/ogg; codecs=opus` for voice,
while generic `audio/ogg` is unsupported for voice capabilities. Its converter
also tolerates generic Ogg, so this alone does not prove the failure cause:
https://github.com/mautrix/whatsapp/blob/bf96ce9f/pkg/connector/capabilities.go
https://github.com/mautrix/whatsapp/blob/bf96ce9f/pkg/msgconv/from-matrix.go

A real-device test must confirm the clip arrives and plays in WhatsApp. Local
fixtures assert both HTTP payloads and preserve the ordinary-file behavior.
They do not encode audio or run Beeper. Native recording/UI, macOS and live
network delivery remain untested for this patch. Waveform is not a supported
Desktop API send field and is not added. The sample rate is not changed.
