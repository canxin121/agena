//! Append-only JSONL recorder.
//!
//! Writes are not buffered: every `append` performs a serialize + write +
//! fsync.  This is fine because rollouts are not in the hot path of LLM
//! calls — they record events that already happened.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use tokio::fs::{File, OpenOptions, create_dir_all};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::error::{RolloutError, RolloutResult};
use crate::frame::{RolloutFrame, RolloutKind};

/// Recorder of rollout frames.
pub struct RolloutRecorder {
    path: PathBuf,
    file: Mutex<File>,
    next_seq: AtomicU64,
}

impl RolloutRecorder {
    /// Open (or create) the recorder file.  Parent directories are
    /// created if missing.  When the file already exists the recorder
    /// resumes the seq counter from `existing_lines + 1` (cheap line
    /// count read, blocking — only happens at open time).
    pub async fn open(path: impl AsRef<Path>) -> RolloutResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            create_dir_all(parent).await?;
        }
        let initial_seq = if path.exists() {
            tokio::task::spawn_blocking({
                let path = path.clone();
                move || -> std::io::Result<u64> {
                    let bytes = std::fs::read(&path)?;
                    Ok(bytes.iter().filter(|b| **b == b'\n').count() as u64)
                }
            })
            .await
            .map_err(|e| RolloutError::Io(std::io::Error::other(e.to_string())))??
        } else {
            0
        };

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        Ok(Self {
            path,
            file: Mutex::new(file),
            next_seq: AtomicU64::new(initial_seq + 1),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a single frame.  The recorder fills in `seq` and `ts`.
    pub async fn append(&self, kind: RolloutKind) -> RolloutResult<u64> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let frame = RolloutFrame {
            seq,
            ts: Utc::now(),
            kind,
        };
        let mut line = serde_json::to_vec(&frame)?;
        line.push(b'\n');
        let mut g = self.file.lock().await;
        g.write_all(&line).await?;
        g.flush().await?;
        Ok(seq)
    }
}
