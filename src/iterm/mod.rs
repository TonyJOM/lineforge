use std::path::Path;

use anyhow::Result;
use uuid::Uuid;

use crate::error::ForgeError;

/// Open a new iTerm2 window, cd to `working_dir`, and run `forge attach <session_id>`
pub fn open_in_iterm(session_id: Uuid, working_dir: &Path) -> Result<()> {
    let dir = working_dir.display();
    let script = format!(
        r#"
        tell application "iTerm2"
            activate
            set newWindow to (create window with default profile)
            tell current session of newWindow
                write text "cd {dir} && forge attach {session_id}"
            end tell
        end tell
        "#
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| ForgeError::Iterm(format!("Failed to run osascript: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForgeError::Iterm(format!("AppleScript error: {stderr}")).into());
    }

    Ok(())
}
