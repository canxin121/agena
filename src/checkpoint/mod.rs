mod git;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use path_clean::PathClean;
use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{message::Message, session::Session};

use self::git::{GitSnapshotBackend, GitSnapshotCheckpoint};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionRestoreMode {
    Filesystem,
    Conversation,
    Both,
}

impl SessionRestoreMode {
    pub const fn restores_filesystem(self) -> bool {
        matches!(self, Self::Filesystem | Self::Both)
    }

    pub const fn restores_conversation(self) -> bool {
        matches!(self, Self::Conversation | Self::Both)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
pub struct SessionRestorePointSnapshot {
    pub conversation: ConversationCheckpoint,
    pub filesystem: FilesystemCheckpoint,
}

impl SessionRestorePointSnapshot {
    pub fn new(session: Session, messages: Vec<Message>, filesystem: FilesystemCheckpoint) -> Self {
        Self {
            conversation: ConversationCheckpoint { session, messages },
            filesystem,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationCheckpoint {
    pub session: Session,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilesystemCheckpoint {
    Journal(FileJournalCheckpoint),
    Composite {
        journal: FileJournalCheckpoint,
        git: GitSnapshotCheckpoint,
    },
}

impl FilesystemCheckpoint {
    pub fn journal(&self) -> &FileJournalCheckpoint {
        match self {
            Self::Journal(journal) | Self::Composite { journal, .. } => journal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileJournalCheckpoint {
    pub entries: Vec<FileJournalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileJournalEntry {
    pub path: TrackedPath,
    pub prior_state: JournalFileState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JournalFileState {
    Missing,
    RegularFile { blob_hash: String, readonly: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum TrackedPath {
    WorkspaceRelative { path: String },
    Absolute { path: String },
}

impl TrackedPath {
    pub fn from_absolute(workspace_root: &Path, absolute: &Path) -> Self {
        if let Ok(relative) = absolute.strip_prefix(workspace_root) {
            let path = normalize_path_text(relative);
            return Self::WorkspaceRelative { path };
        }

        Self::Absolute {
            path: normalize_path_text(absolute),
        }
    }

    pub fn resolve(&self, workspace_root: &Path) -> PathBuf {
        match self {
            Self::WorkspaceRelative { path } => workspace_root.join(path),
            Self::Absolute { path } => PathBuf::from(path),
        }
    }

    pub fn display_path(&self) -> &str {
        match self {
            Self::WorkspaceRelative { path } | Self::Absolute { path } => path.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemCheckpointCapture {
    pub snapshot: FilesystemCheckpoint,
    pub blobs: Vec<CheckpointBlob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointBlob {
    pub hash: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemRestoreReport {
    pub restored_paths: Vec<String>,
    pub used_git_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRestorePoint {
    pub id: i64,
    pub session_id: i64,
    pub upto_seq: i64,
    pub call_id: Option<i64>,
    pub message_id: Option<i64>,
    pub operation_id: Option<String>,
    pub snapshot: SessionRestorePointSnapshot,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRestoreRequest {
    pub session_id: i64,
    pub restore_point_id: Option<i64>,
    pub mode: SessionRestoreMode,
}

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing checkpoint blob: {0}")]
    MissingBlob(String),
    #[error("invalid tracked path: {0}")]
    InvalidPath(String),
    #[error("git snapshot error: {0}")]
    Git(String),
}

pub fn capture_for_paths(
    workspace_root: &Path,
    paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<FilesystemCheckpointCapture, CheckpointError> {
    let mut unique_paths = BTreeSet::new();
    for path in paths {
        unique_paths.insert(path.as_ref().to_string());
    }

    let mut blobs = BTreeMap::<String, Vec<u8>>::new();
    let mut entries = Vec::with_capacity(unique_paths.len());
    for raw_path in unique_paths {
        let absolute = absolute_tracked_path(workspace_root, raw_path.as_str())?;
        let tracked_path = TrackedPath::from_absolute(workspace_root, absolute.as_path());
        let prior_state = capture_journal_file_state(absolute.as_path(), &mut blobs)?;
        entries.push(FileJournalEntry {
            path: tracked_path,
            prior_state,
        });
    }

    let journal = FileJournalCheckpoint { entries };
    let git = GitSnapshotBackend::capture(workspace_root).ok().flatten();
    let snapshot = match git {
        Some(git) => FilesystemCheckpoint::Composite { journal, git },
        None => FilesystemCheckpoint::Journal(journal),
    };

    Ok(FilesystemCheckpointCapture {
        snapshot,
        blobs: blobs
            .into_iter()
            .map(|(hash, bytes)| CheckpointBlob { hash, bytes })
            .collect(),
    })
}

pub fn restore_filesystem<F>(
    workspace_root: &Path,
    snapshot: &FilesystemCheckpoint,
    mut load_blob: F,
) -> Result<FilesystemRestoreReport, CheckpointError>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>, CheckpointError>,
{
    let mut used_git_snapshot = false;
    match snapshot {
        FilesystemCheckpoint::Journal(journal) => {
            let restored_paths = restore_journal(workspace_root, journal, &mut load_blob)?;
            Ok(FilesystemRestoreReport {
                restored_paths,
                used_git_snapshot,
            })
        }
        FilesystemCheckpoint::Composite { journal, git } => {
            if let Err(err) = GitSnapshotBackend::restore(git) {
                tracing::warn!(error = %err, "git snapshot restore failed, falling back to journal");
            } else {
                used_git_snapshot = true;
            }

            let restored_paths = restore_journal(workspace_root, journal, &mut load_blob)?;
            Ok(FilesystemRestoreReport {
                restored_paths,
                used_git_snapshot,
            })
        }
    }
}

fn capture_journal_file_state(
    absolute: &Path,
    blobs: &mut BTreeMap<String, Vec<u8>>,
) -> Result<JournalFileState, CheckpointError> {
    match fs::metadata(absolute) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(CheckpointError::InvalidPath(format!(
                    "checkpoint capture only supports files: {}",
                    absolute.display()
                )));
            }

            let bytes = fs::read(absolute)?;
            let hash = sha256_bytes(&bytes);
            blobs.entry(hash.clone()).or_insert(bytes);
            Ok(JournalFileState::RegularFile {
                blob_hash: hash,
                readonly: metadata.permissions().readonly(),
            })
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(JournalFileState::Missing),
        Err(err) => Err(CheckpointError::Io(err)),
    }
}

fn restore_journal<F>(
    workspace_root: &Path,
    journal: &FileJournalCheckpoint,
    load_blob: &mut F,
) -> Result<Vec<String>, CheckpointError>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>, CheckpointError>,
{
    let mut restored_paths = Vec::with_capacity(journal.entries.len());
    for entry in &journal.entries {
        let absolute = entry.path.resolve(workspace_root);
        match &entry.prior_state {
            JournalFileState::Missing => match fs::metadata(absolute.as_path()) {
                Ok(metadata) if metadata.is_file() => {
                    fs::remove_file(absolute.as_path())?;
                }
                Ok(metadata) if metadata.is_dir() => {
                    fs::remove_dir_all(absolute.as_path())?;
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(CheckpointError::Io(err)),
            },
            JournalFileState::RegularFile {
                blob_hash,
                readonly,
            } => {
                let Some(bytes) = load_blob(blob_hash.as_str())? else {
                    return Err(CheckpointError::MissingBlob(blob_hash.clone()));
                };
                ensure_parent_dir(absolute.as_path())?;
                fs::write(absolute.as_path(), &bytes)?;
                let mut permissions = fs::metadata(absolute.as_path())?.permissions();
                permissions.set_readonly(*readonly);
                fs::set_permissions(absolute.as_path(), permissions)?;
            }
        }

        restored_paths.push(entry.path.display_path().to_string());
    }

    Ok(restored_paths)
}

fn absolute_tracked_path(
    workspace_root: &Path,
    raw_path: &str,
) -> Result<PathBuf, CheckpointError> {
    let path = PathBuf::from(raw_path);
    let absolute = if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    };
    Ok(absolute.clean())
}

fn ensure_parent_dir(path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn normalize_path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn sha256_bytes(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(nibble_to_hex((byte >> 4) & 0x0f));
        out.push(nibble_to_hex(byte & 0x0f));
    }
    out
}

fn nibble_to_hex(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'a' + (v - 10)) as char,
        _ => '0',
    }
}
