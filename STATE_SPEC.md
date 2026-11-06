# `.dev-runner/` State Specification

This document defines the file format and conventions for `~/.dev-runner/`, a well-known state folder used by environment runners and status dashboards like **sutra**, and for the per-project `.dev-runner/` folder that a project can keep instead (see [Per-project state folder](#per-project-state-folder)).

## Overview

`~/.dev-runner/` is a flat directory containing two kinds of files:

1. **Meta files** — one per environment, describing the running instance
2. **Status files** — one per unit within an environment, describing its current state

No locking, coordination, or daemon is required. Writers create/update files atomically; readers scan the directory and parse what they find.

## Directory

```
~/.dev-runner/
```

Created by the environment runner if it does not exist. Sutra only reads it: no folder means no environment has registered.

## Meta Files

### Path

```
~/.dev-runner/<id>
```

`<id>` is a hex string that uniquely identifies an environment instance. Typically a hash derived from the project directory path. Lowercase `[0-9a-f]+` is the convention; the parser accepts uppercase hex too but writers should stick to lowercase.

A valid meta filename:
- Contains only ASCII hex digits (`[0-9a-fA-F]+`; lowercase by convention)
- Does not start with `.`
- Does not contain `.`

### Format

Plain text, one `KEY=VALUE` pair per line. No quoting, no escaping, no comments. Unrecognized keys are ignored.

#### Required keys

| Key       | Type   | Description |
|-----------|--------|-------------|
| `DIR`     | path   | Absolute path to the project directory |
| `PID`     | u32    | Process ID of the environment's main process |

#### Optional keys

| Key       | Type   | Description |
|-----------|--------|-------------|
| `STARTED` | u64    | Unix epoch timestamp (seconds) when the environment was started |
| `*_PORT`  | u16    | Any key ending in `_PORT` declares a port. The prefix (lowercased, with `_PORT` stripped) is matched against unit names. |

#### Port matching

A key like `SERVER_PORT=3000` maps to unit name `server` (lowercase prefix). Multiple port keys are supported:

```
SERVER_PORT=3000
VITE_PORT=5173
METRO_PORT=8081
```

#### Example

```
DIR=/Users/you/code/myproject
PID=48291
STARTED=1700000000
SERVER_PORT=3000
VITE_PORT=5173
```

### Lifecycle

- **Created** by the environment runner when an environment starts
- **Deleted** by the environment runner when an environment stops (along with all associated status files)
- **Read** by sutra on each refresh or filesystem event
- **Liveness**: sutra checks `kill(PID, 0)` to determine if the environment is still alive. `ESRCH` means the process is gone; `EPERM` means it exists but sutra may not signal it (as inside the App Sandbox), so it counts as alive. PID `0` is never alive. A meta file with a dead PID is shown as inactive but not automatically removed.

## Status Files

### Path (two conventions)

```
~/.dev-runner/<id>.<unit_name>.status       # current convention
~/.dev-runner/.<id>.<unit_name>.status      # legacy convention (leading dot)
```

Both are recognized by sutra. New writers should use the current convention (no leading dot).

`<unit_name>` is a lowercase identifier for the unit (e.g., `server`, `vite`, `wasm`, `metro`, `mobile`). It must not contain `.` characters.

### Format

Single line of text:

```
<state>[: <detail>]
```

The state is a lowercase keyword. The optional detail (after `:`) is freeform text providing additional context. The canonical separator is `": "` (colon then space) for readability; the parser accepts `:` with or without surrounding whitespace and trims it.

### Well-known states

| State      | Meaning | Lozenge | Sound     |
|------------|---------|---------|-----------|
| `starting` | Unit is initializing | blue | Submarine |
| `building` | Unit is compiling/bundling | blue | Submarine |
| `running`  | Unit process is active | green | Ping      |
| `ready`    | Unit is serving/accepting connections | green | Ping      |
| `failed`   | Unit crashed or build failed | red | Basso     |
| `stopped`  | Unit was intentionally stopped | grey | —         |

Sutra's window shows each unit as a lozenge: a capsule holding the unit's name and port, coloured by its state as above. Any other string is accepted as `Other` and shown grey, its word in the lozenge's tooltip. An empty status file is treated as `None` and shown grey, its tooltip saying `no status`. Once the environment's `PID` is dead, every unit but a failed one is shown grey, since what it last said is no longer news.

The detail of a unit that is starting, building or failed shows beside its lozenge; every unit's detail is in its tooltip. Clicking the lozenge of a unit with a port opens `http://localhost:<port>`.

### Detail examples

```
building: Compiling Rust bindings
building: wasm-pack
failed: exit code 1
failed: iOS build failed
building: xcode build running
```

### Writing status files

Writers should update status files **atomically** — write to the file directly (single-line content means partial writes are unlikely, but `echo "state" > file` is sufficient for this use case).

Status files should be written **before** any corresponding notification or speech call, so that dashboard UIs can react to the transition promptly.

### Cleanup

When an environment stops, **all** of its status files must be removed:

```bash
rm -f "$REGISTRY_DIR/$KEY".*.status
rm -f "$REGISTRY_DIR/.$KEY".*.status   # legacy cleanup
```

This should happen in stop handlers, cleanup traps, and unregister functions.

## Per-project state folder

A project can publish its environment inside its own folder instead of in `~/.dev-runner/`. This suits one environment per project folder, such as one per git worktree: the folder already says which project it describes, so no hash is needed.

### Layout

```
<project>/.dev-runner/
  meta              # KEY=VALUE, same format as a global meta file
  server.status     # "running"
  vite.status       # "ready"
```

### Meta file

`<project>/.dev-runner/meta` uses the same `KEY=VALUE` format as a global meta file.

| Key       | Required | Description |
|-----------|----------|-------------|
| `PID`     | yes      | Process ID of the environment's main process |
| `DIR`     | no       | Ignored: the project folder itself is the directory |
| `STARTED` | no       | As for a global meta file |
| `*_PORT`  | no       | As for a global meta file |

A `meta` file without a valid `PID` is treated as absent.

### Status files

`<project>/.dev-runner/<unit_name>.status`, with the same single-line format as global status files and no hash prefix. `<unit_name>` must not contain `.`; other files in the folder (for example `server.status.tmp` during an atomic write) are ignored.

A folder with status files but no `meta` is still shown as an environment, with no process and therefore inactive, so a dev script that writes status before it writes `meta` is visible. A folder with neither is no environment.

### Keeping it out of git

The folder holds runtime state, so writers exclude it locally in the repository's `info/exclude` file rather than in a committed `.gitignore`. In a linked worktree `.git` is a file, so find the exclude file with `git rev-parse --git-path info/exclude`; all worktrees of a repository share it.

### Cleanup

When the environment stops, remove `meta` and every `*.status` file. An empty `.dev-runner/` folder is no environment, so it may stay.

### Merging with `~/.dev-runner/`

Sutra's worktree map shows one environment per project folder. It identifies each environment by its canonical project path (symlinks resolved), so a unit keeps the same identity whichever convention publishes it. A global meta file belongs to the folder its `DIR` names. When several environments describe the same folder, sutra ranks them:

1. An environment whose `PID` is dead ranks below every other: a `meta` left behind by a killed script is stale, however specific its folder. A per-project folder with status files but no `meta` claims no process, so it is never dead.
2. Then the per-project folder ranks above a global meta file.
3. Then a later `STARTED` ranks above an earlier one.

The merged environment takes:

- `PID`, `STARTED`, the ports and liveness from the highest-ranked environment that has a valid `PID`.
- Its units united by name; for a unit several describe, the status of the highest-ranked one wins. While any of the environments is live, the units that only dead ones describe are left out.

## State Transitions & Notifications

Sutra detects state transitions by comparing current state against the previously observed state for each `(environment_id, unit_name)` pair.

### First load

On startup, sutra snapshots all current states **without** firing any notifications. This prevents a storm of sounds when the dashboard launches into an already-running environment.

### Transition rules

A notification (sound + speech + macOS banner) fires when:
- The state variant changes (e.g., `building` → `ready`)
- A new unit appears for the first time (after first load)

A notification does **not** fire when:
- The state variant is the same (even if detail text changes)
- The new state is `stopped`, `Other` or `None`

### Sound mapping

| Transition target | System sound |
|-------------------|-------------|
| `starting`, `building` | `/System/Library/Sounds/Submarine.aiff` |
| `running`, `ready` | `/System/Library/Sounds/Ping.aiff` |
| `failed` | `/System/Library/Sounds/Basso.aiff` |
| `stopped`, `None`, `Other` | (none: no sound, speech or banner) |

### Speech

After the system sound, sutra speaks `"<unit_name> <state>"` using the macOS system voice (AppKit NSSpeechSynthesizer, same voice as `say`).

### macOS notifications

A silent banner (the sound above is the only one) is sent via Notification Center with:
- **Title**: `Sutra · <unit_name>`
- **Body**: the state word (e.g., `ready`, `failed`), without the detail

### Muting

Notifications are independently controllable:
- **Global sound mute**: silences all sounds and speech
- **Per-unit sound mute**: silences sounds/speech for a specific unit
- **Global notifications off**: suppresses macOS banners
- **Per-unit notifications off**: suppresses banners for a specific unit

Sound mute and notification suppression are independent — you can mute audio while still receiving banners, or vice versa.

## Example Layout

```
~/.dev-runner/
  df79fed95eebc05d                          # meta: DIR, PID, ports
  df79fed95eebc05d.server.status            # "running"
  df79fed95eebc05d.vite.status              # "ready"
  df79fed95eebc05d.wasm.status              # "building: wasm-pack"
  df79fed95eebc05d.metro.status             # "running"
  df79fed95eebc05d.mobile.status            # "building: xcode build running"
```

## Compatibility Notes

- sutra recognizes both dotted (`.hash.unit.status`) and non-dotted (`hash.unit.status`) conventions
- Meta filenames must be pure hex — any file containing `.` is treated as a status file or ignored
- Meta filename hex digits are case-permissive (parser uses `is_ascii_hexdigit`); writers should use lowercase by convention
- Keys in meta files are case-sensitive (`DIR`, not `dir`)
- State strings in status files are case-sensitive (`ready`, not `Ready`)
- Detail separator is `:` with optional surrounding whitespace (the parser trims); the canonical form `<state>: <detail>` is recommended for readability
- Neither state folder is configurable: sutra reads the global registry at `~/.dev-runner/` and a project's own folder at `<project>/.dev-runner/` (hardcoded conventions)
