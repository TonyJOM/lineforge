use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_tool")]
    pub default_tool: String,
    pub tool_path: Option<String>,
    #[serde(default)]
    pub default_dirs: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub iterm_enabled: bool,
    #[serde(default = "default_log_retention")]
    pub log_retention_days: u32,
    #[serde(default = "default_max_log_lines")]
    pub max_log_lines: usize,
    #[serde(default)]
    pub yolo_mode: bool,
}

fn default_port() -> u16 {
    42067
}
fn default_bind() -> String {
    "tailscale".into()
}

/// Resolve a bind address string to an actual IP.
///
/// - `"tailscale"` → runs `tailscale ip -4` and returns the first IPv4 address,
///   falling back to `127.0.0.1` if Tailscale is unavailable.
/// - Anything else → returned as-is.
pub fn resolve_bind_address(bind: &str) -> String {
    if bind != "tailscale" {
        return bind.to_string();
    }

    match std::process::Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(ip) = stdout
                .lines()
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                tracing::info!("Resolved tailscale bind address: {ip}");
                return ip.to_string();
            }
            tracing::warn!("tailscale ip -4 returned empty output, falling back to 127.0.0.1");
            "127.0.0.1".into()
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("tailscale ip -4 failed: {stderr} — falling back to 127.0.0.1");
            "127.0.0.1".into()
        }
        Err(e) => {
            tracing::warn!(
                "tailscale command not found or failed: {e} — falling back to 127.0.0.1"
            );
            "127.0.0.1".into()
        }
    }
}
fn default_tool() -> String {
    "claude".into()
}
fn default_true() -> bool {
    true
}
fn default_log_retention() -> u32 {
    7
}
fn default_max_log_lines() -> usize {
    10_000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind: default_bind(),
            default_tool: default_tool(),
            tool_path: None,
            default_dirs: Vec::new(),
            iterm_enabled: true,
            log_retention_days: default_log_retention(),
            max_log_lines: default_max_log_lines(),
            yolo_mode: false,
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lineforge")
    }

    pub fn data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lineforge")
    }

    pub fn sessions_dir() -> PathBuf {
        Self::data_dir().join("sessions")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn load(path: Option<&PathBuf>) -> Result<Self> {
        let config_path = path.cloned().unwrap_or_else(Self::config_path);

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
            let config: Config = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config: {}", config_path.display()))?;
            Ok(config)
        } else {
            let config = Config::default();
            config.save(&config_path)?;
            tracing::info!("Created default config at {}", config_path.display());
            Ok(config)
        }
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir: {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))?;
        Ok(())
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(Self::config_dir())?;
        std::fs::create_dir_all(Self::data_dir())?;
        std::fs::create_dir_all(Self::sessions_dir())?;
        Ok(())
    }
}
