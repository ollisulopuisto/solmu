# Local voice-note transcription

Settings > Voice notes lets you select Off, On this computer, or My Wyoming
server. Existing Wyoming settings keep using that server after an update.
Selecting a different engine is explicit. A local failure never sends audio
to a saved server address.

On this computer is intended to download a platform-specific prebuilt engine
and a pinned Whisper Base multilingual model only when requested. Neither is
included in the app download. Solmu has no Whisper build-time dependency.
Selecting the engine alone does not fetch anything. Download & enable is the
only action that starts a model download. It fetches 147,951,465 bytes over
HTTPS from Hugging Face, verifies the pinned SHA-256, and installs the model
atomically under the app's state directory. Failed or canceled downloads are
removed. A prior installed model is not replaced with a partial file.

Allow 260 MB free disk space for the model, runtime and temporary staging.
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

Runtime download selection is still pending. This
branch is not a usable released feature. Engine/installer regressions pass
in isolation and the app library passes check/clippy; whole-app native tests,
screenshots, downloaded-runtime security validation and real-recording actual-app validation are
still required. Do not treat the isolated engine tests as end-to-end proof.

## macOS runtime research

The official b5454 release offers an Apple XCFramework, not a whisper-cli
executable. Its macOS binary is a universal Mach-O dynamic library. It cannot
be launched as the CLI. Linux and Windows CLI archives exist in that release.
A separate optional helper pipeline now builds a pinned official CLI source
revision. It is separate from all Solmu builds. The first arm64 trial passed
Developer ID signing, notarization, stapling and quarantined Gatekeeper
assessment. Intel validation and runtime asset publication are still pending.

A pinned hash does not establish Gatekeeper approval. No code strips quarantine,
weakens library validation or self-signs downloaded executables. The first helper trial passed
these checks; production app installation, repair and real Opus inference are
covered by an explicit manual smoke route whose passing result is still pending.

Sources checked 2026-11-17:
- https://github.com/ggml-org/whisper.cpp/releases/tag/b5454
- https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-b5454-xcframework.zip
- https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/.github/workflows/release.yml
- https://support.apple.com/en-us/102445
