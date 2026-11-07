#!/bin/bash
# No credentials needed: exercise the real bundle path with two rebuilt clients.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
for build in 1 2; do
    printf 'int main(void) { return %s; }\n' "$build" > "$work/main.c"
    for arch in arm64 x86_64; do
        xcrun clang -arch "$arch" -mmacosx-version-min=11.0 "$work/main.c" -o "$work/$arch"
    done
    lipo -create "$work/arm64" "$work/x86_64" -output "$work/client"
    bash "$here/bundle.sh" "$work/client" "$work/Solmu-$build.app" "0.0.$build" beeper
    client="$work/Solmu-$build.app/Contents/MacOS/solmu-client"
    codesign --display --verbose=4 "$client" 2>&1 | grep -qx 'Identifier=fi.sulopuisto.fast-beeper.client'
    shasum -a 256 "$client" | cut -d' ' -f1 > "$work/hash-$build"
done
if cmp -s "$work/hash-1" "$work/hash-2"; then
    echo 'Fixture did not exercise a rebuilt client' >&2
    exit 1
fi
echo 'Two different rebuilt clients keep the same explicit Keychain signing identifier'
