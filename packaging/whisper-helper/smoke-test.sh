#!/bin/bash
# Inference smoke test for a built helper binary: fetch the pinned CI-only
# smoke model, transcribe the pinned source's JFK sample and require the
# expected phrase. Usage: smoke-test.sh <whisper-cli-binary> <whisper-source-dir> <work-dir>
set -euo pipefail
binary="$1"
source_dir="$2"
work="$3"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
source "$here/pins.env"
mkdir -p "$work"
model="$work/$SMOKE_MODEL_NAME"
curl --fail --location --retry 2 --silent --show-error "$SMOKE_MODEL_URL" -o "$model"
actual_bytes="$(stat -c%s "$model" 2>/dev/null || stat -f%z "$model")"
test "$actual_bytes" = "$SMOKE_MODEL_BYTES" || { echo "Smoke model size mismatch: $actual_bytes" >&2; exit 1; }
if command -v shasum >/dev/null 2>&1; then
  actual_hash="$(shasum -a 256 "$model" | cut -d' ' -f1)"
else
  actual_hash="$(sha256sum "$model" | cut -d' ' -f1)"
fi
test "$actual_hash" = "$SMOKE_MODEL_SHA256" || { echo "Smoke model hash mismatch: $actual_hash" >&2; exit 1; }
transcript="$work/jfk.txt"
"$binary" -m "$model" -f "$source_dir/samples/jfk.wav" -nt -otxt -of "$work/jfk" >/dev/null
grep -qi "$SMOKE_EXPECTED" "$transcript" || { echo 'Smoke test transcript missing the expected phrase' >&2; cat "$transcript" >&2; exit 1; }
echo "Smoke test passed: $binary transcribed jfk.wav with $SMOKE_MODEL_NAME"
