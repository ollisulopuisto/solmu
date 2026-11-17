# Local voice-note transcription

Settings > Voice notes lets you select Off, On this computer, or My Wyoming
server. Existing Wyoming settings keep using that server after an update.
Selecting a different engine is explicit. A local failure never sends audio
to a saved server address.

On this computer uses the bundled whisper.cpp engine and a pinned Whisper
Base multilingual model. The model is not included in the app download.
Selecting the engine alone does not fetch anything. Download & enable is the
only action that starts a model download. It fetches 147,951,465 bytes over
HTTPS from Hugging Face, verifies the pinned SHA-256, and installs the model
atomically under the app's state directory. Failed or canceled downloads are
removed. A prior installed model is not replaced with a partial file.

Allow 165 MB free disk space and about 500 MB RAM during transcription.
Download cancellation can wait for an in-flight network operation to stop;
body reads have a five-second timeout. Retry with Download & enable after a
failure. Remove downloaded model switches transcription off and removes the
model, but keeps transcripts. Deleting transcripts is a separate action.

Audio decoding and transcription run in a worker. Local audio is converted to
private temporary 16 kHz mono WAV input; temporary audio and output are
removed after the process exits. Video decoding additionally needs a local
ffmpeg installation. No service credentials or cloud API are used. Base is
multilingual; leaving Language empty asks Whisper to detect the language.
Accuracy varies, especially for short clips and uncommon languages.

## Implementation status

Runtime bundling and the source-policy decision are still pending. This
branch is not a usable released feature. Engine/installer regressions pass
in isolation and the app library passes check/clippy; whole-app native tests,
screenshots, signed packaging and real-recording actual-app validation are
still required. Do not treat the isolated engine tests as end-to-end proof.
