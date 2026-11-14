//! Live cursors and seekable transfers for request/response transports.
use crate::{Application, ApplicationError};
use agena_api::extended_sessions::*;
use base64::Engine as _;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _},
    sync::{Notify, mpsc},
};
use uuid::Uuid;

const IDLE: Duration = Duration::from_secs(600);
const CHUNK: usize = 64 * 1024;
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct OperationIo {
    streams: Mutex<HashMap<Uuid, Arc<StreamEntry>>>,
    transfers: Mutex<HashMap<Uuid, Arc<TransferEntry>>>,
    reaper_started: AtomicBool,
}

struct StreamEntry {
    state: Mutex<StreamState>,
    changed: Notify,
    producer: Mutex<Option<tokio::task::AbortHandle>>,
}
struct StreamState {
    events: VecDeque<(StreamEvent, usize)>,
    bytes: usize,
    sequence: u64,
    closed: bool,
    last_access: Instant,
}
struct TransferEntry {
    last_access: Mutex<Instant>,
    state: tokio::sync::Mutex<TransferState>,
}
struct TransferState {
    file: tokio::fs::File,
    _path: tempfile::TempPath,
    purpose: Option<UploadPurpose>,
    size: u64,
    finish: Option<Arc<UploadFinish>>,
}

#[derive(Default)]
struct UploadFinish {
    result: Mutex<Option<Result<serde_json::Value, agena_failure::Failure>>>,
    changed: Notify,
}
impl UploadFinish {
    async fn wait(&self) -> Result<serde_json::Value, ApplicationError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(result) = self
                .result
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
            {
                return result.map_err(ApplicationError::from_failure);
            }
            changed.await;
        }
    }
}

impl Drop for OperationIo {
    fn drop(&mut self) {
        for entry in self
            .streams
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            entry.stop();
        }
    }
}
impl StreamEntry {
    fn stop(&self) {
        if let Some(task) = self
            .producer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.changed.notify_waiters();
    }
}

impl OperationIo {
    fn start_reaper(self: &Arc<Self>) {
        if self.reaper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(store) = weak.upgrade() else {
                    break;
                };
                store
                    .streams
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|_, entry| {
                        let stale = entry
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .last_access
                            .elapsed()
                            > IDLE;
                        if stale {
                            entry.stop();
                        }
                        !stale
                    });
                store
                    .transfers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|_, entry| {
                        if entry
                            .last_access
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .elapsed()
                            <= IDLE
                        {
                            return true;
                        }
                        match entry.state.try_lock() {
                            Err(_) => true,
                            Ok(state) => state.finish.as_ref().is_some_and(|finish| {
                                finish
                                    .result
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .is_none()
                            }),
                        }
                    });
            }
        });
    }

    pub(crate) fn register_stream(
        self: &Arc<Self>,
        mut source: mpsc::Receiver<(String, serde_json::Value)>,
    ) -> StreamHandle {
        self.start_reaper();
        let id = Uuid::new_v4();
        let entry = Arc::new(StreamEntry {
            state: Mutex::new(StreamState {
                events: VecDeque::new(),
                bytes: 0,
                sequence: 0,
                closed: false,
                last_access: Instant::now(),
            }),
            changed: Notify::new(),
            producer: Mutex::new(None),
        });
        let target = entry.clone();
        let task = tokio::spawn(async move {
            while let Some((event, data)) = source.recv().await {
                let bytes = serde_json::to_vec(&data).map_or(MAX_STREAM_BYTES + 1, |v| v.len());
                {
                    let mut state = target.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.sequence += 1;
                    let sequence = state.sequence;
                    let (event, data, bytes) = if bytes > MAX_STREAM_BYTES {
                        (
                            "error".to_owned(),
                            serde_json::json!({"message":"The live event exceeds the delivery budget; read the resource in pages."}),
                            128,
                        )
                    } else {
                        (event, data, bytes)
                    };
                    while state.events.len() >= 256
                        || state.bytes.saturating_add(bytes) > MAX_STREAM_BYTES
                    {
                        if let Some((_, size)) = state.events.pop_front() {
                            state.bytes = state.bytes.saturating_sub(size);
                        } else {
                            break;
                        }
                    }
                    state.bytes += bytes;
                    state.events.push_back((
                        StreamEvent {
                            sequence,
                            event,
                            data,
                        },
                        bytes,
                    ));
                }
                target.changed.notify_waiters();
            }
            target
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .closed = true;
            target.changed.notify_waiters();
        });
        *entry.producer.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
        self.streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, entry);
        StreamHandle {
            stream_id: id,
            after_seq: 0,
            idle_timeout_ms: IDLE.as_millis() as u64,
        }
    }

    pub(crate) async fn read_stream(
        &self,
        request: ReadStreamParams,
    ) -> Result<StreamPage, ApplicationError> {
        let entry = self
            .streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&request.stream_id)
            .cloned()
            .ok_or_else(|| ApplicationError::not_found("The stream has closed or expired."))?;
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(request.wait_ms.unwrap_or(0).min(30_000));
        loop {
            let changed = entry.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
                state.last_access = Instant::now();
                if request.after_seq > state.sequence {
                    return Err(ApplicationError::bad_request(
                        "The stream cursor is ahead of this handle.",
                    ));
                }
                let first = state
                    .events
                    .front()
                    .map_or(state.sequence + 1, |(event, _)| event.sequence);
                let events: Vec<_> = state
                    .events
                    .iter()
                    .filter(|(event, _)| event.sequence > request.after_seq)
                    .take(request.limit.unwrap_or(64).clamp(1, 128))
                    .map(|(event, _)| event.clone())
                    .collect();
                let next_seq = events
                    .last()
                    .map_or(request.after_seq, |event| event.sequence);
                if !events.is_empty() || state.closed || tokio::time::Instant::now() >= deadline {
                    return Ok(StreamPage {
                        events,
                        next_seq,
                        lagged: request.after_seq.saturating_add(1) < first,
                        closed: state.closed && next_seq >= state.sequence,
                    });
                }
            }
            let _ = tokio::time::timeout_at(deadline, changed).await;
        }
    }

    pub(crate) fn close_stream(&self, id: Uuid) {
        if let Some(entry) = self
            .streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        {
            entry.stop();
        }
    }

    pub(crate) async fn register_transfer(
        self: &Arc<Self>,
        temp: tempfile::NamedTempFile,
        purpose: Option<UploadPurpose>,
        filename: Option<String>,
    ) -> Result<TransferHandle, ApplicationError> {
        self.start_reaper();
        let (file, path) = temp.into_parts();
        let size = file
            .metadata()
            .map_err(|e| ApplicationError::internal_error(&e))?
            .len();
        let id = Uuid::new_v4();
        self.transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                Arc::new(TransferEntry {
                    last_access: Mutex::new(Instant::now()),
                    state: tokio::sync::Mutex::new(TransferState {
                        file: tokio::fs::File::from_std(file),
                        _path: path,
                        purpose,
                        size,
                        finish: None,
                    }),
                }),
            );
        Ok(TransferHandle {
            transfer_id: id,
            filename,
            size,
            chunk_bytes: CHUNK,
            idle_timeout_ms: IDLE.as_millis() as u64,
        })
    }

    fn transfer(&self, id: Uuid) -> Result<Arc<TransferEntry>, ApplicationError> {
        let entry = self
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
            .ok_or_else(|| ApplicationError::not_found("The transfer has closed or expired."))?;
        *entry.last_access.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        Ok(entry)
    }

    pub(crate) async fn read_transfer(
        &self,
        request: ReadTransferParams,
    ) -> Result<TransferChunk, ApplicationError> {
        let entry = self.transfer(request.transfer_id)?;
        let mut state = entry.state.lock().await;
        if state.purpose.is_some() {
            return Err(ApplicationError::bad_request("This handle is an upload."));
        }
        if request.offset > state.size {
            return Err(ApplicationError::bad_request(
                "The byte offset is beyond the transfer.",
            ));
        }
        state
            .file
            .seek(std::io::SeekFrom::Start(request.offset))
            .await
            .map_err(|e| ApplicationError::internal_error(&e))?;
        let count = (state.size - request.offset)
            .min(request.max_bytes.unwrap_or(CHUNK).clamp(1, CHUNK) as u64)
            as usize;
        let mut bytes = vec![0; count];
        state
            .file
            .read_exact(&mut bytes)
            .await
            .map_err(|e| ApplicationError::internal_error(&e))?;
        let next_offset = request.offset + count as u64;
        Ok(TransferChunk {
            offset: request.offset,
            next_offset,
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            eof: next_offset == state.size,
        })
    }

    pub(crate) async fn write_transfer(
        &self,
        id: Uuid,
        offset: u64,
        data: String,
    ) -> Result<u64, ApplicationError> {
        if data.len() > CHUNK.div_ceil(3) * 4 {
            return Err(ApplicationError::bad_request(
                "Transfer chunks must be at most 65536 bytes.",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| ApplicationError::bad_request_error(&e))?;
        if bytes.len() > CHUNK {
            return Err(ApplicationError::bad_request(
                "Transfer chunk exceeds its byte budget.",
            ));
        }
        let entry = self.transfer(id)?;
        let mut state = entry.state.lock().await;
        if state.purpose.is_none() || state.finish.is_some() {
            return Err(ApplicationError::conflict("This transfer is not writable."));
        }
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| ApplicationError::bad_request("Transfer offset overflow."))?;
        if offset > state.size {
            return Err(ApplicationError::conflict(
                "Append at the current transfer offset.",
            ));
        }
        if matches!(state.purpose, Some(UploadPurpose::Attachment { .. }))
            && end > crate::service::MAX_UPLOAD_BYTES
        {
            return Err(ApplicationError::bad_request(
                "Attachments must be at most 50 MiB.",
            ));
        }
        state
            .file
            .seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|e| ApplicationError::internal_error(&e))?;
        if offset < state.size {
            if end > state.size {
                return Err(ApplicationError::conflict(
                    "The retried chunk overlaps unwritten bytes.",
                ));
            }
            let mut previous = vec![0; bytes.len()];
            state
                .file
                .read_exact(&mut previous)
                .await
                .map_err(|e| ApplicationError::internal_error(&e))?;
            if previous != bytes {
                return Err(ApplicationError::conflict(
                    "The retried chunk differs from the stored bytes.",
                ));
            }
        } else {
            state
                .file
                .write_all(&bytes)
                .await
                .map_err(|e| ApplicationError::internal_error(&e))?;
            state.size = end;
        }
        Ok(state.size)
    }

    pub(crate) async fn finish_upload(
        &self,
        app: &Application,
        id: Uuid,
    ) -> Result<serde_json::Value, ApplicationError> {
        let entry = self.transfer(id)?;
        let completion = {
            let mut state = entry.state.lock().await;
            if state.purpose.is_none() {
                return Err(ApplicationError::bad_request("This handle is a download."));
            }
            if let Some(completion) = &state.finish {
                completion.clone()
            } else {
                let completion = Arc::new(UploadFinish::default());
                state.finish = Some(completion.clone());
                let owner = entry.clone();
                let application = app.clone();
                let target = completion.clone();
                // Finishing belongs to the transfer. A disconnected caller can
                // retry this handle without repeating the committed file write.
                tokio::spawn(async move {
                    let result = Self::complete_upload(&application, &owner)
                        .await
                        .map_err(|error| *error.failure);
                    *owner.last_access.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                    *target.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    target.changed.notify_waiters();
                });
                completion
            }
        };
        completion.wait().await
    }

    async fn complete_upload(
        app: &Application,
        entry: &TransferEntry,
    ) -> Result<serde_json::Value, ApplicationError> {
        let mut state = entry.state.lock().await;
        let purpose = state
            .purpose
            .clone()
            .expect("upload purpose checked before starting");
        state
            .file
            .flush()
            .await
            .map_err(|e| ApplicationError::internal_error(&e))?;
        state
            .file
            .seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(|e| ApplicationError::internal_error(&e))?;
        match purpose {
            UploadPurpose::Import { request_id } => serde_json::to_value(
                app.import_session_archive(request_id, &mut state.file)
                    .await?,
            ),
            UploadPurpose::Attachment {
                session_id,
                filename,
                mime,
            } => {
                let mut bytes = Vec::with_capacity(state.size as usize);
                state
                    .file
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|e| ApplicationError::internal_error(&e))?;
                serde_json::to_value(
                    app.service()
                        .upload_session_file_bytes(session_id, filename, mime, bytes)
                        .await?,
                )
            }
        }
        .map_err(|e| ApplicationError::internal_error(&e))
    }

    pub(crate) fn close_transfer(&self, id: Uuid) {
        self.transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }
}
