# macOS packaging

Builds `Sutra.app` and its `.dmg` for direct distribution (the GitHub release
and the Homebrew cask), a sandboxed dev bundle that stands in for the Mac App
Store edition, and a recording build for the store's preview video.

## Two editions

- The **Developer ID edition**, which `bundle.sh` builds, runs outside the App
  Sandbox. It reads the home folder with no grant to ask for (Documents from
  the start, Desktop and Downloads once Settings turns them on, and macOS asks
  once for each), and a card's menu can terminate a dev environment, which sends
  `SIGTERM` to a supervisor PID Sutra did not start. Developer ID signing and
  notarization get it past Gatekeeper.
- The **Mac App Store edition** will run inside the App Sandbox, which allows
  neither: it reads only the folder the user grants in an open panel, and it
  cannot signal other processes, so it offers no Terminate. It has no release
  yet; `sandbox-dev.sh` builds a local stand-in (see below).

## Build

Requirements: Xcode (CI builds with 26.6) and Rust through rustup.

```sh
./packaging/macos/bundle.sh
```

This makes an ad-hoc-signed build that runs on this Mac but is not
distributable:

1. `macos/build.sh --universal` builds the Rust core's static library for
   `aarch64-apple-darwin` and `x86_64-apple-darwin` with
   `MACOSX_DEPLOYMENT_TARGET=14.0` (adding those rustup targets if they are
   missing), joins the two with `lipo`, generates the Swift bindings with
   UniFFI, and wraps the library in an XCFramework. It then builds the app
   once for `arm64` and once for `x86_64` and joins the two executables with
   `lipo`: Xcode 26's SwiftPM hands `swift build --arch arm64 --arch x86_64` to
   the Xcode build system, which fails to link the XCFramework's library.
   Without `--universal` it builds for this Mac's architecture only: the quick
   local build.
2. `make-app.sh` wraps the executable in `Sutra.app`, with `Info.plist` and the
   icon (`make-icns.sh`).
3. `bundle.sh` strips the executable, signs the app (Developer ID with the
   hardened runtime and `sutra.entitlements` when `SIGN_IDENTITY` is set, ad hoc
   otherwise), builds the DMG, and, given notary credentials, notarizes and
   staples both.

Output lands in `dist/`; other files there stay:
- `dist/Sutra.app`
- `dist/Sutra-<version>.dmg`

## Notes

- The app is **universal** (arm64 + x86_64).
- Version is read from `Cargo.toml` and injected into `Info.plist`.
- The minimum macOS is 14, and three places must agree on it:
  `LSMinimumSystemVersion` in `Info.plist`, `platforms` in
  `macos/Package.swift`, and `MACOSX_DEPLOYMENT_TARGET` in `macos/build.sh`.
  The Homebrew cask's `depends_on macos:` should say the same.
- Bundle identifier: `systems.synesthetic.sutra`. Notification Center knows
  Sutra by it, for its banners and for the user's notification settings. The
  app posts its banners itself, through `UNUserNotificationCenter`
  (`macos/Sources/SutraWindow/Platform/NotificationBanners.swift`), which serves
  only a bundled app, so a bare executable from `macos/build.sh` posts none.
  The app's loggers also name it as their subsystem (`NotificationBanners.swift`
  and `Native.swift`).
- `sutra.entitlements` is intentionally an empty dict: the app is unsandboxed
  and needs no Hardened-Runtime-gated capabilities. (Note: codesign's
  entitlements parser rejects XML comments, so keep the file comment-free.) If
  notarization or launch ever fails with a library-validation error, add
  `com.apple.security.cs.disable-library-validation` and re-sign.
- The icon is generated from `assets/icon.png`. That master is currently
  **256×256**, so large Retina icon sizes are upscaled and look soft — drop in a
  **1024×1024** `assets/icon.png` for crisp icons.
- Notarization runs **twice**: once on the DMG, then — after stapling the app
  and rebuilding the DMG around it — once more on the final DMG. This embeds a
  stapled ticket in the app *inside* the DMG (offline-proof), which a single
  pass can't do (rebuilding to include the stapled app changes the DMG's hash).
- `dist/` is gitignored, as are `macos/build/` and `macos/.build/`.

## Sandboxed dev bundle (testing the folder grant)

The Mac App Store edition runs in the App Sandbox, where Sutra may read only the
folder the user picks in an open panel, and keeps that choice as a
security-scoped bookmark so it asks only once. `sandbox-dev.sh` builds a local
bundle that runs under the same sandbox, so the grant can be exercised exactly
as the store build will meet it:

```sh
./packaging/macos/sandbox-dev.sh
open -n "dist/Sutra Sandbox.app"
```

- The output, `dist/Sutra Sandbox.app`, is built by `macos/build.sh` for the
  current architecture only, left unstripped, and ad-hoc signed, so it runs on
  this Mac and is not distributable. The script replaces only that bundle; a
  `dist/Sutra.app` from `bundle.sh` stays.
- Its bundle identifier, `systems.synesthetic.sutra.sandbox`, gives it its own
  container and preferences, so it never touches an installed `Sutra.app`.
- `sutra-sandbox.entitlements` holds the three entitlements the store build
  needs: `com.apple.security.app-sandbox` (the sandbox itself),
  `com.apple.security.files.user-selected.read-write` (access to the folder
  picked in the panel; `src/map/grant.rs` explains why the grant is
  read-write),
  and `com.apple.security.files.bookmarks.app-scope` (saving the grant as a
  bookmark and resolving it on the next launch). The script fails if the
  signature carries anything other than exactly these. Keep the file
  comment-free, like `sutra.entitlements`.
- The first launch opens the window on the welcome screen. Past it, the window
  explains why the home folder matters and offers Choose Folder…, which shows
  the panel. Choosing a folder saves the grant, and later launches restore it
  without asking. Cancelling saves nothing, so the window asks again. The folder
  icon in the window's toolbar chooses another folder later.
- To be asked again after choosing, delete the saved grant and relaunch:
  ```sh
  rm ~/Library/Containers/systems.synesthetic.sutra.sandbox/Data/Library/Application\ Support/Sutra/root.bookmark
  ```

## Recording build (the store's preview video)

The Mac App Store's preview shows the app at work, and nobody's real machine
belongs in it. `recording.sh` builds `Sutra Recording.app`: the app's own
window and welcome screen over invented work.

```sh
./packaging/macos/recording.sh
open -n "dist/Sutra Recording.app"
```

- It is a build of its own (`macos/Sources/SutraRecording`) with no live core
  in it, so nothing scans a folder, reads an agent's session or a process, or
  posts a banner. Its window reads a script (`ScriptedCore.swift`) that plays
  invented projects in folders under `/Demo`, a home no Mac has
  (`Script/Scenario.swift`).
- It opens on the welcome screen at every launch, and the robots dance the same
  calls each time. The script waits behind the screen and plays from the click
  on Let's untangle this: 6 seconds in, the `fix/layout` worktree's agent stops
  to ask permission and its card throbs; at 20 it works on; at 24 the
  `feature/search` worktree's agent and its two subagents finish.
- For another take, quit and launch it again, or choose Help > Welcome to Robo
  Sutra, which brings the screen back and rewinds the script.
- Folding a repository, the sort and the quiet projects' row change the list as
  the app would. The rest is left undone, and no button, menu or link reaches
  the Mac: a stand-in for macOS's services opens, copies and changes nothing.
- Its bundle identifier, `systems.synesthetic.sutra.recording`, and the App
  Sandbox (`sutra-recording.entitlements`, the sandbox alone) give it a
  container and preferences of its own, so it reads and writes none of
  Sutra's.

## Releases (CI)

Releases build in CI, never on a workstation. A version bump on `main` releases
that version:

1. A push to `main` that changes `Cargo.toml` runs **Tag version**
   (`.github/workflows/tag-version.yml`). If the version has no `v<version>`
   tag yet, it tags the pushed commit; otherwise it does nothing.
2. **Release DMG** (`.github/workflows/release-dmg.yml`) runs after Tag version
   succeeds, via `workflow_run` rather than `push: tags`, because a tag pushed
   with `GITHUB_TOKEN` starts no workflow. It releases only the commit the
   version's tag names, so a push that changed `Cargo.toml` but not the version
   releases nothing, and a version that shipped is never rebuilt.
3. Release DMG runs `bundle.sh` on `macos-26` with Xcode 26.6, signed with the
   Developer ID certificate and notarized with an App Store Connect API key,
   checks the stapled DMG with Gatekeeper, and attaches it to the GitHub release
   `Release v<version>`, creating the release.
4. It then sets `version` and `sha256` in `Casks/sutra.rb` of
   `synestheticsystems/homebrew-tap`, pushing with a deploy key. The cask's
   other lines (`depends_on macos:`, `desc`, `zap`) are edited by hand in the tap.

Until the secrets below are configured, Release DMG no-ops with a `::warning`
(no red X). CI's build job (`.github/workflows/ci.yml`) runs the same
`bundle.sh`, ad hoc, on the same runner and Xcode, for every push and pull
request to `main`.

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
not an app-specific password. The workflow file header lists the secrets.
Manual re-run: **Actions → Release DMG → Run workflow → enter the tag**.

## Signing and notarizing locally (debugging only)

Releases are signed and notarized in CI. To debug signing or notarization on a
Mac, `bundle.sh` takes the same credentials from a keychain:

1. **Apple Developer Program** membership.

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

Then:
```sh
SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
NOTARY_PROFILE="sutra-notary" \
./packaging/macos/bundle.sh
```
