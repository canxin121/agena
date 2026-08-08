//! Reader + directory enumerator.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{RolloutError, RolloutResult};
use crate::frame::{RolloutFrame, RolloutKind, SessionMeta};

/// Reader over rollout recordings.
pub struct RolloutReader {
    path: PathBuf,
}

impl RolloutReader {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read every frame eagerly.  Callers that need lazy/iterator
    /// access can use [`Self::iter`].
    pub fn read_all(&self) -> RolloutResult<Vec<RolloutFrame>> {
        self.iter()?.collect()
    }

    pub fn iter(&self) -> RolloutResult<FrameIter> {
        let f = File::open(&self.path)?;
        Ok(FrameIter {
            inner: BufReader::new(f).lines(),
        })
    }

    pub fn session_meta(&self) -> RolloutResult<SessionMeta> {
        let f = File::open(&self.path)?;
        let mut lines = BufReader::new(f).lines();
        let first = lines
            .next()
            .ok_or_else(|| RolloutError::Malformed("empty rollout".into()))?
            .map_err(RolloutError::Io)?;
        let frame: RolloutFrame = serde_json::from_str(&first)?;
        match frame.kind {
            RolloutKind::SessionMeta(m) => Ok(m),
            _ => Err(RolloutError::Malformed(
                "first frame must be SessionMeta".into(),
            )),
        }
    }
}

/// Iterator over rollout frames.
pub struct FrameIter {
    inner: std::io::Lines<BufReader<File>>,
}

impl Iterator for FrameIter {
    type Item = RolloutResult<RolloutFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let raw = self.inner.next()?;
            let line = match raw {
                Ok(l) => l,
                Err(e) => return Some(Err(RolloutError::Io(e))),
            };
            if line.trim().is_empty() {
                continue;
            }
            return Some(serde_json::from_str(&line).map_err(Into::into));
        }
    }
}

/// Recurse `root` and return paths of every `.jsonl` file we treat as a
/// rollout.  Order is filesystem-defined; callers should sort by mtime
/// or session id depending on their needs.
pub fn list_sessions(root: impl AsRef<Path>) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .map(|e| e.into_path())
        .collect()
}
