//! Synchronize disk snapshots and keep a query on the same document version.

use super::*;
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, Range, TextDocumentContentChangeEvent, TextDocumentItem,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncSaveOptions,
    VersionedTextDocumentIdentifier,
};
use tokio::time::Instant;

const MAX_OPEN_DOCUMENTS: usize = 64;

#[derive(Clone)]
pub(super) struct OpenDocState {
    pub version: i32,
    pub publication_at_sync: u64,
    content_hash: u64,
    end: Position,
    last_used: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    Disk,
    Reopen,
    Full,
    Incremental,
}

impl SyncMode {
    fn from_server(server: Option<&InitializeResult>) -> Self {
        let Some(sync) = server.and_then(|server| server.capabilities.text_document_sync.as_ref())
        else {
            return Self::Disk;
        };
        let kind = match sync {
            TextDocumentSyncCapability::Kind(kind) => *kind,
            TextDocumentSyncCapability::Options(options) => {
                if options.open_close == Some(false) {
                    return Self::Disk;
                }
                if options.open_close == Some(true)
                    && options
                        .change
                        .is_none_or(|kind| kind == TextDocumentSyncKind::NONE)
                {
                    return Self::Reopen;
                }
                options.change.unwrap_or(TextDocumentSyncKind::NONE)
            }
        };
        match kind {
            TextDocumentSyncKind::FULL => Self::Full,
            TextDocumentSyncKind::INCREMENTAL => Self::Incremental,
            _ => Self::Disk,
        }
    }
}

impl LspClient {
    /// Synchronize the current contents of a document.
    pub async fn sync_document(&self, uri: Uri, text: String, language_id: &str) -> LspResult<()> {
        self.sync_document_with_receipt(uri, text, language_id)
            .await?;
        Ok(())
    }

    /// Synchronize a document and report whether its contents changed.
    pub async fn sync_document_with_status(
        &self,
        uri: Uri,
        text: String,
        language_id: &str,
    ) -> LspResult<DocumentSyncStatus> {
        Ok(self
            .sync_document_with_receipt(uri, text, language_id)
            .await?
            .status)
    }

    /// Synchronize a document and return its version for diagnostic matching.
    pub async fn sync_document_with_receipt(
        &self,
        uri: Uri,
        text: String,
        language_id: &str,
    ) -> LspResult<DocumentReceipt> {
        let _gate = self.inner.document_sync.lock().await;
        self.sync_locked(uri, text, language_id).await
    }

    /// Hold the client's document gate through a query on the synchronized snapshot.
    pub async fn with_document<T, F>(
        &self,
        uri: Uri,
        text: String,
        language_id: &str,
        query: impl FnOnce(DocumentReceipt) -> F,
    ) -> LspResult<T>
    where
        F: std::future::Future<Output = LspResult<T>> + Send,
    {
        let _gate = self.inner.document_sync.lock().await;
        let receipt = self.sync_locked(uri, text, language_id).await?;
        query(receipt).await
    }

    pub(super) fn syncs_open_documents(&self) -> bool {
        SyncMode::from_server(self.server_info().as_deref()) != SyncMode::Disk
    }

    /// Whether a saved document needs a notification, and whether it must carry text.
    pub fn save_includes_text(&self) -> Option<bool> {
        let server = self.server_info()?;
        let TextDocumentSyncCapability::Options(options) =
            server.capabilities.text_document_sync.as_ref()?
        else {
            return None;
        };
        match options.save.as_ref()? {
            TextDocumentSyncSaveOptions::Supported(true) => Some(false),
            TextDocumentSyncSaveOptions::SaveOptions(options) => {
                Some(options.include_text.unwrap_or_default())
            }
            TextDocumentSyncSaveOptions::Supported(false) => None,
        }
    }

    /// Notify a server after a known disk write, inside the document gate.
    pub async fn document_saved(&self, uri: Uri, text: Option<String>) -> LspResult<()> {
        self.notify(
            "textDocument/didSave",
            DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
                text,
            },
        )
        .await
    }

    async fn sync_locked(
        &self,
        uri: Uri,
        text: String,
        language_id: &str,
    ) -> LspResult<DocumentReceipt> {
        self.ensure_alive()?;
        let hash = hash_str(&text);
        let previous = self.inner.open_docs.get_mut(&uri).map(|mut doc| {
            doc.last_used = Instant::now();
            doc.value().clone()
        });
        if let Some(previous) = previous
            .as_ref()
            .filter(|previous| previous.content_hash == hash)
        {
            return Ok(DocumentReceipt {
                status: DocumentSyncStatus::Unchanged,
                version: previous.version,
            });
        }
        if previous.is_none() && self.inner.open_docs.len() >= MAX_OPEN_DOCUMENTS {
            let oldest = self
                .inner
                .open_docs
                .iter()
                .min_by_key(|entry| entry.last_used)
                .map(|entry| entry.key().clone());
            if let Some(oldest) = oldest {
                self.close_locked(oldest).await?;
            }
        }
        let version =
            i32::try_from(self.inner.document_sequence.fetch_add(1, Ordering::Relaxed))
                .map_err(|_| LspError::Protocol("LSP document version space exhausted".into()))?;
        let mode = SyncMode::from_server(self.server_info().as_deref());
        let reopen = mode == SyncMode::Reopen && previous.is_some();
        let publication_at_sync = *self.inner.diagnostics_changed.borrow();
        self.inner.open_docs.insert(
            uri.clone(),
            OpenDocState {
                version,
                content_hash: hash,
                publication_at_sync,
                end: if mode == SyncMode::Incremental {
                    document_end(&text)
                } else {
                    Position::default()
                },
                last_used: Instant::now(),
            },
        );
        let mut guard = SyncGuard {
            inner: &self.inner,
            uri: &uri,
            previous: previous.clone(),
            interrupted: true,
            committed: false,
        };
        if reopen {
            self.notify_close(uri.clone()).await?;
            guard.previous = None;
        }
        let sent = match (mode, previous.as_ref()) {
            (SyncMode::Disk, _) => Ok(()),
            (SyncMode::Reopen, _) | (_, None) => {
                self.notify(
                    "textDocument/didOpen",
                    DidOpenTextDocumentParams {
                        text_document: TextDocumentItem {
                            language_id: language_id.to_owned(),
                            uri: uri.clone(),
                            version,
                            text,
                        },
                    },
                )
                .await
            }
            (_, Some(previous)) => {
                self.notify(
                    "textDocument/didChange",
                    DidChangeTextDocumentParams {
                        text_document: VersionedTextDocumentIdentifier {
                            uri: uri.clone(),
                            version,
                        },
                        content_changes: vec![TextDocumentContentChangeEvent {
                            range: (mode == SyncMode::Incremental)
                                .then(|| Range::new(Position::default(), previous.end)),
                            range_length: None,
                            text,
                        }],
                    },
                )
                .await
            }
        };
        if matches!(sent, Err(LspError::MessageTooLarge { .. })) {
            guard.interrupted = false;
        }
        sent?;
        guard.committed = true;
        let status = if previous.is_some() {
            DocumentSyncStatus::Changed
        } else {
            DocumentSyncStatus::Opened
        };
        Ok(DocumentReceipt { status, version })
    }

    /// Release an open document and the associated cached diagnostics.
    pub async fn close_document(&self, uri: Uri) -> LspResult<()> {
        let _gate = self.inner.document_sync.lock().await;
        self.close_locked(uri).await
    }

    async fn close_locked(&self, uri: Uri) -> LspResult<()> {
        if let Some((_, previous)) = self.inner.open_docs.remove(&uri) {
            let mut guard = SyncGuard {
                inner: &self.inner,
                uri: &uri,
                previous: Some(previous),
                interrupted: true,
                committed: false,
            };
            if self.syncs_open_documents() {
                self.notify_close(uri.clone()).await?;
            }
            guard.committed = true;
        }
        self.inner.diagnostics.remove(&uri);
        self.inner
            .diagnostics_changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        Ok(())
    }

    async fn notify_close(&self, uri: Uri) -> LspResult<()> {
        self.notify(
            "textDocument/didClose",
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
            },
        )
        .await
    }
}

struct SyncGuard<'a> {
    inner: &'a Inner,
    uri: &'a Uri,
    previous: Option<OpenDocState>,
    interrupted: bool,
    committed: bool,
}

impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if self.interrupted {
            // A canceled send may already have reached the peer. Its document
            // state cannot be reconstructed safely from the local cache.
            self.inner
                .disconnect("LSP document synchronization was interrupted");
        }
        if let Some(previous) = self.previous.take() {
            self.inner.open_docs.insert(self.uri.clone(), previous);
        } else {
            self.inner.open_docs.remove(self.uri);
        }
        self.inner
            .diagnostics_changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

/// Check a zero-based UTF-16 position against the exact document being sent.
pub fn valid_position(text: &str, position: Position) -> bool {
    let Some(line) = document_lines(text).nth(position.line as usize) else {
        return false;
    };
    let mut remaining = position.character as usize;
    for ch in line.chars() {
        if remaining == 0 {
            return true;
        }
        let Some(next) = remaining.checked_sub(ch.len_utf16()) else {
            return false;
        };
        remaining = next;
    }
    remaining == 0
}

fn document_end(text: &str) -> Position {
    let (line, last) = document_lines(text)
        .enumerate()
        .last()
        .expect("one line minimum");
    Position::new(line as u32, last.encode_utf16().count() as u32)
}

fn document_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut start = Some(0);
    let mut breaks = text.match_indices(['\n', '\r']).peekable();
    std::iter::from_fn(move || {
        let from = start.take()?;
        match breaks.next() {
            Some((at, "\r")) if breaks.peek() == Some(&(at + 1, "\n")) => {
                breaks.next();
                start = Some(at + 2);
                Some(&text[from..at])
            }
            Some((at, _)) => {
                start = Some(at + 1);
                Some(&text[from..at])
            }
            None => Some(&text[from..]),
        }
    })
}
