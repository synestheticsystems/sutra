#!/usr/bin/env bash
# Generate a macOS .icns from a square PNG master.
#
# Usage: make-icns.sh <source.png> <out.icns>
set -euo pipefail

SRC="${1:?usage: make-icns.sh <source.png> <out.icns>}"
OUT="${2:?usage: make-icns.sh <source.png> <out.icns>}"

MASTER="$(sips -g pixelWidth "$SRC" | awk '/pixelWidth/{print $2}')"
if [[ -z "$MASTER" ]]; then
    echo "error: could not read pixel width of $SRC" >&2
    exit 1
fi
if (( MASTER < 1024 )); then
    echo "WARN: icon master is ${MASTER}px. The 512px and 1024px (@2x) variants" >&2
    echo "      will be upscaled and look soft. Provide a 1024x1024 master for" >&2
    echo "      crisp Retina icons." >&2
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
ICONSET="$TMP/icon.iconset"
mkdir -p "$ICONSET"

gen() { # <size-px> <filename>
    sips -z "$1" "$1" "$SRC" --out "$ICONSET/$2" >/dev/null
}
gen 16   icon_16x16.png
gen 32   icon_16x16@2x.png
gen 32   icon_32x32.png
gen 64   icon_32x32@2x.png
gen 128  icon_128x128.png
gen 256  icon_128x128@2x.png
gen 256  icon_256x256.png
gen 512  icon_256x256@2x.png
gen 512  icon_512x512.png
gen 1024 icon_512x512@2x.png

mkdir -p "$(dirname "$OUT")"
iconutil -c icns "$ICONSET" -o "$OUT"
echo "icns written: $OUT (master ${MASTER}px)"
