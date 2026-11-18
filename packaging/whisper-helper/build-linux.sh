#!/bin/bash
# Build the unsigned optional Whisper helper for Linux and package it as a
# .tar.gz with bin/whisper-cli inside. Debian 12 is the runtime baseline, so
# this must run on Debian 12 (CI uses a debian:12 container) and the result
# must not depend on anything a minimal Debian 12 does not ship.
# Usage: build-linux.sh <whisper-source-dir> <out-dir> <x86_64|aarch64>
set -euo pipefail
source_dir="$1"
out="$2"
arch="$3"
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
source "$here/pins.env"
case "$arch" in x86_64|aarch64) ;; *) echo 'Unsupported architecture' >&2; exit 1;; esac
# GGML_NATIVE=OFF alone still enables AVX2/FMA/BMI2 in pinned GGML on x86_64.
# Use a portable baseline so the helper runs on any x86_64 or ARMv8 machine.
cpu_flags=(-DGGML_NATIVE=OFF)
if [ "$arch" = x86_64 ]; then
  cpu_flags+=(-DGGML_SSE42=OFF -DGGML_AVX=OFF -DGGML_AVX2=OFF
    -DGGML_FMA=OFF -DGGML_F16C=OFF -DGGML_BMI2=OFF)
fi
cmake -S "$source_dir" -B "$out/build" \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_OPENMP=OFF \
  -DCMAKE_EXE_LINKER_FLAGS='-static-libstdc++ -static-libgcc' \
  -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF \
  -DWHISPER_BUILD_EXAMPLES=ON "${cpu_flags[@]}"
cmake --build "$out/build" --config Release --target whisper-cli -j "$(nproc)"
stage="$out/package"
mkdir -p "$stage/bin"
cp "$out/build/bin/whisper-cli" "$stage/bin/whisper-cli"
cp "$source_dir/LICENSE" "$stage/whisper-LICENSE"
sed -e "s|@MODEL_NAME@|$MODEL_NAME|g" -e "s|@MODEL_URL@|$MODEL_URL|g" \
  -e "s|@MODEL_BYTES@|$MODEL_BYTES|g" -e "s|@MODEL_SHA256@|$MODEL_SHA256|g" \
  "$here/README-runtime.txt" > "$stage/README.txt"
# The helper must only need libraries a minimal Debian 12 ships.
ldd "$stage/bin/whisper-cli" | while read -r library rest; do
  base="$(basename "$library")"
  case "$base" in
    linux-vdso.so.*|libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|ld-linux*.so.*) ;;
    *) echo "Unexpected runtime dependency: $library" >&2; exit 1;;
  esac
done
mkdir -p "$out/dist"
archive="solmu-whisper-v$WHISPER_VERSION-linux-$arch.tar.gz"
tar -czf "$out/dist/$archive" -C "$stage" bin whisper-LICENSE README.txt
echo "$out/dist/$archive"
