#!/bin/bash
# Build only the requested slices. Routine manual runs default to Apple Silicon.
set -euo pipefail
architecture="${1:-arm64}"
case "$architecture" in arm64|universal) ;; *) echo "Unknown architecture: $architecture" >&2; exit 1 ;; esac
rustup target add aarch64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin
mkdir -p macos-binary
if [[ "$architecture" == universal ]]; then
    rustup target add x86_64-apple-darwin
    cargo build --locked --release --target x86_64-apple-darwin
    lipo -create target/aarch64-apple-darwin/release/solmu target/x86_64-apple-darwin/release/solmu -output macos-binary/solmu
else
    cp target/aarch64-apple-darwin/release/solmu macos-binary/solmu
fi
# Leave space for the signed app, DMG and notarization copy.
rm -rf target
