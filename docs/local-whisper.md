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
failure. Remove downloaded engine and model switches transcription off and removes both
runtime and model, but keeps transcripts. Repair verifies the existing model
and downloads it again only if missing or damaged. Named interrupted downloads
are cleaned in the background on next startup; an active-download lock protects
another running instance. Deleting transcripts is a separate action.

Audio decoding and transcription run in a worker. Local audio is converted to
private temporary 16 kHz mono WAV input; temporary audio and output are
removed after the process exits. Video decoding additionally needs a local
ffmpeg installation. No service credentials or cloud API are used. Base is
multilingual; leaving Language empty asks Whisper to detect the language.
Accuracy varies, especially for short clips and uncommon languages.

## Implementation status

Runtime selection is pinned for all six shipped targets to the published
`whisper-helper-v1.9.5` release; every URL, size and hash was recomputed
from the published assets. The macOS archives come from the signed and
notarized default-branch helper run; Linux and Windows assets are unsigned
CI builds and are verified against pinned hashes only, not an OS code
signature. Actual public-URL download/install/repair validation runs through
the `SOLMU_VALIDATE_PUBLIC_DOWNLOAD` smoke route. Do not treat the isolated
engine tests as end-to-end proof.

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
covered by an explicit manual smoke route. The arm64 run 44142397679 passed
production installation, damage/repair, real speech Opus encoding/decoding and
Whisper inference. Public-URL download and enabled-app validation remain pending.

Sources checked 2026-11-17:
- https://github.com/ggml-org/whisper.cpp/releases/tag/b5454
- https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-b5454-xcframework.zip
- https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/.github/workflows/release.yml
- https://support.apple.com/en-us/102445

Transcription accepts up to five minutes of audio. Encoded audio is limited to
64 MB and all source channels are downmixed/resampled incrementally before the PCM cap; oversized input
fails with a message, never silently transcribes only the beginning. Runtime
signature/Gatekeeper assessment runs on first use per worker, with a fingerprint of every runtime file checked again on later jobs. Long audio may need splitting first.

Linux and Windows runtime identity currently relies on the reviewed pinned
executable hash delivered by Solmu's signed/reviewed release, not an independent
OS code signature. No unsigned download is described as Authenticode-signed.
Apple runtime bundles additionally require Developer ID and Gatekeeper checks.

Nix packages pin their GNU tar extractor from the Nix store at compile time. Other Linux packages use `/usr/bin/tar`; Windows uses its system `tar.exe`. The extractor path is not read from a runtime environment variable.
