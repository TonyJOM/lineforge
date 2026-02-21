use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{RwLock, mpsc, oneshot};
use uuid::Uuid;

use crate::config::Config;
use crate::error::ForgeError;
use crate::session::log::SessionLog;
use crate::session::model::{SessionMeta, SessionStatus, ToolKind};

fn sock_dir() -> PathBuf {
    PathBuf::from("/tmp/lineforge")
}

pub struct LiveSession {
    pub meta: SessionMeta,
    pub log: SessionLog,
    pub input_tx: mpsc::Sender<Vec<u8>>,
}

#[derive(Clone)]
pub struct SessionManager {
    pub sessions: Arc<RwLock<HashMap<Uuid, Arc<RwLock<LiveSession>>>>>,
    pub config: Config,
}

impl SessionManager {
    pub fn new(config: Config) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub async fn list(&self) -> Vec<SessionMeta> {
        let sessions = self.sessions.read().await;
        let mut metas = Vec::new();
        for session in sessions.values() {
            let s = session.read().await;
            metas.push(s.meta.clone());
        }
        metas.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        metas
    }

    pub async fn get(&self, id: Uuid) -> Result<SessionMeta> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(ForgeError::SessionNotFound(id))?;
        let s = session.read().await;
        Ok(s.meta.clone())
    }

    #[allow(dead_code)]
    pub async fn resolve_id(&self, prefix: &str) -> Result<Uuid> {
        // Try full UUID first
        if let Ok(id) = prefix.parse::<Uuid>() {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&id) {
                return Ok(id);
            }
            return Err(ForgeError::SessionNotFound(id).into());
        }

        // Try prefix match
        let sessions = self.sessions.read().await;
        let matches: Vec<_> = sessions
            .keys()
            .filter(|id| id.to_string().starts_with(prefix))
            .cloned()
            .collect();

        match matches.len() {
            0 => anyhow::bail!("No session matching prefix: {prefix}"),
            1 => Ok(matches[0]),
            n => anyhow::bail!("{n} sessions match prefix '{prefix}', be more specific"),
        }
    }

    pub async fn spawn(
        &self,
        name: String,
        tool: ToolKind,
        working_dir: PathBuf,
        extra_args: Vec<String>,
    ) -> Result<SessionMeta> {
        let id = Uuid::new_v4();
        let session_dir = Config::sessions_dir().join(id.to_string());
        std::fs::create_dir_all(&session_dir)?;

        let tool_path = crate::session::pty::resolve_tool_path(&self.config, &tool)?;

        let mut extra_args = extra_args;
        if self.config.yolo_mode {
            let yolo_flag = match tool {
                ToolKind::Claude => "--dangerously-skip-permissions",
                ToolKind::Codex => "--yolo",
            };
            if !extra_args.iter().any(|a| a == yolo_flag) {
                extra_args.insert(0, yolo_flag.to_string());
            }
        }

        // Create PTY pair
        let (pty, pts) = pty_process::open()
            .map_err(|e| ForgeError::Pty(format!("Failed to create PTY: {e}")))?;

        // Set reasonable terminal size
        pty.resize(pty_process::Size::new(24, 80))
            .map_err(|e| ForgeError::Pty(format!("Failed to resize PTY: {e}")))?;

        // Build and spawn command (builder methods consume self)
        let child = pty_process::Command::new(&tool_path)
            .args(&extra_args)
            .current_dir(&working_dir)
            .spawn(pts)
            .map_err(|e| ForgeError::Pty(format!("Failed to spawn {tool_path}: {e}")))?;

        let pid = child.id();
        let now = chrono::Utc::now();
        let meta = SessionMeta {
            id,
            name,
            tool,
            status: SessionStatus::Running,
            working_dir,
            created_at: now,
            updated_at: now,
            pid,
            extra_args,
        };

        // Save meta to disk
        let meta_path = session_dir.join("meta.json");
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(&meta_path, meta_json)?;

        // Set up log
        let log_file = session_dir.join("output.log");
        let log = SessionLog::new(self.config.max_log_lines, Some(log_file));

        // Set up input channel
        let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>(256);

        let live = Arc::new(RwLock::new(LiveSession {
            meta: meta.clone(),
            log,
            input_tx,
        }));

        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(id, live.clone());
        }

        // Spawn read/write tasks
        let sessions_ref = self.sessions.clone();
        tokio::spawn(async move {
            run_pty_io(pty, child, input_rx, sessions_ref, id).await;
        });

        // Start Unix socket listener for attach
        let sock_base = sock_dir();
        std::fs::create_dir_all(&sock_base)?;
        let attach_sock = sock_base.join(format!("{id}.sock"));
        let input_tx_attach = {
            let s = live.read().await;
            s.input_tx.clone()
        };
        let broadcast_tx_attach = {
            let s = live.read().await;
            s.log.broadcast_tx.clone()
        };
        let sessions_attach = self.sessions.clone();
        let (sock_ready_tx, sock_ready_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_attach_listener(
                attach_sock,
                input_tx_attach,
                broadcast_tx_attach,
                sessions_attach,
                id,
                sock_ready_tx,
            )
            .await;
        });

        // Wait for the attach socket to be ready before returning
        let _ = sock_ready_rx.await;

        Ok(meta)
    }

    pub async fn send_input(&self, id: Uuid, data: Vec<u8>) -> Result<()> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(ForgeError::SessionNotFound(id))?;
        let s = session.read().await;
        if s.meta.status != SessionStatus::Running {
            return Err(ForgeError::SessionAlreadyStopped(id).into());
        }
        s.input_tx
            .send(data)
            .await
            .map_err(|_| ForgeError::Pty("Input channel closed".into()))?;
        Ok(())
    }

    pub async fn stop(&self, id: Uuid) -> Result<()> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(ForgeError::SessionNotFound(id))?;
        let mut s = session.write().await;
        if s.meta.status != SessionStatus::Running {
            return Err(ForgeError::SessionAlreadyStopped(id).into());
        }

        // Send SIGTERM via kill
        if let Some(pid) = s.meta.pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        s.meta.status = SessionStatus::Stopped;
        s.meta.updated_at = chrono::Utc::now();

        // Update meta on disk
        let meta_path = Config::sessions_dir()
            .join(id.to_string())
            .join("meta.json");
        if let Ok(json) = serde_json::to_string_pretty(&s.meta) {
            let _ = std::fs::write(&meta_path, json);
        }

        // Clean up attach socket
        let sock_file = sock_dir().join(format!("{id}.sock"));
        let _ = std::fs::remove_file(&sock_file);

        Ok(())
    }

    pub async fn get_log_snapshot(&self, id: Uuid) -> Result<Vec<crate::session::log::LogEntry>> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(ForgeError::SessionNotFound(id))?;
        let s = session.read().await;
        Ok(s.log.snapshot())
    }

    pub async fn subscribe_logs(
        &self,
        id: Uuid,
    ) -> Result<tokio::sync::broadcast::Receiver<crate::session::log::LogEntry>> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(ForgeError::SessionNotFound(id))?;
        let s = session.read().await;
        Ok(s.log.subscribe())
    }
}

async fn run_pty_io(
    pty: pty_process::Pty,
    mut child: tokio::process::Child,
    mut input_rx: mpsc::Receiver<Vec<u8>>,
    sessions: Arc<RwLock<HashMap<Uuid, Arc<RwLock<LiveSession>>>>>,
    id: Uuid,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut pty_reader, mut pty_writer) = pty.into_split();

    // Write task: forward input to PTY
    let write_handle = tokio::spawn(async move {
        while let Some(data) = input_rx.recv().await {
            if pty_writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Read loop: PTY output -> broadcast + ring buffer
    let mut buf = vec![0u8; 4096];
    loop {
        match pty_reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                let sessions_guard = sessions.read().await;
                if let Some(session) = sessions_guard.get(&id) {
                    let mut s = session.write().await;
                    s.log.push(text);
                }
            }
            Err(_) => break,
        }
    }

    write_handle.abort();

    // Wait for child process to exit
    let status = child.wait().await;

    // Update session status (only if still Running - stop() may have already set it)
    let sessions_guard = sessions.read().await;
    if let Some(session) = sessions_guard.get(&id) {
        let mut s = session.write().await;
        if s.meta.status == SessionStatus::Running {
            s.meta.status = match status {
                Ok(exit) if exit.success() => SessionStatus::Stopped,
                Ok(_) => SessionStatus::Errored("Process exited with non-zero status".into()),
                Err(e) => SessionStatus::Errored(e.to_string()),
            };
        }
        s.meta.updated_at = chrono::Utc::now();
        s.meta.pid = None;

        // Update meta on disk
        let meta_path = Config::sessions_dir()
            .join(id.to_string())
            .join("meta.json");
        if let Ok(json) = serde_json::to_string_pretty(&s.meta) {
            let _ = std::fs::write(&meta_path, json);
        }
    }
}

// CLI helper functions - these call out to the running server via HTTP
pub async fn create_session_cli(
    config: &Config,
    label: Option<String>,
    cwd: Option<PathBuf>,
    tool: Option<String>,
    extra_args: Vec<String>,
) -> Result<Uuid> {
    let bind = crate::config::resolve_bind_address(&config.bind);
    let url = format!("http://{bind}:{}/api/sessions", config.port);
    let working_dir = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let default_name = working_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("session")
        .to_string();
    let name = label.unwrap_or(default_name);
    let body = serde_json::json!({
        "name": name,
        "tool": tool.unwrap_or_else(|| config.default_tool.clone()),
        "working_dir": working_dir,
        "extra_args": extra_args,
    });

    let client = reqwest::Client::new();
    let resp = client.post(&url).json(&body).send().await?;

    if resp.status().is_success() {
        let meta: SessionMeta = resp.json().await?;
        Ok(meta.id)
    } else {
        let text = resp.text().await?;
        anyhow::bail!("Failed to create session: {text}");
    }
}

pub async fn list_sessions_cli() -> Result<()> {
    let config = Config::load(None)?;
    let bind = crate::config::resolve_bind_address(&config.bind);
    let url = format!("http://{bind}:{}/api/sessions", config.port);

    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await?;

    if resp.status().is_success() {
        let sessions: Vec<SessionMeta> = resp.json().await?;
        if sessions.is_empty() {
            println!("No sessions");
        } else {
            for s in &sessions {
                println!(
                    "{} | {} | {} | {} | {}",
                    &s.id.to_string()[..8],
                    s.name,
                    s.tool,
                    s.status,
                    s.created_at.format("%H:%M:%S")
                );
            }
        }
    } else {
        let text = resp.text().await?;
        anyhow::bail!("Failed to list sessions: {text}");
    }
    Ok(())
}

pub async fn kill_session_cli(id: &str) -> Result<()> {
    let config = Config::load(None)?;
    let bind = crate::config::resolve_bind_address(&config.bind);
    let url = format!("http://{bind}:{}/api/sessions/{id}/stop", config.port);

    let client = reqwest::Client::new();
    let resp = client.post(&url).send().await?;

    if resp.status().is_success() {
        println!("Session stopped");
    } else {
        let text = resp.text().await?;
        anyhow::bail!("Failed to stop session: {text}");
    }
    Ok(())
}

pub async fn attach_session_cli(id: &str) -> Result<()> {
    use crossterm::terminal;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Find the attach socket in /tmp/lineforge/.
    // Retry a few times in case the socket hasn't been created yet (race condition).
    let sock_base = sock_dir();
    let mut sock_path = None;

    for attempt in 0..10 {
        let candidate = sock_base.join(format!("{id}.sock"));
        if candidate.exists() {
            sock_path = Some(candidate);
            break;
        }
        if attempt < 9 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    let sock_path = sock_path.ok_or_else(|| anyhow::anyhow!("No attach socket found for: {id}"))?;

    // Connect to Unix socket
    let stream = tokio::net::UnixStream::connect(&sock_path).await?;
    let (mut sock_reader, mut sock_writer) = tokio::io::split(stream);

    // Enable raw mode
    terminal::enable_raw_mode()?;

    // Ensure raw mode is disabled on exit
    let _guard = RawModeGuard;

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    // Read from socket -> stdout
    let write_stdout = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            match sock_reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
                Err(_) => break,
            }
        }
    });

    // Read from stdin -> socket
    let write_sock = tokio::spawn(async move {
        let mut buf = vec![0u8; 1024];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    // Ctrl+] (0x1d) to detach
                    if buf[..n].contains(&0x1d) {
                        break;
                    }
                    if sock_writer.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait for either direction to end
    tokio::select! {
        _ = write_stdout => {}
        _ = write_sock => {}
    }

    Ok(())
}

struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        // Print newline so shell prompt starts fresh
        println!();
    }
}

async fn run_attach_listener(
    sock_path: PathBuf,
    input_tx: mpsc::Sender<Vec<u8>>,
    broadcast_tx: tokio::sync::broadcast::Sender<crate::session::log::LogEntry>,
    sessions: Arc<RwLock<HashMap<Uuid, Arc<RwLock<LiveSession>>>>>,
    id: Uuid,
    sock_ready_tx: oneshot::Sender<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Clean up stale socket
    let _ = std::fs::remove_file(&sock_path);

    let listener = match tokio::net::UnixListener::bind(&sock_path) {
        Ok(l) => {
            let _ = sock_ready_tx.send(());
            l
        }
        Err(e) => {
            let _ = sock_ready_tx.send(());
            tracing::error!("Failed to bind attach socket {}: {e}", sock_path.display());
            return;
        }
    };

    tracing::debug!("Attach socket listening at {}", sock_path.display());

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("Attach socket accept error: {e}");
                continue;
            }
        };

        let input_tx = input_tx.clone();
        // Subscribe before reading the snapshot so we don't miss entries
        // produced between snapshot and first recv.
        let mut log_rx = broadcast_tx.subscribe();

        // Grab ring buffer snapshot for replay
        let snapshot = {
            let sessions_guard = sessions.read().await;
            if let Some(session) = sessions_guard.get(&id) {
                let s = session.read().await;
                s.log.snapshot()
            } else {
                Vec::new()
            }
        };

        tokio::spawn(async move {
            let (mut reader, mut writer) = tokio::io::split(stream);

            // Forward log output to attached client
            let write_handle = tokio::spawn(async move {
                // Replay ring buffer snapshot first
                for entry in &snapshot {
                    if writer.write_all(entry.data.as_bytes()).await.is_err() {
                        return;
                    }
                }
                let _ = writer.flush().await;

                // Then forward live broadcast
                loop {
                    match log_rx.recv().await {
                        Ok(entry) => {
                            if writer.write_all(entry.data.as_bytes()).await.is_err() {
                                break;
                            }
                            let _ = writer.flush().await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Forward attached client input to PTY
            let mut buf = vec![0u8; 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if input_tx.send(buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }

            write_handle.abort();
        });
    }
}
