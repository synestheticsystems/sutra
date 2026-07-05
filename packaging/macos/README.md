# macOS packaging (Developer ID distribution)

Builds a signed, notarized `Sutra.app` and `.dmg` for direct distribution
(download / Homebrew cask) — **not** the Mac App Store.

## Why not the App Store?

The Mac App Store requires the App Sandbox. Sutra's core features are
incompatible with it:

- it sends `SIGTERM` to dev-runner supervisor PIDs it did not spawn
  (environment termination), which the sandbox forbids;
- it checks PID liveness via `kill(pid, 0)` on foreign processes;
- it watches the shared `~/.dev-runner/` folder written by other tools.

Developer ID + notarization keeps all of this working while still passing
Gatekeeper.

## One-time setup

1. **Apple Developer Program** membership (you have this).

2. **Create a "Developer ID Application" certificate** and install it in your
   login keychain. Easiest via Xcode:
   *Settings → Accounts → (your team) → Manage Certificates → ＋ → Developer ID Application*.
   Or create it in the Apple Developer portal and double-click the downloaded
   `.cer`.

   Confirm it's installed:
   ```sh
   security find-identity -v -p codesigning
   # → look for: "Developer ID Application: Your Name (TEAMID)"
   ```

3. **Create a notarytool credential profile** (stored in the keychain) using an
   app-specific password from <https://appleid.apple.com>:
   ```sh
   xcrun notarytool store-credentials "sutra-notary" \
     --apple-id "you@example.com" \
     --team-id "TEAMID" \
     --password "app-specific-password"
   ```

## Build

Local test build (ad-hoc signed — runs on your machine, not distributable):
```sh
./packaging/macos/bundle.sh
```

Distributable build (signed + notarized + stapled):
```sh
SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
NOTARY_PROFILE="sutra-notary" \
./packaging/macos/bundle.sh
```

Output lands in `dist/`:
- `dist/Sutra.app`
- `dist/Sutra-<version>.dmg`

## Notes

- The app is a **universal binary** (arm64 + x86_64) via `lipo`.
- Version is read from `Cargo.toml` and injected into `Info.plist`.
- Bundle identifier: `systems.synesthetic.sutra` (also the notification app id
  in `src/notifications.rs`).
- `sutra.entitlements` is intentionally an empty dict: the app is unsandboxed
  and needs no Hardened-Runtime-gated capabilities. (Note: codesign's
  entitlements parser rejects XML comments, so keep the file comment-free.) If
  notarization or launch ever fails with a library-validation error, add
  `com.apple.security.cs.disable-library-validation` and re-sign.
- The icon is generated from `assets/icon.png`. That master is currently
  **256×256**, so large Retina icon sizes are upscaled and look soft — drop in a
  **1024×1024** `assets/icon.png` for crisp icons.
- `dist/` is gitignored.

## Automated releases (CI)

`.github/workflows/release-dmg.yml` runs `bundle.sh` in CI to build, sign,
notarize, staple, and attach the DMG to the release, then bumps the Homebrew
cask in `synestheticsystems/homebrew-tap`. It triggers via `workflow_run` after
the **Publish** workflow succeeds — *not* `release: published`, because releases
created by `GITHUB_TOKEN` don't fire downstream events. Because it runs in its
own workflow run after `cargo publish` + the tag push, a signing/notarization
failure can never break the crates.io publish. Until the secrets below are
configured, the job no-ops with a `::warning` (no red X).

One-time secrets on `synestheticsystems/sutra` (`gh secret set <NAME> -R synestheticsystems/sutra`):

| Secret | What |
|---|---|
| `MACOS_CERT_P12` | base64 of the Developer ID Application cert+key `.p12` |
| `MACOS_CERT_PASSWORD` | the `.p12` export password |
| `MACOS_KEYCHAIN_PASSWORD` | any throwaway string (for the temp CI keychain) |
| `NOTARY_KEY_P8` | base64 of the App Store Connect API key `.p8` |
| `NOTARY_KEY_ID` | the API key ID |
| `NOTARY_ISSUER` | the API issuer UUID |
| `HOMEBREW_TAP_DEPLOY_KEY` | SSH private key of a write-enabled deploy key on `homebrew-tap` (never expires; more scoped than a PAT) |

CI notarizes with an **App Store Connect API key** (`notarytool --key/--key-id/--issuer`),
not an app-specific password. The workflow file header lists exact creation
commands. Manual re-run: **Actions → Release DMG → Run workflow → enter the tag**.
