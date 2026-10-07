//! Bounded disk capture before process output is shortened or evicted.
//!
//! One dedicated I/O thread serves all archives. Async pipe readers apply
//! bounded backpressure; PTY drivers reserve a queue slot without waiting so
//! input, signals and process cleanup keep advancing when storage is slow.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use agena_domain::{ProcessOutputArchive, ProcessOutputSegment};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::{mpsc, oneshot},
};
use tokio_util::sync::PollSender;

pub(crate) const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024;
// A stable startup prefix and three recent segments; never concatenate the
// missing middle into a file that pretends to contain contiguous output.
const SEGMENT_BYTES: u64 = MAX_ARCHIVE_BYTES / 4;
const MAX_SEGMENTS: usize = 4;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
const QUEUE_CAPACITY: usize = 64;
const READ_BYTES: usize = 8 * 1024;

pub(crate) struct ArchivePermit(mpsc::OwnedPermit<Message>);

#[derive(Debug, Clone)]
pub(crate) struct OutputArchive(Arc<State>);

#[derive(Debug)]
struct State {
    path: PathBuf,
    workspace_dir: PathBuf,
    file: Mutex<Option<File>>,
    segments: Mutex<Vec<ProcessOutputSegment>>,
    segment_index: AtomicU64,
    recording: Mutex<()>,
    disabled: AtomicBool,
    finished: AtomicBool,
    truncated: AtomicBool,
    total: AtomicU64,
    retained: AtomicU64,
    pending: AtomicUsize,
    stalled_readers: AtomicUsize,
    error: Mutex<Option<String>>,
}

enum Message {
    Write(Arc<State>, u64, Vec<u8>),
    Finish(Arc<State>),
    Barrier(oneshot::Sender<()>),
}

static WRITER: LazyLock<Option<mpsc::Sender<Message>>> = LazyLock::new(|| {
    let (sender, mut receiver) = mpsc::channel(QUEUE_CAPACITY);
    let worker = std::thread::Builder::new()
        .name("agena-output-archive".into())
        .spawn(move || {
            let mut usage = HashMap::new();
            while let Some(message) = receiver.blocking_recv() {
                match message {
                    Message::Write(state, offset, bytes) => {
                        if let Err(error) = write_chunk(&state, offset, &bytes, &mut usage) {
                            state.fail(format!("output archive write failed: {error}"));
                        }
                        state.pending.fetch_sub(1, Ordering::AcqRel);
                        state.close_if_finished();
                    }
                    Message::Finish(state) => state.close_if_finished(),
                    Message::Barrier(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
    match worker {
        Ok(_) => Some(sender),
        Err(error) => {
            tracing::error!(%error, "could not start the output archive writer");
            None
        }
    }
});

impl State {
    fn fail(&self, error: String) {
        self.disabled.store(true, Ordering::Release);
        self.truncated.store(true, Ordering::Release);
        let mut current = self
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        current.get_or_insert(error);
    }

    fn close_if_finished(&self) {
        if self.finished.load(Ordering::Acquire)
            && self.pending.load(Ordering::Acquire) == 0
            && let Ok(mut file) = self.file.try_lock()
        {
            file.take();
        }
    }
}

impl OutputArchive {
    pub(crate) fn new(workspace: &Path, session_id: Option<i64>) -> Self {
        let workspace_dir = crate::tool_output_spill_dir(workspace);
        let path = workspace_dir
            .join(session_id.unwrap_or(0).to_string())
            .join(format!("shell_{}.txt", uuid::Uuid::new_v4().simple()));
        let archive = Self(Arc::new(State {
            path,
            workspace_dir,
            file: Mutex::new(None),
            segments: Mutex::new(Vec::new()),
            segment_index: AtomicU64::new(0),
            recording: Mutex::new(()),
            disabled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            truncated: AtomicBool::new(false),
            total: AtomicU64::new(0),
            retained: AtomicU64::new(0),
            pending: AtomicUsize::new(0),
            stalled_readers: AtomicUsize::new(0),
            error: Mutex::new(None),
        }));
        if WRITER.is_none() {
            archive
                .0
                .fail("output archive writer is unavailable".into());
        }
        archive
    }

    fn can_write(&self) -> bool {
        !self.0.disabled.load(Ordering::Acquire)
    }

    pub(crate) fn capture_busy(&self) -> bool {
        self.0.pending.load(Ordering::Acquire) != 0
            || self.0.stalled_readers.load(Ordering::Acquire) != 0
    }

    /// Err means temporary queue pressure: leave the PTY unread and process
    /// input/control requests. None means capture has hit a storage limit.
    pub(crate) fn try_reserve(&self) -> Result<Option<ArchivePermit>, ()> {
        if !self.can_write() {
            return Ok(None);
        }
        match WRITER
            .as_ref()
            .expect("enabled writer")
            .clone()
            .try_reserve_owned()
        {
            Ok(permit) => Ok(Some(ArchivePermit(permit))),
            Err(mpsc::error::TrySendError::Full(_)) => Err(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.0.fail("output archive writer stopped".into());
                Ok(None)
            }
        }
    }

    fn message(&self, text: &str) -> Option<Message> {
        if text.is_empty() {
            return None;
        }
        let previous = self.0.total.fetch_add(text.len() as u64, Ordering::AcqRel);
        if self.0.disabled.load(Ordering::Acquire) {
            self.0.truncated.store(true, Ordering::Release);
            return None;
        }
        self.0.pending.fetch_add(1, Ordering::AcqRel);
        Some(Message::Write(
            Arc::clone(&self.0),
            previous,
            text.as_bytes().to_vec(),
        ))
    }

    pub(crate) fn record(&self, permit: Option<ArchivePermit>, text: &str) {
        // Assign offsets and enqueue together, including concurrent stdout,
        // stderr and PTY final fragments. The lock never spans a wait or I/O.
        let _recording = self
            .0
            .recording
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(message) = self.message(text) else {
            return;
        };
        if let Some(permit) = permit {
            permit.0.send(message);
        } else {
            self.0.pending.fetch_sub(1, Ordering::AcqRel);
            self.mark_partial("output archive queue was unavailable; some output was not saved");
        }
    }

    /// Diagnostic/final fragments must never block a PTY control operation.
    pub(crate) fn record_fragment(&self, text: &str) {
        match self.try_reserve() {
            Ok(permit) => self.record(permit, text),
            Err(()) => {
                self.0.total.fetch_add(text.len() as u64, Ordering::AcqRel);
                self.mark_partial("output archive queue was full at process cleanup");
            }
        }
    }

    pub(crate) fn mark_partial(&self, reason: &str) {
        // Already queued prefix data is still useful. A reader/cleanup failure
        // marks loss without preventing the writer from saving that prefix.
        self.0.truncated.store(true, Ordering::Release);
        self.0
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert_with(|| reason.into());
    }

    pub(crate) fn finish(&self) {
        self.0.finished.store(true, Ordering::Release);
        if let Some(sender) = WRITER.as_ref() {
            if let Err(mpsc::error::TrySendError::Full(message)) =
                sender.try_send(Message::Finish(Arc::clone(&self.0)))
            {
                // Schedule only the bounded queue admission; the I/O thread
                // owns File close as well as writes/rotation.
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let sender = sender.clone();
                    handle.spawn(async move {
                        let _ = sender.send(message).await;
                    });
                }
            }
        }
    }

    pub(crate) async fn finish_async(&self) {
        self.finish();
        let Some(sender) = WRITER.as_ref() else {
            return;
        };
        let (done, receipt) = oneshot::channel();
        let flush = async {
            sender.send(Message::Barrier(done)).await.map_err(|_| ())?;
            receipt.await.map_err(|_| ())
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), flush).await,
            Ok(Ok(()))
        ) {
            self.0.truncated.store(true, Ordering::Release);
            let mut error = self
                .0
                .error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            error.get_or_insert(
                "output archive flush was not confirmed; queued writes may still finish".into(),
            );
        }
    }

    /// Small foreground results already fit completely in the preview. Their
    /// temporary capture need not consume storage or add a model-facing handle.
    pub(crate) async fn discard_if_fully_visible(
        &self,
        visible: &str,
        preview_bytes: usize,
    ) -> bool {
        let Some(info) = self.snapshot() else {
            return true;
        };
        if !info.complete
            || info.total_bytes > preview_bytes as u64
            || crate::process_output::escaped_bytes(visible) > preview_bytes
            || visible.lines().take(201).count() > 200
        {
            return false;
        }
        matches!(
            tokio::time::timeout(Duration::from_secs(2), tokio::fs::remove_file(&self.0.path))
                .await,
            Ok(Ok(()))
        )
    }

    pub(crate) fn snapshot(&self) -> Option<ProcessOutputArchive> {
        let error = self
            .0
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let total = self.0.total.load(Ordering::Acquire);
        if total == 0 && error.is_none() {
            return None;
        }
        let segments = self
            .0
            .segments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let retained_bytes = segments
            .iter()
            .map(|segment| segment.end_byte - segment.start_byte)
            .sum();
        let pending = self.0.pending.load(Ordering::Acquire) != 0;
        let truncated = self.0.truncated.load(Ordering::Acquire);
        Some(ProcessOutputArchive {
            path: segments.first().map(|segment| segment.path.clone()),
            segments,
            retained_bytes,
            total_bytes: total,
            pending,
            complete: self.0.finished.load(Ordering::Acquire) && !pending && !truncated,
            truncated,
            limit_bytes: MAX_ARCHIVE_BYTES,
            error,
        })
    }
}

/// Archives pipe text before line framing/filtering or in-memory truncation.
/// At most one 8 KiB raw segment is staged per reader. No queue reservation is
/// held while waiting for a quiet pipe, which would starve active processes.
pub(crate) struct ArchivedReader<R> {
    reader: R,
    archive: Option<OutputArchive>,
    sender: Option<PollSender<Message>>,
    staged: Vec<u8>,
    position: usize,
    text: Option<String>,
    utf8: Vec<u8>,
    eof: bool,
    stalled: bool,
}

impl<R> ArchivedReader<R> {
    pub(crate) fn new(reader: R, archive: Option<OutputArchive>) -> Self {
        let sender = archive
            .as_ref()
            .and_then(|_| WRITER.as_ref().cloned())
            .map(PollSender::new);
        Self {
            reader,
            archive,
            sender,
            staged: Vec::new(),
            position: 0,
            text: None,
            utf8: Vec::new(),
            eof: false,
            stalled: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ArchivedReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if this.staged.is_empty() && !this.eof {
            let mut bytes = [0_u8; READ_BYTES];
            let mut read = ReadBuf::new(&mut bytes);
            match Pin::new(&mut this.reader).poll_read(cx, &mut read) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    if let Some(archive) = &this.archive {
                        archive.mark_partial("process output pipe read failed");
                    }
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(())) => {}
            }
            this.eof = read.filled().is_empty();
            if this.archive.is_some() {
                this.text = Some(crate::tool::shell::decode_output(
                    &mut this.utf8,
                    read.filled(),
                    this.eof,
                ));
            }
            this.staged.extend_from_slice(read.filled());
        }
        if let (Some(archive), Some(text)) = (&this.archive, this.text.take()) {
            if !text.is_empty() {
                if archive.can_write() {
                    let sender = this.sender.as_mut().expect("enabled writer");
                    match sender.poll_reserve(cx) {
                        Poll::Pending => {
                            if !this.stalled {
                                archive.0.stalled_readers.fetch_add(1, Ordering::AcqRel);
                                this.stalled = true;
                            }
                            this.text = Some(text);
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(_)) => {
                            archive.0.fail("output archive writer stopped".into());
                            let _ = archive.message(&text);
                        }
                        Poll::Ready(Ok(())) => {
                            let _recording = archive
                                .0
                                .recording
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if let Some(message) = archive.message(&text) {
                                if sender.send_item(message).is_err() {
                                    archive.0.pending.fetch_sub(1, Ordering::AcqRel);
                                    archive.0.fail("output archive writer stopped".into());
                                }
                            } else {
                                sender.abort_send();
                            }
                        }
                    }
                } else {
                    // Still count text beyond a storage quota without sending
                    // it to the writer or slowing the process's pipe drain.
                    let _ = archive.message(&text);
                }
            }
        }
        if this.stalled {
            this.archive
                .as_ref()
                .expect("stalled archive reader")
                .0
                .stalled_readers
                .fetch_sub(1, Ordering::AcqRel);
            this.stalled = false;
        }
        let length = output.remaining().min(this.staged.len() - this.position);
        output.put_slice(&this.staged[this.position..this.position + length]);
        this.position += length;
        if this.position == this.staged.len() {
            this.staged.clear();
            this.position = 0;
        }
        Poll::Ready(Ok(()))
    }
}

impl<R> Drop for ArchivedReader<R> {
    fn drop(&mut self) {
        if self.stalled {
            self.archive
                .as_ref()
                .expect("stalled archive reader")
                .0
                .stalled_readers
                .fetch_sub(1, Ordering::AcqRel);
        }

        if (!self.eof || self.text.as_ref().is_some_and(|text| !text.is_empty()))
            && let Some(archive) = &self.archive
        {
            archive.mark_partial("output reader stopped before the pipe was fully drained");
        }
    }
}

fn write_chunk(
    state: &State,
    offset: u64,
    bytes: &[u8],
    usage: &mut HashMap<PathBuf, u64>,
) -> io::Result<()> {
    if state.disabled.load(Ordering::Acquire) {
        return Ok(());
    }
    if !usage.contains_key(&state.workspace_dir) {
        usage.insert(
            state.workspace_dir.clone(),
            workspace_usage(&state.workspace_dir)?,
        );
    }
    let mut slot = state
        .file
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
    let mut position = 0;
    while position < bytes.len() {
        let last = state
            .segments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned();
        let remaining = last.as_ref().map_or(SEGMENT_BYTES, |segment| {
            SEGMENT_BYTES.saturating_sub(segment.end_byte - segment.start_byte)
        });
        let mut length = remaining.min((bytes.len() - position) as u64) as usize;
        while length > 0 && !text.is_char_boundary(position + length) {
            length -= 1;
        }
        // A multibyte codepoint may not fit the last few bytes in a segment.
        if length == 0
            || last
                .as_ref()
                .is_some_and(|segment| segment.end_byte != offset + position as u64)
        {
            slot.take();
        }
        if slot.is_none() {
            let mut segments = state
                .segments
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if segments.len() >= MAX_SEGMENTS {
                let removed = segments.remove(1); // preserve the startup prefix
                std::fs::remove_file(&removed.path)?;
                let removed_bytes = removed.end_byte - removed.start_byte;
                let used = usage
                    .get_mut(&state.workspace_dir)
                    .expect("workspace usage");
                *used = used.saturating_sub(removed_bytes);
                state.retained.fetch_sub(removed_bytes, Ordering::AcqRel);
                state.truncated.store(true, Ordering::Release);
                *state
                    .segments
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = segments;
            }
            let index = state.segment_index.fetch_add(1, Ordering::AcqRel);
            let path = if index == 0 {
                state.path.clone()
            } else {
                state.path.with_file_name(format!(
                    "{}_{}.txt",
                    state
                        .path
                        .file_stem()
                        .expect("archive stem")
                        .to_string_lossy(),
                    index
                ))
            };
            let mut directory = std::fs::DirBuilder::new();
            directory.recursive(true);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
                directory.mode(0o700);
                options.mode(0o600);
            }
            directory.create(state.path.parent().expect("archive directory"))?;
            *slot = Some(options.open(&path)?);
            state
                .segments
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(ProcessOutputSegment {
                    path: path.to_string_lossy().into_owned(),
                    start_byte: offset + position as u64,
                    end_byte: offset + position as u64,
                });
            length = (bytes.len() - position).min(SEGMENT_BYTES as usize);
            while !text.is_char_boundary(position + length) {
                length -= 1;
            }
        }
        // Recheck only near quota. Rotation releases this process's old tail
        // before admission, so a full archive can keep capturing recent data.
        if usage[&state.workspace_dir].saturating_add(length as u64) > MAX_WORKSPACE_BYTES {
            usage.insert(
                state.workspace_dir.clone(),
                workspace_usage(&state.workspace_dir)?,
            );
        }
        let used = usage
            .get_mut(&state.workspace_dir)
            .expect("workspace usage");
        length = length.min(MAX_WORKSPACE_BYTES.saturating_sub(*used) as usize);
        while length > 0 && !text.is_char_boundary(position + length) {
            length -= 1;
        }
        if length == 0 {
            state.fail("workspace shell output archives reached the 256 MiB storage limit".into());
            return Ok(());
        }
        let end = position + length;
        while position < end {
            match slot
                .as_mut()
                .expect("open archive")
                .write(&bytes[position..end])
            {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "archive write returned zero",
                    ));
                }
                Ok(written) => {
                    position += written;
                    *used += written as u64;
                    state.retained.fetch_add(written as u64, Ordering::Release);
                    state
                        .segments
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .last_mut()
                        .expect("active segment")
                        .end_byte += written as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn workspace_usage(base: &Path) -> io::Result<u64> {
    let sessions = match std::fs::read_dir(base) {
        Ok(sessions) => sessions,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut total = 0u64;
    let mut visited = 0usize;
    for session in sessions {
        let session = session?;
        if !session.file_type()?.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(session.path())? {
            let entry = entry?;
            visited += 1;
            if visited > 100_000 {
                return Err(io::Error::other(
                    "shell archive quota scan exceeded its entry limit",
                ));
            }
            if entry.file_name().to_string_lossy().starts_with("shell_")
                && entry.file_type()?.is_file()
            {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
    }
    Ok(total)
}

pub(crate) fn archive_hint(archive: &ProcessOutputArchive) -> String {
    let location = archive.path.as_deref().map_or_else(
        || "Output archive is not available yet.".into(),
        |path| format!("Captured output: {path}\nSearch with fs.grep (path, pattern, max_results); read selected lines with fs.read (file_path, mode=\"text\", offset, limit)."),
    );
    let mut location = location;
    for segment in archive.segments.iter().skip(1) {
        location.push_str(&format!(
            "\nRetained range [{}..{}): {} (file-local offsets start at 0).",
            segment.start_byte, segment.end_byte, segment.path
        ));
    }
    let status = if archive.truncated {
        "Archive retains a startup prefix and recent segments; gaps or capture errors mean older output is unavailable. Check segments for original byte ranges."
    } else if archive.pending {
        "Archive writes are pending; the file may still grow."
    } else if !archive.complete {
        "Process output archive may still grow."
    } else {
        "Archive contains all captured process output."
    };
    format!(
        "{location}\n{status} Retained {} of {} UTF-8 bytes (limit {} bytes).{}",
        archive.retained_bytes,
        archive.total_bytes,
        archive.limit_bytes,
        archive
            .error
            .as_ref()
            .map(|error| format!(" {error}"))
            .unwrap_or_default()
    )
}
