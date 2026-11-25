#!/bin/bash
# Exact slices, not just presence: an arm-only release must not hide Intel code.
set -euo pipefail
binary="$1"
case "${2:-universal}" in
    arm64) expected=arm64 ;;
    universal) expected='arm64 x86_64' ;;
    *) exit 1 ;;
esac
actual="$(lipo -archs "$binary" | tr ' ' '\n' | sed '/^$/d' | sort | paste -sd ' ' -)"
[[ "$actual" == "$expected" ]] || { echo "Wrong slices: $actual; expected $expected" >&2; exit 1; }
