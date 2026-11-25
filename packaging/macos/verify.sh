#!/bin/bash
# Verify the app that users receive inside a signed, notarized release DMG.
set -euo pipefail

dmg="$1"
architecture="${2:-universal}"
case "$architecture" in arm64|universal) ;; *) echo "Unknown architecture: $architecture" >&2; exit 1 ;; esac
verify_architectures() {
    bash "$(dirname "$0")/verify-architectures.sh" "$1" "$architecture"
}
temporary="$(mktemp -d)"
mount="$temporary/mount"
mkdir "$mount"
cleanup() {
    hdiutil detach "$mount" >/dev/null 2>&1 || true
    rm -f "$temporary/entitlements.plist"
    rmdir "$mount" "$temporary" 2>/dev/null || true
}
trap cleanup EXIT

xcrun stapler validate "$dmg"
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount" >/dev/null
app="$mount/Solmu.app"
codesign --verify --strict --deep "$app"
spctl --assess --type execute --verbose=2 "$app"
verify_architectures "$app/Contents/MacOS/solmu"
# This is the process that actually reads Keychain. Its designated requirement
# must survive a rebuild rather than inheriting a UUID-derived Mach-O identity.
client="$app/Contents/MacOS/solmu-client"
identity="$(codesign --display --verbose=4 "$client" 2>&1)"
grep -qx 'Identifier=fi.sulopuisto.fast-beeper.client' <<< "$identity"
grep -qx 'TeamIdentifier=LWSR8T5392' <<< "$identity"
requirement="$(codesign -d -r- "$client" 2>&1)"
grep -Fq 'identifier "fi.sulopuisto.fast-beeper.client"' <<< "$requirement"
grep -Fq 'LWSR8T5392' <<< "$requirement"
if grep -Eq 'cdhash|55554944' <<< "$requirement"; then
    echo "Client requirement is pinned to one build rather than its stable signing identity" >&2
    exit 1
fi
# Beeper bundles contain both a Swift launcher and the Rust client. Checking
# just CFBundleExecutable would miss an incorrectly packaged client.
verify_architectures "$app/Contents/MacOS/solmu-client"
codesign --display --entitlements - --xml "$app" > "$temporary/entitlements.plist"

python3 - "$app/Contents/Info.plist" "$temporary/entitlements.plist" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as source:
    info = plistlib.load(source)
with open(sys.argv[2], "rb") as source:
    entitlements = plistlib.load(source)
if info.get("LSUIElement") is not True:
    sys.exit("The tunnel launcher must start as an agent, not a second Cmd-Tab app")
if not info.get("NSMicrophoneUsageDescription", "").strip():
    sys.exit("The release app is missing its microphone permission description")
if entitlements.get("com.apple.security.device.audio-input") is not True:
    sys.exit("The signed release app is missing its audio-input entitlement")
if not info.get("NSCameraUsageDescription", "").strip():
    sys.exit("The release app is missing its camera permission description")
if entitlements.get("com.apple.security.device.camera") is not True:
    sys.exit("The signed release app is missing its camera entitlement")
print("Verified microphone and camera permission metadata in the signed app")
PY
