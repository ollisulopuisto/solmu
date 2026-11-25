#!/bin/bash
# Write the per-target metadata JSON the Solmu installer consumes, then print
# it. Nothing is uploaded; the log is the report. Usage:
# metadata-unix.sh <archive-file> <executable-file> <executable-path-in-archive> <target-triple> <signed> <notarized>
set -euo pipefail
archive="$1"
executable_file="$2"
executable_path="$3"
target="$4"
signed="$5"
notarized="$6"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
source "$here/pins.env"
if command -v shasum >/dev/null 2>&1; then
  archive_hash="$(shasum -a 256 "$archive" | cut -d' ' -f1)"
  exe_hash="$(shasum -a 256 "$executable_file" | cut -d' ' -f1)"
else
  archive_hash="$(sha256sum "$archive" | cut -d' ' -f1)"
  exe_hash="$(sha256sum "$executable_file" | cut -d' ' -f1)"
fi
archive_bytes="$(stat -c%s "$archive" 2>/dev/null || stat -f%z "$archive")"
cat > "$archive.json" <<JSON
{
  "target": "$target",
  "filename": "$(basename "$archive")",
  "size": $archive_bytes,
  "bytes": $archive_bytes,
  "sha256": "$archive_hash",
  "executable": "$executable_path",
  "executable_sha256": "$exe_hash",
  "upstream_revision": "$WHISPER_REVISION",
  "signed": $signed,
  "notarized": $notarized
}
JSON
cat "$archive.json"
