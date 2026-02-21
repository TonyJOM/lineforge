use thiserror::Error;

#[derive(Error, Debug)]
pub enum ForgeError {
    #[error("Session not found: {0}")]
    SessionNotFound(uuid::Uuid),

    #[error("Session already stopped: {0}")]
    SessionAlreadyStopped(uuid::Uuid),

    #[error("PTY error: {0}")]
    Pty(String),

    #[error("Config error: {0}")]
    #[allow(dead_code)]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("iTerm2 error: {0}")]
    Iterm(String),
}
