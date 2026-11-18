# Optional Whisper helper pipeline

Solmu never links or bundles a Whisper runtime. Users who pick the "On this
computer" voice-note engine get a standalone `whisper-cli` helper plus a
pinned model, both downloaded by Solmu at their explicit request and verified
byte-for-byte before use. This directory is the packaging pipeline that
produces those runtime archives for every supported platform. It is never
invoked by a Solmu build.

## Targets

| Target triple             | Runner            | Archive                                   | Executable in archive                    |
| ------------------------- | ----------------- | ----------------------------------------- | ---------------------------------------- |
| `aarch64-apple-darwin`    | `macos-15`        | `solmu-whisper-v1.9.5-arm64.zip`          | `SolmuWhisper.app/Contents/MacOS/whisper-cli` |
| `x86_64-apple-darwin`     | `macos-15-intel`  | `solmu-whisper-v1.9.5-x86_64.zip`         | `SolmuWhisper.app/Contents/MacOS/whisper-cli` |
| `x86_64-unknown-linux-gnu`| `ubuntu-24.04` + `debian:12` container | `solmu-whisper-v1.9.5-linux-x86_64.tar.gz` | `bin/whisper-cli` |
| `aarch64-unknown-linux-gnu`| `ubuntu-24.04-arm` + `debian:12` container | `solmu-whisper-v1.9.5-linux-aarch64.tar.gz` | `bin/whisper-cli` |
| `x86_64-pc-windows-msvc`  | `windows-2025`    | `solmu-whisper-v1.9.5-windows-x86_64.tar.gz` | `bin/whisper-cli.exe` |
| `aarch64-pc-windows-msvc` | `windows-11-arm`  | `solmu-whisper-v1.9.5-windows-arm64.tar.gz` | `bin/whisper-cli.exe` |

macOS keeps the existing `SolmuWhisper.app` bundle layout and archive names so
the signed default-branch pipeline (`whisper-helper.yml`) and this matrix
produce the same shape. Linux and Windows use plain `.tar.gz` (Windows ships a
system `tar.exe`) with the binary under `bin/`.

Linux builds and their inference smoke test run inside a `debian:12` container
so the binaries only need what a minimal Debian 12 ships; `build-linux.sh`
fails if `ldd` reports anything else. Windows links the MSVC runtime statically. All builds use a portable CPU
baseline (`GGML_NATIVE=OFF`, no AVX/AVX2/FMA/F16C/BMI2 on x86-64) so the helper
runs on any machine Solmu supports; pinned GGML refuses `GGML_NATIVE=OFF` with
MSVC on ARM, and MSVC ARM64 targets the ARMv8.0 baseline by itself, so the
Windows ARM64 helper stays portable without the flag.

## Files

- `pins.env` — the one place that pins the whisper.cpp revision and the model
  and smoke-model URLs/sizes/SHA-256s. Keep it in sync with
  `src/local_whisper.rs` and `src/whisper_runtime.rs`. Bash scripts source it;
  PowerShell scripts read it through `read-pins.ps1`.
- `fetch-source.sh` — fetches the pinned whisper.cpp source and verifies the
  checked-out revision.
- `build-linux.sh`, `build-windows.ps1` — build and package one archive.
  macOS uses the existing `packaging/macos/build-whisper-helper.sh`.
- `smoke-test.sh`, `smoke-test.ps1` — inference smoke test: download the
  pinned CI-only tiny model, transcribe the pinned source's JFK sample,
  require the expected phrase.
- `download-model.sh` — fetch the pinned full model (CI fixture), verified by
  size and SHA-256 before an atomic rename.
- `metadata-unix.sh`, `metadata-windows.ps1` — emit the per-target metadata
  JSON (`target`, `filename`, `bytes`, `sha256`, `executable`,
  `executable_sha256`, `upstream_revision`, `signed`, `notarized`) next to the
  archive and print it to the log.
- `README-runtime.txt` — template for the README shipped inside each archive;
  `@MODEL_*@` placeholders come from `pins.env`.

## Workflow

`.github/workflows/whisper-helper-matrix.yml` runs these scripts for all six
targets on pull requests that touch this pipeline, and on manual dispatch. It
verifies each archive in-job (dependency guard, inference smoke test, metadata
hashes recomputed from the produced files) and reports the metadata JSON in
the log. On Linux and Windows it also runs the #359 installer's real-app smoke
test (`local_whisper_real_recording_installs_repairs_and_transcribes_opus`,
pinned to a reviewed `feat/local-whisper-setup` commit) against the produced
archive, its metadata, the verified Base model and the public JFK sample. On
macOS only the standalone smoke test runs: the real-app test verifies
Gatekeeper, which an unsigned branch helper must fail, and verification is
never bypassed. The workflow uploads no artifacts, uses no caches, publishes
no releases and requests no secrets: macOS signing and notarization stay
exclusively in `whisper-helper.yml`, which only runs on the default branch. The hashes of
unsigned branch builds are not the release pins; when the maintainer
publishes, the metadata is recomputed from the published archives (signed and
notarized on macOS) and recorded in `src/whisper_runtime.rs`.
