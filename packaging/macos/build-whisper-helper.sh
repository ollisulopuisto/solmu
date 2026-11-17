#!/bin/bash
# Separate optional runtime artifact. Never invoked by a Solmu build.
set -euo pipefail
source_dir="$1"
out="$2"
arch="$3"
case "$arch" in arm64|x86_64) ;; *) echo 'Unsupported architecture' >&2; exit 1;; esac
cmake -S "$source_dir" -B "$out/build" \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES="$arch" \
  -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF \
  -DGGML_METAL_EMBED_LIBRARY=ON -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF -DWHISPER_BUILD_EXAMPLES=ON
cmake --build "$out/build" --config Release --target whisper-cli -j 3
app="$out/SolmuWhisper.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$out/build/bin/whisper-cli" "$app/Contents/MacOS/whisper-cli"
cp "$source_dir/LICENSE" "$app/Contents/Resources/whisper-LICENSE"
# The static build must not depend on files from its build directory or Homebrew.
otool -L "$app/Contents/MacOS/whisper-cli" | tail -n +2 | while read -r library rest; do
  case "$library" in /System/Library/*|/usr/lib/*) ;; *) echo "Unexpected runtime dependency: $library" >&2; exit 1;; esac
done
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>fi.sulopuisto.solmu.whisper</string>
<key>CFBundleName</key><string>SolmuWhisper</string>
<key>CFBundleExecutable</key><string>whisper-cli</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1.9.5</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>LSUIElement</key><true/>
</dict></plist>
PLIST
lipo "$app/Contents/MacOS/whisper-cli" -verify_arch "$arch"
