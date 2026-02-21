use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Parser)]
#[command(name = "forge", version, about = "Lineforge - AI session manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the backend server and web UI
    Serve {
        /// Port to listen on
        #[arg(long)]
        port: Option<u16>,

        /// Address to bind to
        #[arg(long)]
        bind: Option<String>,

        /// Path to config file
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Create a new session
    New {
        /// Session label
        #[arg(long)]
        label: Option<String>,

        /// Working directory
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Tool to use (claude or codex)
        #[arg(long)]
        tool: Option<String>,

        /// Attach terminal immediately
        #[arg(long)]
        attach: bool,

        /// Skip auto-opening iTerm2 tab
        #[arg(long)]
        no_iterm: bool,

        /// Extra arguments passed to the CLI tool
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    /// Attach terminal to a session PTY
    Attach {
        /// Session ID (UUID or prefix)
        id: String,
    },

    /// List all sessions
    List,

    /// Stop a session
    Kill {
        /// Session ID (UUID or prefix)
        id: String,
    },

    /// Interactive configuration wizard
    Setup,
}

pub async fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Serve { port, bind, config } => {
            let mut cfg = Config::load(config.as_ref())?;
            if let Some(p) = port {
                cfg.port = p;
            }
            if let Some(b) = bind {
                cfg.bind = b;
            }
            cfg.ensure_dirs()?;
            crate::server::start(cfg).await?;
        }
        Command::New {
            label,
            cwd,
            tool,
            attach,
            no_iterm,
            extra_args,
        } => {
            let cfg = Config::load(None)?;
            crate::session::manager::create_session_cli(
                &cfg, label, cwd, tool, attach, no_iterm, extra_args,
            )
            .await?;
        }
        Command::Attach { id } => {
            crate::session::manager::attach_session_cli(&id).await?;
        }
        Command::List => {
            crate::session::manager::list_sessions_cli().await?;
        }
        Command::Kill { id } => {
            crate::session::manager::kill_session_cli(&id).await?;
        }
        Command::Setup => {
            println!("Setup wizard coming in Milestone 6");
        }
    }
    Ok(())
}
