# Sutra — Resume for Fresh Agent

## What is this?

Sutra is a macOS status dashboard that monitors the well-known state folder `~/.dev-runner/`. It displays each environment's units in a GUI (iced) or TUI (ratatui), with system sounds, speech, and macOS notifications on state transitions.

The repo directory is `/Users/daniel/code/sutra`, remote is `git@github.com:synestheticsystems/sutra.git`.

## Current state (2026-02-15)

**Compiles clean** — zero warnings, `cargo clippy -- -D warnings` passes on macOS. Cross-platform compilation supported (Linux, macOS).

**Published** to GitHub at `origin/main`. `CARGO_REGISTRY_TOKEN` is configured — CI auto-publishes to crates.io on version bumps.

### What was done this session

1. **Cross-platform compilation** (latest): Moved `rodio`, `tts`, `mac-notification-sys` behind `target.'cfg(target_os = "macos")'.dependencies`. Cfg-gated all macOS-specific code in `notifications.rs`. Audio thread still runs on all platforms but only plays sounds/speaks on macOS. Publish workflow switched to `ubuntu-latest`. GitHub issue #1 created for Linux/Windows GUI support.

2. **CRITICAL bug fixed**: GUI crashed on every start due to `text("").size(0)` in gui.rs (port cell for environments without ports). `cosmic-text` panics on zero line height, which hit the ObjC boundary causing SIGABRT. Fixed by removing `.size(0)`.

2. **Code quality fixes** (all from the review list):
   - `state_variant_eq` now compares `Other` inner strings (was using `mem::discriminant` only)
   - All `pid as i32` casts replaced with `i32::try_from()` (model.rs, gui.rs, tui.rs)
   - `registry_dir()` renamed to `state_dir()`, now returns `Option<PathBuf>` instead of panicking
   - `UnitStatus::parse` returns `UnitStatus` directly (was misleadingly `Option`)
   - `muted_units`/`notifications_off_units` changed from `HashSet<(String, String)>` to `HashSet<String>` with `unit_key()` helper
   - Toggle functions take `&str` instead of `String`

3. **UX improvements**:
   - Light-mode yellow changed to `#9a6700` (WCAG compliant)
   - Empty state shows "Watching ~/.dev-runner/ for environments" subtitle
   - Cmd+Q keyboard shortcut via `iced::keyboard::on_key_press`
   - Notification batching: one sound (highest priority) + one combined speech utterance
   - Terminate button changed from broken Unicode `⏹` to SVG `ICON_SQUARE`
   - SIGHUP changed to SIGTERM for environment termination
   - Tooltips on all interactive GUI elements (toolbar icons, per-unit icons, terminate, open browser)
   - `WatchEvent` simplified to unit variant (payload was never used)

4. **CI/Publishing fixes**:
   - `ci.yml`: Added `components: rustfmt, clippy` to toolchain step
   - `publish.yml`: Added `published` output gate (was always triggering release), removed `--allow-dirty`, switched to `ubuntu-latest`, added concurrency block
   - `release.yml`: Removed `push: tags` trigger (prevented double-trigger), renamed assets to `sutra-macos-{arch}` (was name collision), upgraded to `softprops/action-gh-release@v2`
   - `Cargo.toml`: Added `readme`, `rust-version = "1.85"`, `exclude = ["resume.md", "create-test-envs.sh"]`

5. **Cleanup**:
   - Dead None/Other transition check removed from notifications.rs (was unreachable)
   - `set_application()` moved from per-notification to `Notifier::new()`
   - `&PathBuf` parameter changed to `&Path` in watcher.rs
   - Added `Default` impl for `Notifier` (clippy)
   - README.md fully rewritten: all GUI/TUI controls documented, correct indicator symbols, --foreground documented, batching mentioned
   - STATE_SPEC.md indicator symbols fixed (◌ for starting, ◑ for building)

### Architecture

```
src/
  main.rs           # CLI: `sutra` defaults to `sutra mon` (GUI), `sutra mon --tui` for TUI
                    # GUI mode backgrounds by default (re-execs with --foreground)
  lib.rs            # Feature-gated modules (gui, tui)
  model.rs          # Environment, UnitStatus, State enum, file parsing, state_dir()
  watcher.rs        # FSEvents watcher on ~/.dev-runner/, emits WatchEvent
  notifications.rs  # Notifier: sound (rodio), speech (tts AppKit), macOS notifications
                    # Batched audio, transition detection, global + per-unit mute/notification toggles
                    # All macOS-specific code cfg-gated; compiles cross-platform
  gui.rs            # iced GUI: cards, SVG toolbar icons, tooltips, per-unit toggles, hover, Cmd+Q
                    # Dock icon via objc2 NSApplication.setApplicationIconImage
  tui.rs            # ratatui TUI: mouse support, unit selection, auto-scroll
                    # Keys: j/k select, m/n global toggles, M/N per-unit, o open, x terminate, q quit
assets/
  icon.png              # 256x256 app icon (dark purple bg, green eye)
  icon-transparent.png  # 256x256 transparent version (used in README)
```

### Key design decisions

- `state_dir()` returns `Option<PathBuf>` — callers handle missing home dir gracefully
- `unit_key()` uses `"\x00"` separator for HashSet keys (avoids tuple of two Strings)
- Notification batching collects all transitions, picks highest-priority sound (Basso > Ping > Submarine), joins speech into one utterance
- `WatchEvent` is a unit enum variant — the payload was never used, both GUI and TUI do full `load_all()` on any FS event
- `set_application()` called once in `Notifier::new()`, not per-notification

### Dependencies

```toml
# Cross-platform
notify = "7"                          # filesystem watcher
dirs = "6"                            # home dir
nix = "0.29"                          # PID liveness + SIGTERM (Unix only)
clap = "4"                            # CLI
ratatui = "0.29" + crossterm = "0.28" # TUI (optional)
iced = "0.13" (tokio, svg, image)     # GUI (optional)

# macOS only (cfg-gated)
rodio = "0.21"                        # system sounds (.aiff playback)
tts = "0.26"                          # speech (AppKit backend)
mac-notification-sys = "0.6"          # notification center
objc2 = "0.5"                         # macOS dock icon
objc2-foundation = "0.2"              # NSData
objc2-app-kit = "0.2"                 # NSApplication, NSImage
```

### CI / Publishing

- `.github/workflows/ci.yml` — test + fmt + clippy (with components), matrix build (macOS x86_64 + aarch64)
- `.github/workflows/publish.yml` — auto-publish to crates.io on version bump, `published` output gates release, ubuntu-latest (cross-platform build), concurrency guard
- `.github/workflows/release.yml` — workflow_call only (no push:tags), builds stripped binaries with distinct asset names, softprops/action-gh-release@v2
- `LICENSE-MIT` + `LICENSE-APACHE` — dual licensed
- `resume.md` and `create-test-envs.sh` excluded from crate tarball
- Remote: `git@github.com:synestheticsystems/sutra.git` (branch `main`)

### Test data

- `create-test-envs.sh` — creates 15 fake environments (53 units) in `~/.dev-runner/`
  - Run: `bash create-test-envs.sh`
  - Clean: `bash create-test-envs.sh --clean`
  - Uses hex IDs `a0a0a0a0a0a00001` through `a0a0a0a0a0a0000f`, PIDs 99999-99985 (dead)

## Remaining known issues

### Still TODO (not blocking publish)

1. **Duplicated App struct** between GUI and TUI — both define `App` with overlapping fields (`envs`, `notifier`). Could extract shared `AppState`.
2. **TUI hardcodes white text** — no theme support, invisible on light terminal backgrounds.
3. **Speech on by default** with no independent toggle — can be surprising/annoying.
4. **Should follow system dark/light appearance** — currently hardcoded to light mode.
5. **Magic number `CHAR_W = 7.2`** in gui.rs for column width estimation — fragile across display scales.
6. **`state_color` duplicated** between GUI and TUI with subtly inconsistent Stopped color.

### Immediate next steps

1. GitHub issue #1 tracks Linux/Windows GUI support (notifications, TTS, sounds).

## Commands

```bash
cargo check                                        # verify all features
cargo clippy -- -D warnings                        # lint check
cargo run                                          # GUI (backgrounds by default)
cargo run -- mon --foreground                      # GUI, attached to terminal
cargo run -- mon --tui                             # TUI
cargo run --no-default-features --features tui     # TUI-only binary
cargo publish --dry-run                            # test crate packaging
cargo publish                                      # publish to crates.io (first time: creates crate)
bash create-test-envs.sh                           # create 15 fake environments
bash create-test-envs.sh --clean                   # remove fake environments
```

## Related files

- `STATE_SPEC.md` — full spec for the `~/.dev-runner/` file format
- `README.md` — user-facing docs with icon, install, usage, full control reference
- `create-test-envs.sh` — test data generator
