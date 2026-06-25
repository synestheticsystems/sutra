<p align="center">
  <img src="assets/icon-transparent.png" width="128" alt="sutra">
</p>

# sutra

[![CI](https://github.com/synestheticsystems/sutra/actions/workflows/ci.yml/badge.svg)](https://github.com/synestheticsystems/sutra/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/sutra.svg)](https://crates.io/crates/sutra)

A dev environment status dashboard. Monitors a well-known state folder for environment meta and per-unit status files, rendering everything in a native GUI (iced) or TUI (ratatui). On macOS, state transitions trigger system sounds, speech, and native notifications.

## Install

```sh
cargo install sutra
```

## Usage

```sh
sutra                       # launch GUI (backgrounds by default)
sutra mon --foreground      # GUI, attached to terminal
sutra mon --tui             # terminal UI
```

Both interfaces support per-unit and global toggles for sound and notification muting, environment termination, and opening browser ports.

## Platform support

| Feature | macOS | Linux |
|---|---|---|
| GUI (iced) | yes | yes |
| TUI (ratatui) | yes | yes |
| System sounds | yes | -- |
| Speech (TTS) | yes | -- |
| Native notifications | yes | -- |

Linux/Windows audio and notification support is tracked in [#1](https://github.com/synestheticsystems/sutra/issues/1).

## Features

- `gui` -- iced-based native window (default)
- `tui` -- ratatui terminal interface (default)

Build with only one:

```sh
cargo build --no-default-features --features tui
```

## License

MIT OR Apache-2.0
