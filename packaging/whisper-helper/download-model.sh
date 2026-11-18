#!/bin/bash
# Download the pinned model to "$1" and verify size and SHA-256 before the
# atomic rename. Used by CI fixtures; Solmu itself downloads and verifies the
# same pin in src/local_whisper.rs.
set -euo pipefail
target="$1"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
source "$here/pins.env"
mkdir -p "$(dirname "$target")"
tmp="$target.download"
curl --fail --location --retry 2 --silent --show-error "$MODEL_URL" -o "$tmp"
actual_bytes="$(stat -c%s "$tmp" 2>/dev/null || stat -f%z "$tmp")"
test "$actual_bytes" = "$MODEL_BYTES" || { rm -f "$tmp"; echo "Model size mismatch: $actual_bytes" >&2; exit 1; }
if command -v shasum >/dev/null 2>&1; then
  actual_hash="$(shasum -a 256 "$tmp" | cut -d' ' -f1)"
else
  actual_hash="$(sha256sum "$tmp" | cut -d' ' -f1)"
fi
test "$actual_hash" = "$MODEL_SHA256" || { rm -f "$tmp"; echo "Model hash mismatch: $actual_hash" >&2; exit 1; }
mv "$tmp" "$target"
echo "Model verified: $target"
