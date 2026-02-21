use std::collections::VecDeque;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub data: String,
}

pub struct SessionLog {
    buffer: VecDeque<LogEntry>,
    max_lines: usize,
    pub broadcast_tx: broadcast::Sender<LogEntry>,
    log_file: Option<PathBuf>,
}

impl SessionLog {
    pub fn new(max_lines: usize, log_file: Option<PathBuf>) -> Self {
        let (broadcast_tx, _) = broadcast::channel(1000);
        Self {
            buffer: VecDeque::with_capacity(max_lines),
            max_lines,
            broadcast_tx,
            log_file,
        }
    }

    pub fn push(&mut self, data: String) {
        let entry = LogEntry {
            timestamp: Utc::now(),
            data,
        };

        if self.buffer.len() >= self.max_lines {
            self.buffer.pop_front();
        }
        self.buffer.push_back(entry.clone());

        // Best-effort broadcast; receivers may have been dropped
        let _ = self.broadcast_tx.send(entry.clone());

        // Append to log file if configured
        if let Some(ref path) = self.log_file {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                use std::io::Write;
                let _ = writeln!(file, "{}", entry.data);
            }
        }
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.buffer.iter().cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.broadcast_tx.subscribe()
    }
}
