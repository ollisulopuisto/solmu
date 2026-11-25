#!/bin/bash
# Fetch the pinned whisper.cpp source into "$1" and verify the revision.
# Separate optional runtime input. Never invoked by a Solmu build.
set -euo pipefail
dest="$1"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
source "$here/pins.env"
git init -q "$dest"
git -C "$dest" remote add origin https://github.com/ggml-org/whisper.cpp.git
git -C "$dest" fetch -q --depth 1 origin "$WHISPER_REVISION"
git -C "$dest" checkout -q --detach FETCH_HEAD
test "$(git -C "$dest" rev-parse HEAD)" = "$WHISPER_REVISION"
echo "whisper.cpp $WHISPER_VERSION at $WHISPER_REVISION ready in $dest"
