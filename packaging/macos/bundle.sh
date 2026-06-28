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
#   NOTARY_PROFILE  notarytool keychain profile (local flow; see README.md).
#   SKIP_NOTARIZE   Set to 1 to sign with Developer ID but skip notarization.
#
# Notarization credentials — choose ONE of:
#   (a) Keychain profile (local dev):
#         NOTARY_PROFILE="sutra-notary"
#   (b) App Store Connect API key (CI):
#         NOTARY_KEY_PATH   path to the .p8 private key file
#         NOTARY_KEY_ID     the key ID (e.g. ABCDE12345)
#         NOTARY_ISSUER     the issuer UUID
#   If the API-key trio is set it takes precedence over NOTARY_PROFILE.
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
NOTARY_KEY_PATH="${NOTARY_KEY_PATH:-}"
NOTARY_KEY_ID="${NOTARY_KEY_ID:-}"
NOTARY_ISSUER="${NOTARY_ISSUER:-}"

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
# Two credential sources are supported. The App Store Connect API key trio
# (NOTARY_KEY_PATH + NOTARY_KEY_ID + NOTARY_ISSUER) is used by CI and takes
# precedence; a notarytool keychain profile (NOTARY_PROFILE) is used locally.
if [[ -n "$SIGN_IDENTITY" && "$SKIP_NOTARIZE" != "1" ]]; then
    NOTARIZED=0
    if [[ -n "$NOTARY_KEY_PATH" && -n "$NOTARY_KEY_ID" && -n "$NOTARY_ISSUER" ]]; then
        echo "==> Submitting to notary service (App Store Connect API key: $NOTARY_KEY_ID)"
        xcrun notarytool submit "$DMG_PATH" \
            --key "$NOTARY_KEY_PATH" \
            --key-id "$NOTARY_KEY_ID" \
            --issuer "$NOTARY_ISSUER" \
            --wait
        NOTARIZED=1
    elif [[ -n "$NOTARY_PROFILE" ]]; then
        echo "==> Submitting to notary service (profile: $NOTARY_PROFILE)"
        xcrun notarytool submit "$DMG_PATH" --keychain-profile "$NOTARY_PROFILE" --wait
        NOTARIZED=1
    else
        echo "!! No notarization credentials — skipping notarization."
        echo "   Local:  xcrun notarytool store-credentials (see README), then NOTARY_PROFILE=..."
        echo "   CI:     set NOTARY_KEY_PATH + NOTARY_KEY_ID + NOTARY_ISSUER."
    fi

    if [[ "$NOTARIZED" == "1" ]]; then
        # Staple the ticket to the DMG we just submitted. Do NOT rebuild the DMG
        # afterwards: rebuilding changes its hash, and the notarization ticket is
        # keyed to the submitted hash, so stapling a rebuilt DMG fails with
        # "Could not find base64 encoded ticket".
        echo "==> Stapling notarization ticket"
        xcrun stapler staple "$DMG_PATH"
        # The app is notarized as nested code in that submission, so its ticket is
        # on Apple's CDN and can also be stapled to the standalone .app.
        xcrun stapler staple "$APP_DIR"
        echo "==> Gatekeeper check"
        spctl -a -t exec -vv "$APP_DIR" || true
        xcrun stapler validate "$DMG_PATH" || true
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
