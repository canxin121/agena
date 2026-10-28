//! Scoped live UI heads. A prepared tool result owns the initial document; a
//! private per-session record owns explicitly versioned updates. Atomic rename
//! and the existing cross-process mutation lock protect compare-and-swap.
use super::storage::{
    PluginStorageError, StorageLocator, default_storage_root, namespace_path,
    write_secure_file_unlocked,
};
use agena_domain::{
    InlineDocument, InlinePhase, InlineSnapshot, InlineUpdate, inline_reference_tool_id,
};
use agena_plugin_host::sdk::host_api::{HostStorageScope, HostStorageVisibility};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
};
use tokio::sync::watch;

static WATCHES: LazyLock<Mutex<BTreeMap<PathBuf, Weak<watch::Sender<u64>>>>> =
    LazyLock::new(Default::default);
/// Update validation and CAS failures are distinct from storage I/O/corruption.
#[derive(Debug, thiserror::Error)]
pub enum InlineUiUpdateError {
    #[error("revision conflict: expected {expected}, current {current}; read and reconcile")]
    RevisionConflict { expected: u64, current: u64 },
    #[error("invalid UI update: {0}")]
    Invalid(String),
    #[error("UI not found in this session")]
    NotFound,
    #[error(transparent)]
    Storage(#[from] PluginStorageError),
}

#[derive(Clone)]
pub struct InlineUiStore {
    root: PathBuf,
}
impl Default for InlineUiStore {
    fn default() -> Self {
        Self::new(default_storage_root())
    }
}
impl InlineUiStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    fn path(&self, session_id: i64, reference: &str) -> Result<PathBuf, PluginStorageError> {
        if session_id <= 0 || inline_reference_tool_id(reference).is_none() {
            return Err(PluginStorageError::Data(
                "invalid live UI scope/reference".into(),
            ));
        }
        let locator = StorageLocator::new(
            HostStorageScope::Session,
            HostStorageVisibility::Private,
            "agena.inline_ui"
                .parse()
                .map_err(|_| PluginStorageError::Data("invalid plugin identity".into()))?,
            Some(session_id),
            None,
        )?;
        namespace_path(&self.root, &locator, &format!("live-{reference}"))
    }
    pub fn read(
        &self,
        session: i64,
        reference: &str,
    ) -> Result<Option<InlineSnapshot>, PluginStorageError> {
        let value = read(&self.path(session, reference)?)?;
        if value
            .as_ref()
            .is_some_and(|snapshot| snapshot.reference != reference)
        {
            return Err(PluginStorageError::Data(
                "live UI record identity mismatch".into(),
            ));
        }
        Ok(value)
    }
    pub fn create(
        &self,
        session: i64,
        reference: &str,
        document: InlineDocument,
        call_id: i64,
        phase: InlinePhase,
    ) -> Result<InlineSnapshot, PluginStorageError> {
        document.validate().map_err(PluginStorageError::Data)?;
        if inline_reference_tool_id(reference) != Some(call_id) {
            return Err(PluginStorageError::Data(
                "create must belong to the current tool invocation".into(),
            ));
        }
        let path = self.path(session, reference)?;
        let snapshot =
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
                if let Some(existing) = read(&path)? {
                    return Ok(existing);
                }
                let snapshot = InlineSnapshot {
                    reference: reference.into(),
                    revision: 1,
                    phase,
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                    last_call_id: call_id,
                    document,
                };
                write(&path, &snapshot)?;
                Ok::<_, PluginStorageError>(snapshot)
            })??;
        publish(&path, snapshot.revision);
        Ok(snapshot)
    }
    pub fn update(
        &self,
        session: i64,
        update: &InlineUpdate,
        call_id: i64,
    ) -> Result<InlineSnapshot, InlineUiUpdateError> {
        if call_id <= 0 {
            return Err(InlineUiUpdateError::Invalid(
                "update requires an active invocation".into(),
            ));
        }
        let path = self.path(session, &update.reference)?;
        let (snapshot, changed) =
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
                let current = read(&path)?.ok_or(InlineUiUpdateError::NotFound)?;
                // An identical invocation can be retried, but never silently rebase
                // an unrelated caller on the head's revision.
                if !(current.last_call_id == call_id && call_id > 0)
                    && update.expected_revision != current.revision
                {
                    return Err(InlineUiUpdateError::RevisionConflict {
                        expected: update.expected_revision,
                        current: current.revision,
                    });
                }
                let next = current
                    .apply(update, call_id, chrono::Utc::now().timestamp_millis())
                    .map_err(InlineUiUpdateError::Invalid)?;
                let changed = next != current;
                if changed {
                    write(&path, &next)?;
                }
                Ok::<_, InlineUiUpdateError>((next, changed))
            })
            .map_err(|error| InlineUiUpdateError::Storage(error.into()))??;
        if changed {
            publish(&path, snapshot.revision);
        }
        Ok(snapshot)
    }
    /// Subscribe before reading, so a concurrent commit cannot be missed.
    pub fn subscribe(
        &self,
        session: i64,
        reference: &str,
    ) -> Result<InlineUiSubscription, PluginStorageError> {
        let path = self.path(session, reference)?;
        let mut watches = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
        watches.retain(|_, v| v.strong_count() > 0);
        let sender = watches
            .get(&path)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let (sender, _) = watch::channel(0);
                let sender = Arc::new(sender);
                watches.insert(path, Arc::downgrade(&sender));
                sender
            });
        Ok(InlineUiSubscription {
            receiver: sender.subscribe(),
            _sender: sender,
        })
    }
}
pub struct InlineUiSubscription {
    receiver: watch::Receiver<u64>,
    _sender: Arc<watch::Sender<u64>>,
}
impl InlineUiSubscription {
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.receiver.changed().await
    }
}
fn publish(path: &Path, revision: u64) {
    let sender = WATCHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .and_then(Weak::upgrade);
    if let Some(sender) = sender {
        sender.send_replace(revision);
    }
}
fn read(path: &Path) -> Result<Option<InlineSnapshot>, PluginStorageError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_file() || metadata.len() > 128 * 1024 {
        return Err(PluginStorageError::Data(
            "invalid/oversized live UI record".into(),
        ));
    }
    let snapshot: InlineSnapshot = serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|e| PluginStorageError::Data(e.to_string()))?;
    snapshot.validate().map_err(PluginStorageError::Data)?;
    Ok(Some(snapshot))
}
fn write(path: &Path, snapshot: &InlineSnapshot) -> Result<(), PluginStorageError> {
    let bytes =
        serde_json::to_vec(snapshot).map_err(|e| PluginStorageError::Data(e.to_string()))?;
    if bytes.len() > 128 * 1024 {
        return Err(PluginStorageError::Data(
            "UI snapshot exceeds storage budget".into(),
        ));
    }
    write_secure_file_unlocked(path, &bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn stateful_ui_store_is_atomic_revision_checked_scoped_and_live() {
        let dir = tempfile::tempdir().unwrap();
        let store = InlineUiStore::new(dir.path());
        let reference = "ui_7_0123456789abcdef0123456789abcdef";
        let doc = InlineDocument::from_value(
            json!({"version":1,"blocks":[{"type":"paragraph","id":"p","text":"First"}]}),
        )
        .unwrap();
        let mut watch = store.subscribe(1, reference).unwrap();
        let first = store
            .create(1, reference, doc.clone(), 7, InlinePhase::Complete)
            .unwrap();
        assert_eq!(first.revision, 1);
        watch.changed().await.unwrap();
        assert!(store.read(2, reference).unwrap().is_none());
        let update = InlineUpdate {
            reference: reference.into(),
            expected_revision: 1,
            document: Some(
                InlineDocument::from_value(
                    json!({"version":1,"blocks":[{"type":"paragraph","id":"p","text":"Second"}]}),
                )
                .unwrap(),
            ),
            patch: None,
            phase: Some(InlinePhase::Complete),
        };
        let next = store.update(1, &update, 8).unwrap();
        assert_eq!(next.revision, 2);
        watch.changed().await.unwrap();
        assert_eq!(
            store.update(1, &update, 8).unwrap().revision,
            2,
            "same invocation is idempotent"
        );
        assert!(
            store
                .update(1, &update, 9)
                .unwrap_err()
                .to_string()
                .contains("conflict")
        );
        assert!(store.update(2, &update, 9).is_err());
        assert_eq!(
            InlineUiStore::new(dir.path())
                .read(1, reference)
                .unwrap()
                .unwrap(),
            next,
            "restart keeps current revision"
        );
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use agena_domain::{InlineBlock, InlinePanelPatch, UiState};
    use serde_json::json;
    #[test]
    fn live_panel_updates_are_atomic_and_concurrent_writers_cannot_overwrite_each_other() {
        let directory = tempfile::tempdir().unwrap();
        let store = InlineUiStore::new(directory.path());
        let reference = "ui_17_0123456789abcdef0123456789abcdef";
        let document = InlineDocument::from_value(
            serde_json::from_str(include_str!(
                "../../../../examples/inline-ui/live-dashboard.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let first = store
            .create(1, reference, document, 17, InlinePhase::Working)
            .unwrap();
        let InlineBlock::Panel { program, .. } = &first.document.blocks[0] else {
            panic!("panel")
        };
        let state = program
            .input(&UiState::new(), "channel", json!("paid"))
            .unwrap()
            .state;
        let patch = InlineUpdate {
            reference: reference.into(),
            expected_revision: 1,
            document: None,
            patch: Some(InlinePanelPatch {
                block_id: "workspace".into(),
                data: [("processed".into(), json!(6))].into(),
                ..Default::default()
            }),
            phase: Some(InlinePhase::Complete),
        };
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let jobs = (18..20)
            .map(|id| {
                let store = store.clone();
                let patch = patch.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.update(1, &patch, id)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = jobs
            .into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let current = store.read(1, reference).unwrap().unwrap();
        assert_eq!(current.revision, 2);
        let InlineBlock::Panel { program, .. } = &current.document.blocks[0] else {
            panic!("panel")
        };
        let view = program.render(&state).unwrap();
        assert_eq!(view.state["channel"], json!("paid"));
        assert_eq!(
            view.root.find("agentProgress").unwrap().props["value"],
            json!(6)
        );
        let bad = InlineUpdate {
            expected_revision: 2,
            patch: Some(InlinePanelPatch {
                block_id: "workspace".into(),
                remove: vec!["root".into()],
                ..Default::default()
            }),
            ..patch
        };
        assert!(store.update(1, &bad, 20).is_err());
        assert_eq!(store.read(1, reference).unwrap().unwrap(), current);
    }
}
