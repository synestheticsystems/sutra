# Integrating sutra with your dev script

> **Hi, agent.** This doc is written for you. A user has pointed you
> here and asked you to make their project's dev script sutra-compatible.
>
> Your job:
> 1. Find the project's existing dev runner (`dev.sh`, `dev.ts`,
>    `Justfile`, `Makefile`, `package.json` scripts, `bin/dev`, etc.).
>    If none exists, create one.
> 2. Make it satisfy the contract below, applying the patterns in
>    "Recommended pattern" that match the project's stack.
> 3. Verify with the "Checklist" section before declaring done.
>
> Don't rewrite a working dev runner. Add the registry/status hooks
> alongside whatever it already does.

Sutra is a **situational-awareness dashboard** for the dev runners on
a developer's machine. Each compatible dev runner publishes a tiny set
of files into `~/.dev-runner/` advertising itself; sutra reads them
and renders one unified view of every active environment. No daemon,
no IPC, no SDK — just files.

What sutra does:

- **Visualizes the discrete components of each dev runner** — server,
  vite, wasm, mobile, etc. — and the state of each, *as advertised by
  the dev runner itself*. Sutra doesn't measure or infer state; it
  surfaces what the dev script writes.
- **Offers limited control** — essentially just a shutdown button per
  environment. The button sends a single SIGTERM to the supervisor
  PID the dev runner published; your supervisor's trap is what
  actually reaps children, runs cleanup, and removes the registry
  entry. Anything richer (restart, per-unit stop, port reassignment)
  belongs in the dev script's own CLI, not in sutra.

Your dev script owns its lifecycle. Sutra is the dashboard.

For exact file format and edge cases, see [STATE_SPEC.md](../STATE_SPEC.md).

## The contract

Two file shapes live in `~/.dev-runner/`:

**1. A meta file** named after a hex hash of your project path. One per
running environment.

```
~/.dev-runner/<id>
```

Plain `KEY=VALUE`, one per line. Keys:

| Key       | Required | Format                                |
|-----------|----------|---------------------------------------|
| `DIR`     | yes      | absolute path to the project          |
| `PID`     | yes      | supervisor PID (decimal integer)      |
| `STARTED` | no       | Unix epoch seconds (e.g. `1700000000`)|
| `*_PORT`  | no       | port number; prefix maps to unit name lowercased (`SERVER_PORT` → `server`) |

**2. Status files**, one per subprocess (server, build watcher, dev
server, …):

```
~/.dev-runner/<id>.<unit_name>.status
```

(Sutra also tolerates a legacy form with a leading dot
`~/.dev-runner/.<id>.<unit_name>.status` for back-compat. New
integrations should use the no-leading-dot form.)

Single line: `<state>` or `<state>: <detail>`. The canonical states
are `starting`, `building`, `running`, `ready`, `failed`, `stopped`.
Stick to these — anything else is accepted but won't get the right
semantic treatment downstream.

When the environment exits, delete both. That's the whole protocol.

## Minimum recipe

The smallest script that lights up sutra:

```bash
#!/bin/bash
# `pipefail` matters: without it, `echo | sha256sum | cut` masks
# upstream failures and you get an empty $ID, which is dangerous —
# see the `clear_all_status` guard below.
set -euo pipefail

REG="$HOME/.dev-runner"; mkdir -p "$REG"

# `sha256sum` ships in coreutils on every Linux; macOS only has `shasum`.
# Probe so the script works on both. Both produce the same hex digest.
sha256() { if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi; }
ID=$(printf %s "$PWD" | sha256 | cut -c1-16)
[ ${#ID} -ge 8 ] || { echo "fatal: empty/short ID" >&2; exit 1; }

META="$REG/$ID"

trap 'rm -f "$META" "$REG/$ID".*.status' EXIT INT TERM

cat > "$META" <<EOF
DIR=$PWD
PID=$$
STARTED=$(date +%s)
SERVER_PORT=3000
EOF

update_status() {                 # atomic: write+rename, never bare ">"
    local f="$REG/$ID.$1.status"
    printf '%s\n' "$2" > "$f.tmp" && mv -f "$f.tmp" "$f"
}

update_status server "starting"
# ... start your server ...
update_status server "ready"
wait
```

Run this and sutra immediately shows a card with a green dot next to
"server" and a clickable `:3000` open-in-browser link.

**Why the safety dance** in this "minimum" recipe? Two of the lines
look paranoid for a 20-line snippet:

- The `[ ${#ID} -ge 8 ]` check guards against `sha256sum` being absent
  (or any other pipeline failure) producing an empty `$ID`. With an
  empty id, the trap line `rm -f "$REG/".*.status` would wipe every
  other sutra-tracked project's status files on the machine.
- The `write_status` helper does write-then-rename instead of `echo >
  file`. A bare `> file` is *truncate then write* — sutra's watcher
  fires on the truncate and can read the empty file before the write
  completes, flickering the dot to "no state". Atomic rename avoids
  the window entirely.

These are cheap to keep, costly to leave out.

## Recommended pattern

For a real dev runner — multiple subprocesses, restart support, sticky
ports — the snippets below mirror the structure of a working production
script. Adapt to taste.

### 1. Set up registry handles

```bash
# Central registry shared across all sutra-aware projects
REGISTRY_DIR="$HOME/.dev-runner"
mkdir -p "$REGISTRY_DIR"

# Portable SHA-256: macOS ships `shasum`, Linux ships `sha256sum`.
sha256() { if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi; }

# Stable per-project id derived from the project path. The `:-$0`
# fallback handles being sourced via stdin (`bash < dev.sh`), where
# BASH_SOURCE is empty.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" && pwd)"
REGISTRY_KEY=$(printf %s "$SCRIPT_DIR" | sha256 | cut -c1-16)
[ ${#REGISTRY_KEY} -ge 8 ] || { echo "fatal: empty registry key" >&2; exit 1; }
REGISTRY_FILE="$REGISTRY_DIR/$REGISTRY_KEY"
```

A 16-char hex prefix is plenty; sutra accepts any hex string (in
practice lowercase by convention, though the parser is case-permissive).
Hashing the path means the same project gets the same id across runs,
but two checkouts of the same project get different ids.

**The length guard isn't optional.** Several places below reference
`$REGISTRY_KEY` in glob patterns; if it ever goes empty those globs
match every dotfile in `~/.dev-runner/` belonging to *other* projects.
Fail fast at the top.

### 2. Write the meta file when you start

```bash
register_instance() {
    cat > "$REGISTRY_FILE" <<EOF
DIR=$SCRIPT_DIR
PID=$1
SERVER_PORT=$SERVER_PORT
FRONTEND_PORT=$FRONTEND_PORT
STARTED=$(date +%s)
EOF
}
```

- `DIR` — sutra shows the basename as the friendly project name.
- `PID` — sutra polls with `kill -0` to mark the env alive/dead.
- `*_PORT` — declares a port and matches by lowercase prefix to the
  unit name. `SERVER_PORT` → `server`, so the row gets a `↗`
  open-in-browser affordance.

### 3. Write status updates from each subprocess

```bash
# Convention: "<state>" or "<state>: <detail>".
# Atomic write+rename — see "Why atomic" below.
update_status() {
    local name="$1" status="$2"
    local f="$REGISTRY_DIR/$REGISTRY_KEY.$name.status"
    printf '%s\n' "$status" > "$f.tmp" && mv -f "$f.tmp" "$f"
}

# Export so subshells, cargo-watch, npm scripts can call it.
# Caveat: `export -f` is a bash extension. If a hook spawns `sh -c …`
# (POSIX sh, dash, BusyBox ash), the function won't be visible. Drop a
# tiny wrapper into a tmpdir on $PATH if you need cross-shell access:
#
#     TMPBIN=$(mktemp -d); export PATH="$TMPBIN:$PATH"
#     cat > "$TMPBIN/update_status" <<'SHIM'
#     #!/bin/sh
#     f="$REGISTRY_DIR/$REGISTRY_KEY.$1.status"
#     printf '%s\n' "$2" > "$f.tmp" && mv -f "$f.tmp" "$f"
#     SHIM
#     chmod +x "$TMPBIN/update_status"
export REGISTRY_DIR REGISTRY_KEY
export -f update_status
```

**Why atomic** (`> tmp && mv -f` instead of plain `> file`): a bare
redirect is `open(O_TRUNC) → write → close`. Sutra's filesystem
watcher fires on the truncate, reads the file mid-write, and gets
empty content (`State::None`). With write-and-rename the file
contents flip atomically from old to new, so the watcher only ever
sees a coherent state.

**Detail separator**: the canonical form is `"<state>: <detail>"`
(colon then space) — sutra's parser is lenient and also accepts
`"<state>:<detail>"` (no space) and trims surrounding whitespace,
but the canonical form reads better.

There are two ways to wire this up. Use whichever fits the subprocess.

#### a. Wrap the subprocess (when it's yours)

For code you control or a wrapper script that already brackets the
process, write states directly:

```bash
update_status server "building: cargo"
if cargo build --release; then
    update_status server "running"
    cargo run --release
else
    update_status server "failed: build error"
fi
```

For a watcher (cargo-watch, esbuild …) put `update_status` calls in
the watcher's pre/post-build hooks so the dot flips yellow ↔ green on
every rebuild.

#### b. Poll readiness from the side (when it isn't)

For third-party servers like uvicorn, Vite, Metro, or anything else
you don't want to fork or pipe-monitor, run a small sidecar loop that
flips status to `ready` once a probe URL responds:

```bash
update_status server "starting"
update_status vite   "starting"

# Server (uvicorn, gunicorn, …)
( cd "$SCRIPT_DIR/server" && exec ./run-server.sh --port "$SERVER_PORT" ) &

# UI (Vite, Next, …)
( cd "$SCRIPT_DIR/ui" && exec npm run dev -- --port "$VITE_PORT" ) &

# Readiness probe — flip to "ready" when each URL responds.
# Use 127.0.0.1 (not localhost) consistently so a v6-only resolver
# can't trip the IPv4-bound probe.
(
    server_ready=false
    vite_ready=false
    i=0
    while [ "$i" -lt 60 ]; do
        if [ "$server_ready" = false ] && \
           curl -sf "http://127.0.0.1:$SERVER_PORT/health" >/dev/null 2>&1; then
            update_status server "ready"
            server_ready=true
        fi
        if [ "$vite_ready" = false ] && \
           curl -sf "http://127.0.0.1:$VITE_PORT/" >/dev/null 2>&1; then
            update_status vite "ready"
            vite_ready=true
        fi
        [ "$server_ready" = true ] && [ "$vite_ready" = true ] && break
        sleep 1
        i=$((i+1))
    done
    [ "$server_ready" = false ] && update_status server "failed: timeout"
    [ "$vite_ready"   = false ] && update_status vite   "failed: timeout"
) &
SIDECAR_PID=$!
# Ensure the sidecar dies with the supervisor — otherwise it can
# outlive the parent and write a stale "failed: timeout" 60s after
# you've already cleaned up. Add to your cleanup trap:
#     kill "$SIDECAR_PID" 2>/dev/null || true
```

The sidecar is a single backgrounded subshell that lives only until
both probes succeed or the 60s budget expires. Track its PID and kill
it from your cleanup trap so it can't outlive the parent script.

The `while`/`i++` loop is portable; the C-style `for ((i=0; i<60; i++))`
form is bash-only and won't run under `dash`/`ash`/POSIX `sh`.

If `curl` isn't available everywhere your script runs (some minimal
containers ship `wget` only), probe with `wget -qO- "$URL"` or use
bash's `/dev/tcp` builtin: `(echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null`.

#### Self-clearing transient units

Short-lived units (`build`, `seed`, `migrate`) shouldn't linger as
stale `ready` rows after they finish. Have them remove their own file
a beat after completing — but guard with the parent script's PID so
the orphan can't outlive the run and delete a *fresh* run's status:

```bash
update_status build "ready"
script_pid=$$
( sleep 2; kill -0 "$script_pid" 2>/dev/null && \
    rm -f "$REGISTRY_DIR/$REGISTRY_KEY.build.status" ) &
```

The 2-second window is enough for the user to see the success state
and for sutra to fire its transition notifications; after that the
row disappears so it doesn't sit around as a stale `ready`. The
`kill -0` guard short-circuits if the parent has already exited and
been replaced by a re-run — without it, the orphan would race the
new run and delete *its* freshly-written status file.

### 4. Always clean up

Status files outlive the script if you forget to remove them. Put the
cleanup in a trap **and** in your `--stop` handler:

```bash
clear_all_status() {
    # Defensive: refuse if REGISTRY_KEY is somehow empty/short. Without
    # this guard, a bug elsewhere that leaves $REGISTRY_KEY empty turns
    # this rm into "wipe every other project's status files in
    # ~/.dev-runner/". Cheap insurance for a destructive operation.
    [ -n "${REGISTRY_KEY:-}" ] && [ ${#REGISTRY_KEY} -ge 8 ] || {
        echo "clear_all_status: refusing — REGISTRY_KEY missing or too short" >&2
        return 1
    }
    rm -f "$REGISTRY_DIR/$REGISTRY_KEY".*.status
}

unregister_instance() {
    clear_all_status
    rm -f "$REGISTRY_FILE"
}

trap unregister_instance EXIT INT TERM HUP
```

It's also worth calling `clear_all_status` at the **start** of a fresh
run, not just on exit. Crashes can leave status files behind that
weren't covered by the trap; clearing them defensively keeps the
dashboard truthful:

```bash
cmd_start() {
    if check_running; then show_status; exit 0; fi
    clear_all_status   # belt-and-suspenders for prior crashes
    do_fresh_start
}
```

If your dev runner backgrounds itself, run the work inside a process-
group leader so a single `kill -- -$PGID` reaps the whole tree. Bash
gives you this with `set -m`. **Important**: `$BASHPID` requires bash
4.0+; macOS ships bash 3.2 by default, where it expands to the empty
string and the cleanup silently does nothing. Either require bash 4+
explicitly, or use the portable `ps` workaround below.

```bash
# Require bash 4+ so $BASHPID works (macOS users: `brew install bash`)
if [ "${BASH_VERSINFO[0]:-0}" -lt 4 ]; then
    echo "fatal: bash 4+ required (you have ${BASH_VERSION:-unknown})" >&2
    echo "  macOS: brew install bash, then re-run with /opt/homebrew/bin/bash $0" >&2
    exit 1
fi

set -m
(
    MY_PGID=$BASHPID
    cleanup() {
        trap - EXIT INT TERM HUP
        rm -f "$REGISTRY_FILE"
        clear_all_status
        kill -- -"$MY_PGID" 2>/dev/null || true
    }
    trap cleanup EXIT INT TERM HUP

    # ... start all subprocesses ...
    wait
) > "$LOG_FILE" 2>&1 &
SUPERVISOR_PID=$!
echo "$SUPERVISOR_PID" > "$PID_FILE"
register_instance "$SUPERVISOR_PID"

# Verify the supervisor is its own process-group leader. If $set -m
# didn't take effect (some non-interactive shells), the trap above
# can't `kill -- -$MY_PGID` cleanly and a SIGTERM (e.g. from sutra's
# terminate button, or from `./dev.sh --stop`) will leak children.
sleep 0.05
actual_pgid=$(ps -o pgid= -p "$SUPERVISOR_PID" 2>/dev/null | tr -d ' ')
[ "$actual_pgid" = "$SUPERVISOR_PID" ] || \
    echo "warning: supervisor is not a process-group leader — your trap can't reap the group" >&2
```

If you can't depend on bash 4+ (e.g. you ship to colleagues who use
the system bash on macOS), substitute the `MY_PGID` line with a
portable read from `ps`:

```bash
# Inside the subshell, before installing the trap:
MY_PGID=$(ps -o pgid= -p $$ | tr -d ' ')
```

Note that on BusyBox `ps` (Alpine, embedded Linux) the `-o pgid=`
extension isn't available; on those platforms you'll need
`/proc/$$/stat` field 5 instead.

### 5. (Optional) Detect stale instances

Before starting, check whether a previous run is still alive — if not,
clean up its leftovers:

```bash
check_running() {
    [ -f "$PID_FILE" ] || return 1
    local pid=$(cat "$PID_FILE")
    if ! kill -0 "$pid" 2>/dev/null; then
        rm -f "$PID_FILE"
        unregister_instance
        return 1
    fi
    return 0
}
```

A stricter version verifies the PID is still a process-group leader,
guarding against PID recycling. Add this *inside* the `check_running`
function above, after the `kill -0` block (using `local` outside a
function is a syntax error, so don't lift the snippet standalone):

```bash
check_running() {
    [ -f "$PID_FILE" ] || return 1
    local pid
    pid=$(cat "$PID_FILE")
    if ! kill -0 "$pid" 2>/dev/null; then
        rm -f "$PID_FILE"; unregister_instance; return 1
    fi

    # Stricter: verify $pid is still its own process-group leader.
    # Catches the (rare) case where the kernel recycled the PID to
    # a different, unrelated process.
    local pgid
    pgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')
    [ "$pgid" = "$pid" ] || { rm -f "$PID_FILE"; unregister_instance; return 1; }

    return 0
}
```

`ps -o pgid= -p` works on macOS/BSD `ps` and GNU `ps` (procps). On
BusyBox `ps` (Alpine, embedded) the `-o` extension is limited; fall
back to reading `/proc/$pid/stat` field 5 if you need to support those.

## States — what to write and when

Status file content is `<state>` or `<state>: <detail>` on a single
line. Overwrite atomically (`echo "..." > file`, never `>>`) on each
state transition, not on every log line.

The canonical states, roughly in the order a unit moves through them:

- **`starting`** — you've launched the process but it isn't doing
  useful work yet (port binding, config load, importing modules).
  Optional; for fast-starting things you can skip straight to
  `running` or `ready`.

- **`building`** — work is in progress that the user is actively
  waiting on: compiling, bundling, installing dependencies. Use the
  detail to name the substep so the user can tell *what* is building
  without tailing logs: `building: cargo`, `building: wasm-pack`,
  `building: npm install`. Write this every time a watcher
  (cargo-watch, vite, esbuild) re-runs, not just on first build.

- **`running`** — the process is up. For things without a port
  (workers, watchers, background jobs) this is the terminal state.
  For servers it's an intermediate state between "started" and
  "accepting traffic" — flip to `ready` once the URL responds.

- **`ready`** — the process is fully serving. Use whenever there's a
  port and a readiness condition you can check (HTTP 200 on
  `/health`, log line `Listening on...`). Downstream tooling treats
  `ready` as "safe to hit it now."

- **`failed`** — unrecoverable error. *Always include a detail*
  saying why, in 2–5 words: `failed: exit code 1`,
  `failed: build error`, `failed: timeout`. Without the detail the
  user has to tail logs to find out what went wrong, defeating the
  point.

- **`stopped`** — you intentionally stopped *one* unit but the
  supervisor and its other units are still alive. Concrete example:
  a `--mobile disable` flag that takes down the Metro bundler but
  leaves the API server running — you'd write `stopped` to
  `metro.status` while the `server` unit stays `ready`. On *full*
  shutdown of the whole environment, *delete* the status file
  instead — that's how `clear_all_status` works. If your dev script
  doesn't have a per-unit disable feature, you'll never write
  `stopped` and that's fine.

The detail after `: ` is free-form. Same-state writes that only
change the detail (e.g. `building: cargo` → `building: wasm-pack`)
don't fire transition notifications, so it's safe to update the
detail aggressively without spamming the user.

For sutra's rendering of these states (color, glyph, system sound)
see [STATE_SPEC.md](../STATE_SPEC.md). As a writer you don't need to
care about that — pick the right state string and the rendering
follows.

## Common units

A typical web stack might emit:

| Unit       | Example states |
|------------|----------------|
| `server`   | `building: cargo`, `running` |
| `wasm`     | `building: wasm-pack`, `ready`, `failed: wasm-pack error` |
| `vite`     | `starting`, `ready` |
| `metro`    | `starting`, `running`, `failed: timeout` |
| `mobile`   | `building: rust bindings`, `building: xcode`, `ready` |
| `seed`     | `running`, `ready`, `failed` (self-clearing — see "transient units" above) |

Names are arbitrary — pick whatever makes sense and is short enough to
fit in a column.

## Writing status from non-bash subprocesses

The protocol is "just files", so anything that can write a one-line
text file with an atomic rename can update status. A few one-liners:

**Python** (3.6+):
```python
import os, tempfile
def update_status(name, status):
    reg = os.environ["REGISTRY_DIR"]; key = os.environ["REGISTRY_KEY"]
    path = f"{reg}/{key}.{name}.status"
    fd, tmp = tempfile.mkstemp(dir=reg); os.write(fd, status.encode()+b"\n"); os.close(fd)
    os.replace(tmp, path)
```

**Node.js**:
```js
const fs = require("fs"), path = require("path");
function updateStatus(name, status) {
  const reg = process.env.REGISTRY_DIR, key = process.env.REGISTRY_KEY;
  const dest = path.join(reg, `${key}.${name}.status`);
  const tmp = `${dest}.tmp`;
  fs.writeFileSync(tmp, status + "\n"); fs.renameSync(tmp, dest);
}
```

**Rust**: write to `<dest>.tmp`, then `std::fs::rename`. Same pattern.

In all cases: export `REGISTRY_DIR` and `REGISTRY_KEY` from your
top-level dev script (per §1) so the subprocess inherits them. The
write-then-rename pattern matches the bash `update_status` helper —
sutra's watcher only ever sees the complete file.

## Subprocess crashes after `ready`

Sutra's PID liveness check covers the supervisor only. If a *child*
(server, vite, metro) crashes mid-run, sutra has no way to know — the
last status it has from that unit was `ready`, and that's what it'll
keep showing.

Two reasonable patterns for catching post-`ready` crashes:

1. **Keep probing** — extend the readiness sidecar to a continuous
   liveness loop:

   ```bash
   (
       while sleep 5; do
           curl -sf "http://127.0.0.1:$SERVER_PORT/health" >/dev/null 2>&1 \
               || update_status server "failed: probe lost"
       done
   ) &
   LIVENESS_PID=$!  # add to cleanup trap
   ```

2. **`wait` on the child PID** — if you launched the child yourself,
   the supervisor knows when it dies:

   ```bash
   ( exec ./run-server.sh --port "$SERVER_PORT" ) &
   SERVER_PID=$!
   (
       wait "$SERVER_PID"
       rc=$?
       update_status server "failed: exited rc=$rc"
   ) &
   ```

Pick one based on whether you have the child PID or just a port.

## Workflow runners (Procfile / foreman / overmind / make)

If your "dev runner" is a `Procfile` driven by `foreman`/`overmind`,
or a `Makefile` target, you have two options:

- **Wrap, don't replace.** Write a thin `dev.sh` that does the sutra
  registration and then `exec`s your existing tool — `exec foreman
  start` or `exec make dev`. The registry/status/cleanup lives in the
  wrapper; the actual orchestration stays where you have it.
- **Add status writes inside the inner tool.** For `make`, add
  `update_status build "ready"`-style calls to recipe targets. For
  `Procfile`, it's harder — Procfile entries don't have hooks; use
  the readiness-probe sidecar instead.

The "wrap, don't replace" pattern is usually the right call. It also
lets you keep `./dev.sh --stop` etc. while not touching the user's
existing workflow tools.

## HMR / live-reload servers

`vite dev`, `uvicorn --reload`, `nodemon`, etc. restart their inner
process on file changes. They don't expose pre-/post-restart hooks
the way `cargo-watch -s` does, so you can't easily flash `building`
on every reload. Two pragmatic options:

- **Status `ready` once, leave it alone.** The reload happens in the
  background; users notice via the browser, not sutra. This is the
  simplest path.
- **Tail the tool's output.** Run the dev server with stdout going
  through a `tee` into a fifo and a `while read` that flips status:

  ```bash
  ( npm run dev 2>&1 | while IFS= read -r line; do
      echo "$line"
      case "$line" in
          *"ready in"*|*"server running at"*) update_status vite "ready" ;;
          *"page reload"*|*"hmr update"*)     update_status vite "building: hmr" ;;
      esac
  done ) &
  ```

  Note: piping a long-running process through `while read` makes the
  pipe a SIGPIPE risk if the reader exits — the writer dies on its
  next stdout write. Either keep the loop alive for the whole run, or
  write to a log file and tail it separately.

## Checklist

Before declaring the integration done, verify each of these. They map
directly to bugs sutra users hit when a dev runner is partway done.

- [ ] **Stable id.** `REGISTRY_KEY` is derived from the absolute project
      path (SHA-256, first 16+ hex chars), no dots, lowercase by
      convention.
- [ ] **`set -euo pipefail`** at the top of the script (not just `set
      -e`). Otherwise a missing `sha256sum`/`shasum` silently produces
      an empty `$REGISTRY_KEY`.
- [ ] **Length-guard on `$REGISTRY_KEY`.** `[ ${#REGISTRY_KEY} -ge 8 ]
      || exit 1` immediately after computing it. Empty key + glob
      cleanup = wiping every other project's status files.
- [ ] **`clear_all_status` refuses on empty key.** Same reason as
      above; defense in depth.
- [ ] **Meta written at start.** `~/.dev-runner/<id>` exists once the
      script reaches "servers running", with at least `DIR=` and
      `PID=`. Add `*_PORT=` for any service that has a port.
- [ ] **Atomic status writes.** Use write-to-tmp-then-rename (`mv -f
      file.tmp file`), not bare `> file`. The watcher fires on
      `O_TRUNC` and reads the empty interim state.
- [ ] **Status written for each subprocess.** Every long-running
      subprocess has a corresponding `<id>.<unit>.status` file that
      transitions through `starting` → `ready` (or `building` →
      `running`/`ready`).
- [ ] **`failed` is reachable.** If the build or server crashes, *some*
      unit ends up at `failed: <reason>` rather than just disappearing.
- [ ] **Trap on EXIT/INT/TERM/HUP.** Both the meta file and *all*
      `<id>.*.status` files are removed when the script exits, even
      under Ctrl+C or `kill -TERM`.
- [ ] **Defensive clear at start.** `clear_all_status` runs before a
      fresh start, so a previous crash doesn't leave the dashboard
      showing stale `ready` rows.
- [ ] **`PID=` is the supervisor and a process-group leader.** The
      PID written to the meta file should satisfy `PGID == PID` —
      verify with `ps -o pgid= -p $PID`. This isn't for sutra's
      benefit (sutra only ever signals the one PID); it's so your
      *own* SIGTERM trap can `kill -- -$PGID` to reap children when
      anyone — sutra, `./dev.sh --stop`, manual `kill` — asks the
      supervisor to exit. If you're on the bash `set -m + (...) &`
      pattern, also verify you're on bash 4+ (macOS bash 3.2 makes
      `$BASHPID` empty and silently breaks group cleanup).
- [ ] **Re-runs are idempotent.** Running the script twice in a row
      doesn't pile up duplicate registry entries or status files. (The
      `kill -0` liveness check + `unregister_instance` covers this.)
- [ ] **Background helpers don't outlive the parent.** Sidecar probes
      and self-clearing-unit timers should either be killed in the
      cleanup trap (track their PID) or guard themselves with `kill -0
      $script_pid` before acting.
- [ ] **Manual smoke test.** Start the script, run
      `ls ~/.dev-runner/`, confirm one meta file + one or more
      `<id>.*.status` files. Stop the script, run `ls` again, confirm
      the entries are gone.

If `~/.dev-runner/` doesn't exist on the user's machine, your script
should `mkdir -p` it — sutra creates it on its own startup but
doesn't require it to exist before the dev script runs.

## Troubleshooting

- **My env doesn't show up.** The meta filename must be hex-only with
  no `.`. `id=$(... | cut -c1-16)` is fine; `id="myproject"` is not.
  Also check `$REGISTRY_KEY` isn't empty (`sha256sum`/`shasum` failed
  silently — see the length-guard in the recipe).
- **Status row sticks around after stop.** Trap missed, or the process
  was killed with `-9` first. Add `clear_all_status` to your `--stop`
  handler too. If you're using a sidecar probe or self-clearing
  transient unit, make sure it can't outlive the parent (track its
  PID in cleanup, or guard with `kill -0 $script_pid`).
- **Stale `ready` on a unit whose process actually died.** Sutra's
  liveness check covers the supervisor only — see the
  "Subprocess crashes after `ready`" section above for the two
  patterns that catch child crashes.
- **Sutra plays a sound storm on startup.** It shouldn't — sutra
  snapshots state silently on launch. If it does, file an issue with
  the contents of `~/.dev-runner/`.
- **Detail text isn't updating live.** Make sure your status writes
  are atomic — write to `<file>.tmp` and `mv -f` it into place. Bare
  `> file` is truncate-then-write and the watcher can read the empty
  intermediate state.
- **macOS: SIGTERM to the supervisor doesn't reap children.** Your
  supervisor's trap is silently failing to kill its group — you're
  probably on bash 3.2 (the system default), where `$BASHPID` is
  empty and `kill -- -""` is a no-op. Either `brew install bash` and
  use `/opt/homebrew/bin/bash` in your shebang, or substitute
  `MY_PGID=$(ps -o pgid= -p $$ | tr -d ' ')` for the `$BASHPID` line.
  This affects any caller that sends SIGTERM — sutra's terminate
  button, `./dev.sh --stop`, manual `kill <pid>` — they all rely on
  your trap to reap the group.
- **Two checkouts of the same project conflict.** They won't — the id
  hashes the absolute path, so `~/code/myapp` and `~/work/myapp` get
  different ids.

## See also

- [STATE_SPEC.md](../STATE_SPEC.md) — exact file format and edge cases.
- Source repo: <https://github.com/synestheticsystems/sutra>
