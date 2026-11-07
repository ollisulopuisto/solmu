#!/bin/bash
# Build Solmu.app from a GUI binary on macOS.
#
#   packaging/macos/bundle.sh <binary> <output.app> <version>
#
# Set CODESIGN_IDENTITY to use a Developer ID. Otherwise, use the ad-hoc
# signature required on arm64.
#
# Generate the .icns from the committed 1024px PNG with macOS iconutil.
# Info.plist is next to this script.
set -euo pipefail

binary="$1"
app="$2"
version="$3"
mode="${4:-beeper}"
if [[ "$mode" != beeper ]]; then echo "Only Beeper app mode is supported" >&2; exit 1; fi
numeric_version="${version%%-*}"
here="$(cd "$(dirname "$0")" && pwd)"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$binary" "$app/Contents/MacOS/solmu"
chmod 755 "$app/Contents/MacOS/solmu"
scheduler="${5:-}"
if [[ "$mode" == beeper ]]; then
    mv "$app/Contents/MacOS/solmu" "$app/Contents/MacOS/solmu-client"
    # swiftc otherwise builds only for the runner's host architecture. Match
    # the Rust client's slices so a universal release also has a universal
    # launcher (the bundle's main executable).
    launcher_build="$(mktemp -d)"
    trap 'rm -rf "$launcher_build"' EXIT
    launcher_slices=()
    for arch in $(lipo -archs "$app/Contents/MacOS/solmu-client"); do
        case "$arch" in
            arm64|x86_64) ;;
            *) echo "Unsupported launcher architecture: $arch" >&2; exit 1 ;;
        esac
        xcrun swiftc -swift-version 5 -framework AppKit \
            -target "$arch-apple-macosx11.0" "$here/BeeperLauncher.swift" \
            -o "$launcher_build/solmu-$arch"
        launcher_slices+=("$launcher_build/solmu-$arch")
    done
    lipo -create "${launcher_slices[@]}" -output "$app/Contents/MacOS/solmu"
    lipo "$app/Contents/MacOS/solmu" -verify_arch $(lipo -archs "$app/Contents/MacOS/solmu-client")
    cp "$here/scheduler-health.sh" "$app/Contents/Resources/scheduler-health.sh"
    # Optional Send Later sidecar, bundled for the one-time in-app Studio
    # install (still approval-gated in the launcher before any install).
    if [[ -n "$scheduler" ]]; then
        cp "$scheduler" "$app/Contents/MacOS/solmu-scheduler"
        chmod 755 "$app/Contents/MacOS/solmu-scheduler"
    fi
fi
sed "s/__VERSION__/$numeric_version/g" "$here/Info.plist" > "$app/Contents/Info.plist"

if [[ "$mode" == beeper ]]; then
    # A separate fork identity preserves existing installations.
    /usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier fi.sulopuisto.fast-beeper' "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName Solmu' "$app/Contents/Info.plist"
fi

# Starting in the tray at login: a login item cannot pass --start-hidden, so
# SMAppService registers this agent from the bundle instead.
identifier=me.paolino.fastsapp
mkdir -p "$app/Contents/Library/LaunchAgents"
cp "$here/autostart.plist" "$app/Contents/Library/LaunchAgents/$identifier.autostart.plist"

iconset="$(mktemp -d)/solmu.iconset"
mkdir -p "$iconset"
# iconutil reads these base sizes and optional @2x versions. It ignores 64x64.
for size in 16 32 128 256 512; do
    sips -z $size $size "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/solmu.icns"

# The notification plays Solmu's built-in sounds from the bundle by name, so
# Do Not Disturb and Focus silence them (src/notify/macos.rs).
sounds="$here/../../assets/sounds"
cp "$sounds/receive.wav" "$app/Contents/Resources/solmu-receive.wav"
cp "$sounds/alert.wav" "$app/Contents/Resources/solmu-alert.wav"

# Sign both executables in the fork, then the containing app.
if [[ "$mode" == beeper ]]; then
    # A bare Mach-O's default identifier includes its linker UUID. That changes
    # on every rebuild and breaks the Keychain ACL of this actual reader.
    # Set a stable identifier before native-packages preserves it on re-sign.
    codesign --force --identifier fi.sulopuisto.fast-beeper.client --entitlements "$here/entitlements.plist" --sign "${CODESIGN_IDENTITY:--}" "$app/Contents/MacOS/solmu-client"
    if [[ -f "$app/Contents/MacOS/solmu-scheduler" ]]; then
        codesign --force --identifier fi.sulopuisto.fast-beeper.scheduler --sign "${CODESIGN_IDENTITY:--}" "$app/Contents/MacOS/solmu-scheduler"
    fi
fi

# Attach the microphone entitlement before native-packages preserves it when
# replacing the ad-hoc signature with a hardened Developer ID signature.
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --options runtime \
        --entitlements "$here/entitlements.plist" --sign "$CODESIGN_IDENTITY" "$app"
else
    codesign --force --entitlements "$here/entitlements.plist" --sign - "$app"
fi
codesign --verify --strict "$app"

echo "$app"
