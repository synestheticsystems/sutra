#!/usr/bin/env bash
# Build, sign, and (optionally) notarize a macOS .app + .dmg for sutra,
# for Developer ID distribution (NOT the Mac App Store — sutra is unsandboxed).
#
# Usage:
#   # Local test build (ad-hoc signed, not distributable):
#   ./packaging/macos/bundle.sh
#
#   # Distributable build (signed + notarized + stapled):
#   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
#   NOTARY_PROFILE="sutra-notary" \
#   ./packaging/macos/bundle.sh
#
# Env vars:
#   SIGN_IDENTITY   Developer ID Application identity. If empty, ad-hoc signs.
#   NOTARY_PROFILE  notarytool keychain profile (see packaging/macos/README.md).
#   SKIP_NOTARIZE   Set to 1 to sign with Developer ID but skip notarization.
#
# Output: dist/Sutra-<version>.dmg and dist/Sutra.app
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

APP_NAME="Sutra"
BIN_NAME="sutra"
DIST_DIR="$REPO_ROOT/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"

SIGN_IDENTITY="${SIGN_IDENTITY:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"
SKIP_NOTARIZE="${SKIP_NOTARIZE:-0}"

VERSION="$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')"
DMG_PATH="$DIST_DIR/$APP_NAME-$VERSION.dmg"

echo "==> Packaging $APP_NAME $VERSION"

# 1. Build a universal (arm64 + x86_64) release binary.
echo "==> Building release binaries"
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

echo "==> Assembling $APP_NAME.app"
rm -rf "$DIST_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
lipo -create \
    "target/aarch64-apple-darwin/release/$BIN_NAME" \
    "target/x86_64-apple-darwin/release/$BIN_NAME" \
    -output "$APP_DIR/Contents/MacOS/$BIN_NAME"
strip "$APP_DIR/Contents/MacOS/$BIN_NAME"

# 2. Icon + Info.plist
"$SCRIPT_DIR/make-icns.sh" "$REPO_ROOT/assets/icon.png" "$APP_DIR/Contents/Resources/$BIN_NAME.icns"
sed "s/__VERSION__/$VERSION/g" "$SCRIPT_DIR/Info.plist" > "$APP_DIR/Contents/Info.plist"

# 3. Sign the app.
if [[ -n "$SIGN_IDENTITY" ]]; then
    echo "==> Signing app with: $SIGN_IDENTITY"
    codesign --force --options runtime --timestamp \
        --entitlements "$SCRIPT_DIR/sutra.entitlements" \
        --sign "$SIGN_IDENTITY" "$APP_DIR"
    codesign --verify --strict --verbose=2 "$APP_DIR"
else
    echo "==> No SIGN_IDENTITY — ad-hoc signing (LOCAL TESTING ONLY, not distributable)"
    codesign --force --deep --sign - "$APP_DIR"
fi

# 4. Build the DMG (app + /Applications symlink).
build_dmg() {
    local staging="$DIST_DIR/dmg-staging"
    rm -rf "$staging"; mkdir -p "$staging"
    cp -R "$APP_DIR" "$staging/"
    ln -s /Applications "$staging/Applications"
    rm -f "$DMG_PATH"
    hdiutil create -volname "$APP_NAME $VERSION" -srcfolder "$staging" \
        -ov -format UDZO "$DMG_PATH" >/dev/null
    rm -rf "$staging"
}
echo "==> Building DMG"
build_dmg
[[ -n "$SIGN_IDENTITY" ]] && codesign --force --sign "$SIGN_IDENTITY" "$DMG_PATH"

# 5. Notarize + staple (requires Developer ID signing).
if [[ -n "$SIGN_IDENTITY" && "$SKIP_NOTARIZE" != "1" ]]; then
    if [[ -z "$NOTARY_PROFILE" ]]; then
        echo "!! NOTARY_PROFILE empty — skipping notarization."
        echo "   Set it up with: xcrun notarytool store-credentials (see README)."
    else
        echo "==> Submitting to notary service (profile: $NOTARY_PROFILE)"
        xcrun notarytool submit "$DMG_PATH" --keychain-profile "$NOTARY_PROFILE" --wait
        echo "==> Stapling app and rebuilding stapled DMG"
        xcrun stapler staple "$APP_DIR"
        build_dmg
        codesign --force --sign "$SIGN_IDENTITY" "$DMG_PATH"
        xcrun stapler staple "$DMG_PATH"
        echo "==> Gatekeeper check"
        spctl -a -t exec -vv "$APP_DIR" || true
    fi
fi

echo ""
echo "Done."
echo "  App: $APP_DIR"
echo "  DMG: $DMG_PATH"
if [[ -z "$SIGN_IDENTITY" ]]; then
    echo ""
    echo "NOTE: ad-hoc signed only. For distribution, set SIGN_IDENTITY + NOTARY_PROFILE."
fi
