#!/usr/bin/env bash
# Build "Sutra Recording.app": Sutra's window over invented work, for recording the Mac App Store
# preview without anyone's machine in it. It is a build of its own (macos/Sources/SutraRecording),
# which has no live core to start. Its bundle identifier gives it a container and preferences of
# its own, apart from an installed Sutra.app, and the App Sandbox keeps it out of everything else.
#
# Usage: ./packaging/macos/recording.sh
#
# Output: dist/Sutra Recording.app (ad-hoc signed, current architecture only, not distributable)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

APP_NAME="Sutra Recording"
BUNDLE_ID="systems.synesthetic.sutra.recording"
ENTITLEMENTS="$SCRIPT_DIR/sutra-recording.entitlements"
APP_DIR="$REPO_ROOT/dist/$APP_NAME.app"

# macos/build.sh makes the core's Swift bindings and XCFramework, which the package reads, and
# builds the app with them, which this bundle has no use for.
echo "==> Building the core's bindings for $(uname -m)"
"$REPO_ROOT/macos/build.sh" >/dev/null

# The native build system, as macos/build.sh explains, so the window looks as the app's does.
echo "==> Building the recording build"
swift_build=(swift build -c release --package-path "$REPO_ROOT/macos" --product SutraRecording
    --build-system native)
"${swift_build[@]}"
EXECUTABLE="$("${swift_build[@]}" --show-bin-path)/SutraRecording"

echo "==> Assembling $APP_NAME.app"
# Replaces only this bundle: whatever else is in dist/ stays.
"$SCRIPT_DIR/make-app.sh" "$EXECUTABLE" "$APP_DIR"
PLIST="$APP_DIR/Contents/Info.plist"
plutil -replace CFBundleIdentifier -string "$BUNDLE_ID" "$PLIST"
plutil -replace CFBundleName -string "$APP_NAME" "$PLIST"
plutil -replace CFBundleDisplayName -string "$APP_NAME" "$PLIST"

echo "==> Ad-hoc signing with $(basename "$ENTITLEMENTS")"
codesign --force --sign - --entitlements "$ENTITLEMENTS" "$APP_DIR"
codesign --verify --strict "$APP_DIR"

# The sandbox applies only the entitlements the signature carries, so a signature that dropped
# them would let the recording reach what the sandbox keeps it out of.
if ! diff <(plutil -p "$ENTITLEMENTS") \
    <(codesign -d --entitlements - --xml "$APP_DIR" 2>/dev/null | plutil -p -); then
    echo "error: the signed entitlements differ from $ENTITLEMENTS" >&2
    exit 1
fi

echo ""
echo "Done. Launch it with:"
echo "  open -n \"$APP_DIR\""
