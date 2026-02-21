use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Stopped,
    Errored(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Claude,
    Codex,
}

impl ToolKind {
    pub fn command_name(&self) -> &str {
        match self {
            ToolKind::Claude => "claude",
            ToolKind::Codex => "codex",
        }
    }
}

impl std::fmt::Display for ToolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolKind::Claude => write!(f, "claude"),
            ToolKind::Codex => write!(f, "codex"),
        }
    }
}

impl std::str::FromStr for ToolKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(ToolKind::Claude),
            "codex" => Ok(ToolKind::Codex),
            other => Err(format!(
                "Unknown tool: {other}. Expected 'claude' or 'codex'"
            )),
        }
    }
}

impl SessionStatus {
    pub fn css_class(&self) -> &str {
        match self {
            SessionStatus::Running => "running",
            SessionStatus::Stopped => "stopped",
            SessionStatus::Errored(_) => "errored",
        }
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Stopped => write!(f, "stopped"),
            SessionStatus::Errored(e) => write!(f, "errored: {e}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: Uuid,
    pub name: String,
    pub tool: ToolKind,
    pub status: SessionStatus,
    pub working_dir: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub pid: Option<u32>,
    pub extra_args: Vec<String>,
}
