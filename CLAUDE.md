# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
cargo run -- serve              # Run dev server (default port 42067)
cargo build --release           # Release build
cargo test                      # Run tests (none exist yet)
```

After modifying code, always run format and lint to match CI:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```

Binary name is `forge`. Crate name is also `forge`.

## Architecture

Lineforge is a Rust-based AI session manager that spawns and controls CLI tools (Claude Code, Codex) via PTY, with a web UI for monitoring and a CLI for direct terminal attach.

### Core Flow

**Server mode** (`forge serve`): Loads `Config` from `~/.config/lineforge/config.toml` → resolves bind address (supports Tailscale IP auto-detection) → creates `SessionManager` → restores on-disk session metadata (marks stale `Running` as `Stopped`) → starts axum server with API, SSE, template, and static asset routes.

**Session lifecycle** (`SessionManager::spawn`): Opens PTY pair → spawns tool process → creates `SessionLog` (ring buffer + broadcast channel) → creates mpsc channel for input commands → splits PTY into `(OwnedReadPty, OwnedWritePty)` for concurrent read/write tasks → starts Unix socket attach listener at `/tmp/lineforge/{id}.sock` → stores `LiveSession` in `Arc<RwLock<HashMap<Uuid, Arc<RwLock<LiveSession>>>>>`.

**Web UI**: Askama templates + xterm.js. Browser connects via SSE (`/api/sessions/{id}/logs`) for live output. Input sent via `POST /api/sessions/{id}/input`. Resize events flow bidirectionally through SSE and a `watch` channel to the PTY.

**CLI attach**: Connects to session's Unix socket, enables terminal raw mode via crossterm, streams PTY output directly. `Ctrl+]` (0x1d) detaches. On disconnect, session is stopped via SIGTERM.

### Key Modules

- `src/session/manager.rs` — Session spawning, PTY I/O loop, attach socket, stop/kill logic. The central module.
- `src/session/log.rs` — Ring buffer (`VecDeque`) + `broadcast::Sender` for log distribution, with file persistence to `output.log`.
- `src/session/model.rs` — `SessionMeta`, `SessionStatus`, `ToolKind` data types.
- `src/server/api.rs` — REST API routes (axum 0.8, route params use `{id}` syntax).
- `src/server/sse.rs` — SSE streaming: snapshot replay + live broadcast + resize events via `tokio::select!`.
- `src/server/templates.rs` — Askama HTML template routes.
- `src/cli/commands.rs` — Clap CLI definition and dispatch. CLI subcommands call the running server via reqwest HTTP.
- `src/cli/settings.rs` — Ratatui-based TUI settings editor.
- `src/config/mod.rs` — TOML config load/save with auto-creation of defaults.

### Important Patterns

- **pty-process builder consumes self**: `Command::new(path).args(&a).current_dir(&d).spawn(pts)` must be chained in one expression.
- **PTY split for concurrency**: `pty.into_split()` → `(OwnedReadPty, OwnedWritePty)`. Resize goes through the write half.
- **Stop/exit race condition**: When `stop()` sends SIGTERM, the PTY read loop also exits. The read loop checks if status is still `Running` before overwriting — prevents clobbering the `Stopped` status set by `stop()`.
- **Attach socket readiness**: Uses `oneshot::channel` to signal socket is listening before `spawn()` returns.
- **UTF-8 partial reads**: PTY output is binary; a `leftover` buffer accumulates incomplete multi-byte sequences across reads.
- **Static assets embedded**: `rust-embed` compiles `static/` into the binary at build time.
- **tokio-stream `sync` feature**: Required for `BroadcastStream`.

### File Locations at Runtime

- Config: `~/.config/lineforge/config.toml`
- Session data: `~/.local/share/lineforge/sessions/{id}/meta.json` and `output.log`
- Attach sockets: `/tmp/lineforge/{id}.sock`

## CI

CI (on push/PR to `main`) runs: `fmt --check` → `clippy` → `build` → `test`. Version bumps use conventional commits (`feat:` = minor, `fix:` = patch, `BREAKING CHANGE` = major).
