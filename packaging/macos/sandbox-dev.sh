#!/usr/bin/env bash
# Build "Sutra Sandbox.app", a local bundle that runs inside the App Sandbox with the entitlements
# the Mac App Store edition will carry, so the open-folder grant can be exercised exactly as that
# edition meets it. Its own bundle identifier gives it its own sandbox container and preferences,
# apart from an installed Sutra.app.
#
# Usage: ./packaging/macos/sandbox-dev.sh
#
# Output: dist/Sutra Sandbox.app (ad-hoc signed, current architecture only, not distributable)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

APP_NAME="Sutra Sandbox"
BUNDLE_ID="systems.synesthetic.sutra.sandbox"
ENTITLEMENTS="$SCRIPT_DIR/sutra-sandbox.entitlements"
APP_DIR="$REPO_ROOT/dist/$APP_NAME.app"

echo "==> Building the app for $(uname -m)"
EXECUTABLE="$("$REPO_ROOT/macos/build.sh")"

echo "==> Assembling $APP_NAME.app"
# Replaces only this bundle: a Sutra.app that bundle.sh left in dist/ stays. Left unstripped,
# unlike bundle.sh's release, so a crash report from inside the sandbox names its functions.
"$SCRIPT_DIR/make-app.sh" "$EXECUTABLE" "$APP_DIR"
PLIST="$APP_DIR/Contents/Info.plist"
plutil -replace CFBundleIdentifier -string "$BUNDLE_ID" "$PLIST"
plutil -replace CFBundleName -string "$APP_NAME" "$PLIST"
plutil -replace CFBundleDisplayName -string "$APP_NAME" "$PLIST"

echo "==> Ad-hoc signing with $(basename "$ENTITLEMENTS")"
codesign --force --sign - --entitlements "$ENTITLEMENTS" "$APP_DIR"
codesign --verify --strict "$APP_DIR"

# The sandbox applies only the entitlements the signature carries, so a signature that dropped one
# would test a different app than the store will run.
echo "==> Entitlements in the signature"
codesign -d --entitlements - "$APP_DIR"
if ! diff <(plutil -p "$ENTITLEMENTS") \
    <(codesign -d --entitlements - --xml "$APP_DIR" 2>/dev/null | plutil -p -); then
    echo "error: the signed entitlements differ from $ENTITLEMENTS" >&2
    exit 1
fi

echo ""
echo "Done. Launch it with:"
echo "  open -n \"$APP_DIR\""
