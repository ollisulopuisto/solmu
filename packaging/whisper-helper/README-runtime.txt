Solmu optional Whisper helper
=============================

This archive is Solmu's optional local voice-note transcription runtime. It is
a standalone build of the pinned whisper.cpp `whisper-cli`; Solmu never links
or bundles Whisper at build time, and nothing here is required unless you turn
on the "On this computer" engine in Settings > Voice notes.

Contents
  bin/whisper-cli   the transcription binary (whisper-cli.exe on Windows)
  whisper-LICENSE   the upstream whisper.cpp license (MIT)

Model
  The helper needs a Whisper model, downloaded separately by Solmu from this
  pinned, verified source:
    name:    @MODEL_NAME@
    url:     @MODEL_URL@
    bytes:   @MODEL_BYTES@
    sha256:  @MODEL_SHA256@
  A file is only used when its size and SHA-256 match exactly.

Usage (Solmu runs this for you; no command line is needed)
  whisper-cli -m @MODEL_NAME@ -f recording.wav
