use anyhow::Result;

use crate::config::Config;
use crate::session::model::ToolKind;

/// Resolve the tool binary path
pub fn resolve_tool_path(config: &Config, tool: &ToolKind) -> Result<String> {
    if let Some(ref path) = config.tool_path {
        return Ok(path.clone());
    }
    Ok(tool.command_name().to_string())
}
