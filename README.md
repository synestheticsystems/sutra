<p align="center">
  <img src="assets/icon-transparent.png" width="128" alt="Sutra">
</p>

# Sutra

[![CI](https://github.com/synestheticsystems/sutra/actions/workflows/ci.yml/badge.svg)](https://github.com/synestheticsystems/sutra/actions/workflows/ci.yml)

You have fourteen worktrees, three coding agents, two dev servers and a `target/` folder the size of a small moon. Quick: which agent is waiting on you?

Sutra knows. It's a Mac app that maps every git worktree and project folder in your home folder, and shows you, for each one:

- **The agents at work in it.** Claude Code and Codex sessions, busy or waiting on you. A waiting agent's card throbs amber, the Dock badge counts them, and if the wait drags on, Sutra says so out loud. Click a session the Claude app runs and you're right back in it.
- **Its dev environments.** Whatever your dev scripts publish in `~/.dev-runner/` or the project's own `.dev-runner/`. Each unit gets a coloured lozenge, each port is one click from the browser, and you get a sound, a spoken word and a banner when something starts, comes up or falls over.
- **Its disk use.** What the folder takes, and how much of that is build folders. (It's the build folders. It's always the build folders.)

Then it cleans up. Sutra deletes a Rust build folder the way `cargo clean` would, and never while a build is still writing to it. Tell it how much free space to keep, and when the disk runs short it clears out the build folders that have been quiet the longest, all by itself, leaving alone anything an agent or a dev server is using. Your `node_modules` stay exactly where you left them. We would never.

The folders that matter get a card: anything with an agent, a dev environment, recent activity or a hefty build folder. The quiet ones fold away, one click from view. We look in Documents, because Codex keeps its worktrees there. Desktop and Downloads stay out of it until you invite them in from Settings. We have manners.

## Install

With Homebrew:

```sh
brew install --cask synestheticsystems/tap/sutra
```

Or grab the DMG from the [latest release](https://github.com/synestheticsystems/sutra/releases/latest) and drag Sutra to Applications. It's signed and notarized, so it opens like any other app.

Sutra needs macOS 14 Sonoma or later, on Apple silicon or Intel.

## Make your dev script talk

Sutra shows a dev environment once your dev script says it has one: a few small files, no SDK, no daemon. [docs/INTEGRATION.md](docs/INTEGRATION.md) is the recipe (written so you can hand it straight to your coding agent), and [STATE_SPEC.md](STATE_SPEC.md) is the fine print.

## Build from source

You'll need Xcode (we build releases with 26.6) and Rust, through [rustup](https://rustup.rs).

```sh
macos/build.sh               # builds the app for this Mac, and prints where it put it
packaging/macos/bundle.sh    # dist/Sutra.app and a DMG, for Apple silicon and Intel, signed ad hoc
```

[packaging/macos/README.md](packaging/macos/README.md) has the rest: signing, notarizing, the sandboxed build, and how a release happens.

## License

MIT OR Apache-2.0
