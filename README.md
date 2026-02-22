# Lineforge

AI session manager for Claude Code and Codex CLIs. Spawn, monitor, and attach to AI coding sessions through a web UI or your terminal.

## Features

- **PTY-based sessions** — Spawn Claude Code or Codex in managed pseudo-terminals
- **Web UI** — Dark-themed dashboard with xterm.js terminal, live log streaming via SSE
- **Terminal attach** — Connect to any running session from your terminal with `forge attach`
- **iTerm2 integration** — Auto-open sessions in iTerm2 windows on macOS
- **Settings TUI** — Interactive ratatui-based configuration editor
- **Tailscale-first networking** — Binds to your Tailscale IP by default for remote access
- **Yolo mode** — Auto-approve AI tool calls for unattended sessions

## Quick Start

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/tonyjom/lineforge/main/scripts/install.sh | bash

# Start the server
forge serve

# Create a session and attach
forge new --label my-project --cwd ~/projects/myapp
```

Press `Ctrl+]` to detach from a session.

## Installation

### Install script (recommended)

Downloads a pre-built binary or falls back to building from source:

```bash
bash scripts/install.sh
```

To force a source build:

```bash
bash scripts/install.sh --from-source
```

The binary installs to `~/.local/bin/forge`.

### Build from source

Requires Rust (install via [rustup.rs](https://rustup.rs)):

```bash
git clone https://github.com/tonyjom/lineforge.git
cd lineforge
cargo build --release
cp target/release/forge ~/.local/bin/
```

## CLI Reference

### `forge serve`

Start the backend server and web UI.

```
forge serve [--port <PORT>] [--bind <BIND>] [--config <PATH>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `42067` | Port to listen on |
| `--bind` | `tailscale` | Bind address (`tailscale` auto-resolves your Tailscale IP, falls back to `127.0.0.1`) |
| `--config` | `~/.config/lineforge/config.toml` | Config file path |

### `forge new`

Create a new session and attach to it immediately.

```
forge new [--label <NAME>] [--cwd <DIR>] [--tool <claude|codex>] [--no-iterm] [-- extra args...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--label` | auto-generated | Session name |
| `--cwd` | current directory | Working directory for the AI CLI |
| `--tool` | from config (`claude`) | AI CLI to use |
| `--no-iterm` | — | Skip auto-opening iTerm2 |
| trailing args | — | Extra arguments passed to the AI CLI |

### `forge new-session`

Create a session without attaching. Prints the session ID. Takes the same flags as `forge new`.

### `forge attach <ID>`

Attach your terminal to a running session via Unix socket. Supports UUID prefix matching.

### `forge list`

List all sessions with status, tool, and creation time.

### `forge kill <ID>`

Stop a running session (sends SIGTERM). Supports UUID prefix matching.

### `forge settings`

Open the interactive TUI settings editor.

| Key | Action |
|-----|--------|
| `j/k` or arrows | Navigate |
| `Enter` / `Space` | Toggle value |
| `h/l` or arrows | Adjust numbers |
| `s` | Save |
| `q` / `Esc` | Quit |

## Configuration

Config file: `~/.config/lineforge/config.toml`

```toml
port = 42067
bind = "tailscale"
default_tool = "claude"
# tool_path = "/usr/local/bin/claude"
# default_dirs = ["/home/user/projects"]
iterm_enabled = true
log_retention_days = 7
max_log_lines = 10000
yolo_mode = false
```

| Field | Default | Description |
|-------|---------|-------------|
| `port` | `42067` | Server port |
| `bind` | `"tailscale"` | Bind address (set to `"127.0.0.1"` for local-only) |
| `default_tool` | `"claude"` | Default AI CLI (`claude` or `codex`) |
| `tool_path` | — | Custom path to the AI CLI binary |
| `default_dirs` | `[]` | Suggested working directories |
| `iterm_enabled` | `true` | Enable iTerm2 integration (macOS) |
| `log_retention_days` | `7` | Days to keep session logs |
| `max_log_lines` | `10000` | Ring buffer size for in-memory logs |
| `yolo_mode` | `false` | Auto-approve AI tool calls (`--dangerously-skip-permissions` for Claude, `--yolo` for Codex) |

## Web UI

Once the server is running, open `http://<bind>:<port>` in your browser.

- **Dashboard** (`/`) — List all sessions with status badges
- **New session** (`/new`) — Form to create a session
- **Session view** (`/sessions/{id}`) — Live terminal via xterm.js, stop/iTerm2 buttons

### API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/health` | Health check |
| `GET` | `/api/sessions` | List sessions (JSON) |
| `POST` | `/api/sessions` | Create session |
| `GET` | `/api/sessions/{id}` | Get session metadata |
| `POST` | `/api/sessions/{id}/input` | Send input to session PTY |
| `POST` | `/api/sessions/{id}/stop` | Stop session |
| `GET` | `/api/sessions/{id}/logs` | Stream logs (SSE) |
| `POST` | `/api/sessions/{id}/open-iterm` | Open in iTerm2 |

## Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐
│  Web UI     │────▶│  Axum Server │────▶│  SessionManager │
│  (xterm.js) │◀─SSE│  (port 42067)│     │                 │
└─────────────┘     └──────────────┘     │  HashMap<Uuid,  │
                                         │   LiveSession>   │
┌─────────────┐     ┌──────────────┐     │                 │
│  forge CLI  │────▶│ Unix Socket  │────▶│  PTY ──▶ claude │
│  (attach)   │◀────│ (/tmp/lineforge)   │      or codex   │
└─────────────┘     └──────────────┘     └─────────────────┘
```

Each session runs as a child process in a PTY. Output flows through a ring buffer and broadcast channel to both SSE (web) and Unix socket (terminal attach) clients. Session metadata persists to `~/.local/share/lineforge/sessions/{id}/meta.json`.

## Development

```bash
# Run in development
cargo run -- serve

# Run checks
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

# Build release
cargo build --release
```

CI runs on push/PR to `main` via GitHub Actions (format, lint, build, test). Releases are automated on version tags — version bumps follow [conventional commits](https://www.conventionalcommits.org/).
