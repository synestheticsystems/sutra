#!/usr/bin/env bash
# Wrap a built Sutra executable in an app bundle: the executable, Info.plist with the version from
# Cargo.toml, and the icon. Signing is left to the caller, which knows what the bundle is for.
#
# Usage: make-app.sh <executable> <out.app>
set -euo pipefail

EXECUTABLE="${1:?usage: make-app.sh <executable> <out.app>}"
APP_DIR="${2:?usage: make-app.sh <executable> <out.app>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VERSION="$(grep '^version = ' "$REPO_ROOT/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
# The names Info.plist gives the executable (CFBundleExecutable) and the icon (CFBundleIconFile).
cp "$EXECUTABLE" "$APP_DIR/Contents/MacOS/Sutra"
sed "s/__VERSION__/$VERSION/g" "$SCRIPT_DIR/Info.plist" > "$APP_DIR/Contents/Info.plist"
"$SCRIPT_DIR/make-icns.sh" "$REPO_ROOT/assets/icon.png" "$APP_DIR/Contents/Resources/AppIcon.icns"
