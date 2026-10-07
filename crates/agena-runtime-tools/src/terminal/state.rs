use super::*;
use crate::ProcessReadOptions;
use crate::process_output::{OutputCursor, OutputSelection, select_events, text_budget};
use agena_domain::ProcessStream;
use chrono::Utc;
use std::{
    collections::VecDeque,
    sync::{
        Condvar,
        atomic::{AtomicU8, Ordering},
    },
};

pub(super) enum Control {
    Write(Vec<u8>),
    Resize(u16, u16),
    Interrupt,
}
pub(super) struct Request {
    pub control: Control,
    pub deadline: Instant,
    pub cancel: CancellationToken,
    pub reply: mpsc::SyncSender<Result<(), String>>,
}

pub(super) struct State {
    pub id: String,
    pub owner: TerminalOwner,
    pub command: String,
    pub description: String,
    pub workdir: PathBuf,
    pub started_at_ms: i64,
    pub interaction: Mutex<()>,
    pub stopping: AtomicU8,
    pub archive: crate::process_output_archive::OutputArchive,
    inner: Mutex<Inner>,
    changed: Condvar,
    controls: mpsc::SyncSender<Request>,
    listener: Option<Arc<dyn MonitorListener>>,
}

struct Inner {
    spawned: bool,
    status: ProcessStatus,
    exit_code: Option<i32>,
    reason: Option<String>,
    ended_at_ms: Option<i64>,
    events: VecDeque<ProcessEvent>,
    buffered_bytes: usize,
    seq: u64,
    cursor: OutputCursor,
    dropped_events: u64,
    dropped_bytes: u64,
    utf8: Vec<u8>,
    parser: vt100::Parser<super::protocol::Protocol>,
    osc_guard: super::osc_guard::OscGuard,
}

impl State {
    pub fn new(
        id: String,
        params: &TerminalStartParams,
        controls: mpsc::SyncSender<Request>,
        listener: Option<Arc<dyn MonitorListener>>,
    ) -> Self {
        Self {
            archive: crate::process_output_archive::OutputArchive::new(
                &params.owner.workspace,
                params.owner.session_id,
            ),
            id,
            owner: params.owner.clone(),
            command: params.display_command.clone(),
            description: params.description.clone(),
            workdir: params.workdir.clone(),
            started_at_ms: Utc::now().timestamp_millis(),
            interaction: Mutex::new(()),
            stopping: AtomicU8::new(0),
            controls,
            changed: Condvar::new(),
            listener,
            inner: Mutex::new(Inner {
                spawned: false,
                status: ProcessStatus::Running,
                exit_code: None,
                reason: None,
                ended_at_ms: None,
                events: VecDeque::new(),
                buffered_bytes: 0,
                seq: 0,
                cursor: OutputCursor::default(),
                dropped_events: 0,
                dropped_bytes: 0,
                utf8: Vec::new(),
                parser: vt100::Parser::new_with_callbacks(
                    params.rows,
                    params.cols,
                    0,
                    super::protocol::Protocol::default(),
                ),
                osc_guard: super::osc_guard::OscGuard::default(),
            }),
        }
    }

    pub fn is_running(&self) -> bool {
        lock(&self.inner).status == ProcessStatus::Running
    }
    pub fn mark_spawned(&self) {
        lock(&self.inner).spawned = true;
        self.changed.notify_all();
        self.notify_started();
    }

    /// Startup confirmation is independent of a call's output-yield budget.
    pub fn confirm_started(
        &self,
        cancel: &CancellationToken,
        cleanup_on_cancel: bool,
    ) -> Result<(), MonitorError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut inner = lock(&self.inner);
        loop {
            if cancel.is_cancelled() || Instant::now() >= deadline {
                if cleanup_on_cancel {
                    self.stopping.store(2, Ordering::Release);
                }
                return Err(invalid(
                    "terminal launch cancelled or startup confirmation timed out",
                ));
            }
            if inner.spawned {
                return Ok(());
            }
            if inner.status != ProcessStatus::Running {
                return Err(invalid(format!(
                    "terminal {} could not start: {}",
                    self.id,
                    inner
                        .events
                        .back()
                        .map(|event| event.line.as_str())
                        .unwrap_or("terminal startup failed")
                )));
            }
            inner = self
                .changed
                .wait_timeout(inner, Duration::from_millis(50))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
    pub fn summary(&self) -> ProcessSummary {
        self.summary_locked(&lock(&self.inner))
    }

    fn summary_locked(&self, inner: &Inner) -> ProcessSummary {
        ProcessSummary {
            websocket: false,
            process_id: self.id.clone(),
            output_archive: self.archive.snapshot(),
            tty: true,
            command: self.command.clone(),
            description: self.description.clone(),
            status: inner.status,
            background: true,
            monitored: false,
            ready: false,
            started_at_ms: self.started_at_ms,
            ended_at_ms: inner.ended_at_ms,
            buffered_lines: inner.events.len() as u32,
            last_seq: inner.seq,
            dropped_lines: inner.dropped_events,
            exit_code: inner.exit_code,
            completion_reason: inner.reason.clone(),
            owner_session_id: self.owner.session_id,
        }
    }

    pub fn append(&self, bytes: &[u8]) {
        self.append_captured(bytes, None);
    }

    pub fn append_captured(
        &self,
        bytes: &[u8],
        permit: Option<crate::process_output_archive::ArchivePermit>,
    ) {
        let event = {
            let mut inner = lock(&self.inner);
            let screen_bytes = inner.osc_guard.filter(bytes);
            inner.parser.process(&screen_bytes);
            inner.utf8.extend_from_slice(bytes);
            // Decode invalid bytes lossily, but retain any incomplete trailing
            // code point even when an invalid byte preceded it in this chunk.
            let mut offset = 0;
            let mut text = String::new();
            while offset < inner.utf8.len() {
                match std::str::from_utf8(&inner.utf8[offset..]) {
                    Ok(valid) => {
                        text.push_str(valid);
                        offset = inner.utf8.len();
                    }
                    Err(error) => {
                        let valid_end = offset + error.valid_up_to();
                        text.push_str(
                            std::str::from_utf8(&inner.utf8[offset..valid_end])
                                .expect("validated UTF-8 prefix"),
                        );
                        offset = valid_end;
                        if let Some(length) = error.error_len() {
                            text.push('\u{fffd}');
                            offset += length;
                        } else {
                            break;
                        }
                    }
                }
            }
            inner.utf8.drain(..offset);
            if text.is_empty() {
                return;
            }
            Self::push(&mut inner, text)
        };
        if let Some(permit) = permit {
            self.archive.record(Some(permit), &event.line);
        } else {
            self.archive.record_fragment(&event.line);
        }
        self.changed.notify_all();
        if let Some(listener) = &self.listener {
            let summary = self.summary();
            self.observe(|| listener.on_event(&event, &summary));
        }
    }

    fn push(inner: &mut Inner, line: String) -> ProcessEvent {
        inner.seq += 1;
        let event = ProcessEvent {
            seq: inner.seq,
            stream: ProcessStream::Stdout,
            ts_ms: Utc::now().timestamp_millis(),
            line,
            chunk: true,
            notification: None,
            notification_seq: None,
        };
        inner.buffered_bytes += event.line.len();
        inner.events.push_back(event.clone());
        while inner.buffered_bytes > MAX_BUFFER_BYTES || inner.events.len() > MAX_BUFFER_EVENTS {
            if let Some(evicted) = inner.events.pop_front() {
                inner.buffered_bytes -= evicted.line.len();
                inner.dropped_bytes += evicted.line.len() as u64;
                inner.dropped_events += 1;
            }
        }
        event
    }

    pub fn take_protocol_replies(&self) -> Result<Vec<u8>, &'static str> {
        lock(&self.inner).parser.callbacks_mut().take()
    }

    pub fn resize_screen(&self, rows: u16, cols: u16) {
        lock(&self.inner).parser.screen_mut().set_size(rows, cols);
        self.changed.notify_all();
    }

    pub fn finish(&self, status: ProcessStatus, exit_code: Option<i32>, reason: &str) {
        {
            let mut inner = lock(&self.inner);
            if inner.status != ProcessStatus::Running {
                return;
            }
            if !inner.utf8.is_empty() {
                let final_text = String::from_utf8_lossy(&inner.utf8).into_owned();
                self.archive.record_fragment(&final_text);
                inner.utf8.clear();
                Self::push(&mut inner, final_text);
            }
            inner.status = status;
            inner.exit_code = exit_code;
            inner.reason = Some(reason.to_owned());
            inner.ended_at_ms = Some(Utc::now().timestamp_millis());
        }
        self.archive.finish();
        self.changed.notify_all();
        if let Some(listener) = &self.listener {
            let summary = self.summary();
            self.observe(|| listener.on_finished(&summary));
        }
    }

    pub fn notify_started(&self) {
        if let Some(listener) = &self.listener {
            let summary = self.summary();
            self.observe(|| listener.on_started(&summary));
        }
    }

    fn observe(&self, callback: impl FnOnce()) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)).is_err() {
            tracing::error!(process_id = %self.id, "terminal lifecycle observer panicked");
        }
    }

    pub fn read(
        &self,
        since: Option<u64>,
        wait_ms: u64,
        limit: Option<u32>,
    ) -> Result<TerminalRead, MonitorError> {
        self.read_cancellable(since, wait_ms, limit, &CancellationToken::new())
    }

    pub fn read_cancellable(
        &self,
        since: Option<u64>,
        wait_ms: u64,
        limit: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        let _interaction = call::interaction(&self.interaction, cancel)?;
        self.read_locked_cancellable(since, wait_ms, limit, cancel)
    }

    pub fn read_options_cancellable(
        &self,
        options: ProcessReadOptions,
        include_screen: bool,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        let _interaction = call::interaction(&self.interaction, cancel)?;
        self.read_options_locked(options, include_screen, cancel)
    }

    pub fn validate_options_cursor(
        &self,
        options: &ProcessReadOptions,
    ) -> Result<(), MonitorError> {
        crate::process_output::validate_cursor(options.since_seq, options.event_offset)?;
        self.validate_cursor(options.since_seq)?;
        if options.event_offset != 0 {
            let inner = lock(&self.inner);
            let seq = options.since_seq.expect("validated cursor");
            let event = inner
                .events
                .iter()
                .find(|event| event.seq == seq.saturating_add(1))
                .ok_or_else(|| {
                    invalid("partially read event was evicted; inspect output_archive")
                })?;
            let offset = options.event_offset as usize;
            if offset > event.line.len() || !event.line.is_char_boundary(offset) {
                return Err(invalid("event_offset is outside the event or splits UTF-8"));
            }
        }
        Ok(())
    }

    pub fn validate_cursor(&self, since: Option<u64>) -> Result<(), MonitorError> {
        if since.is_some_and(|seq| seq > lock(&self.inner).seq) {
            Err(invalid("output cursor is ahead of this terminal"))
        } else {
            Ok(())
        }
    }

    pub fn read_locked_cancellable(
        &self,
        since: Option<u64>,
        wait_ms: u64,
        limit: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        self.read_selected_locked(
            ProcessReadOptions {
                since_seq: since,
                wait_ms,
                limit,
                ..Default::default()
            },
            true,
            cancel,
            OutputSelection::WholeEvents,
        )
    }

    pub fn read_options_locked(
        &self,
        options: ProcessReadOptions,
        include_screen: bool,
        cancel: &CancellationToken,
    ) -> Result<TerminalRead, MonitorError> {
        self.read_selected_locked(options, include_screen, cancel, OutputSelection::Resumable)
    }

    pub fn read_selected_locked(
        &self,
        options: ProcessReadOptions,
        include_screen: bool,
        cancel: &CancellationToken,
        selection: OutputSelection,
    ) -> Result<TerminalRead, MonitorError> {
        let ProcessReadOptions {
            since_seq: since,
            event_offset,
            wait_ms,
            limit,
            max_output_bytes,
        } = options;
        crate::process_output::validate_cursor(since, event_offset)?;
        call::check(cancel)?;
        validate_wait(wait_ms)?;
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        let mut inner = lock(&self.inner);
        let cursor = since
            .map(|seq| OutputCursor {
                seq,
                offset: event_offset,
            })
            .unwrap_or(inner.cursor);
        let since_seq = cursor.seq;
        if since_seq > inner.seq {
            return Err(invalid("output cursor is ahead of this terminal"));
        }
        while inner.seq <= since_seq && inner.status == ProcessStatus::Running && wait_ms > 0 {
            call::check(cancel)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            inner = self
                .changed
                .wait_timeout(inner, remaining.min(Duration::from_millis(50)))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        // Coalesce a short burst (for example echo + REPL response), bounded
        // by the original deadline. This is not an idle-completion heuristic.
        if inner.status == ProcessStatus::Running && inner.seq > since_seq {
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20));
            if !remaining.is_zero() {
                inner = self
                    .changed
                    .wait_timeout(inner, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
        }
        call::check(cancel)?;
        let slice = select_events(
            inner.events.iter(),
            cursor,
            inner.seq,
            limit.unwrap_or(200).clamp(1, 2000) as usize,
            text_budget(max_output_bytes, include_screen),
            false,
            since.is_some(),
            selection,
        )?;
        if since.is_none() {
            inner.cursor = slice.cursor;
        }
        let screen = inner.parser.screen();
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let mut text = if include_screen {
            screen.contents()
        } else {
            String::new()
        };
        let screen_end =
            crate::process_output::prefix_end(&text, text_budget(max_output_bytes, true));
        let truncated = screen_end < text.len();
        text.truncate(screen_end);
        Ok(TerminalRead {
            summary: self.summary_locked(&inner),
            events: slice.events,
            output: slice.output,
            last_seq: slice.cursor.seq,
            next_event_offset: slice.cursor.offset,
            has_more: slice.has_more,
            dropped_bytes: inner.dropped_bytes,
            screen: TerminalScreen {
                rows,
                cols,
                cursor_row,
                cursor_col,
                cursor_visible: !screen.hide_cursor(),
                alternate_screen: screen.alternate_screen(),
                bracketed_paste: screen.bracketed_paste(),
                application_cursor: screen.application_cursor(),
                text,
                truncated,
            },
        })
    }

    pub fn control_cancellable(
        &self,
        control: Control,
        cancel: &CancellationToken,
    ) -> Result<(), MonitorError> {
        call::check(cancel)?;
        if !self.is_running() || self.stopping.load(Ordering::Acquire) != 0 {
            return Err(invalid(
                "terminal has exited or is stopping; start a new terminal",
            ));
        }
        let (reply, result) = mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_secs(2);
        self.controls
            .try_send(Request {
                control,
                deadline,
                cancel: cancel.clone(),
                reply,
            })
            .map_err(|_| invalid("terminal control channel is unavailable"))?;
        match result.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(invalid(error)),
            Err(_) => {
                self.request_stop(true);
                Err(invalid(
                    "terminal input acknowledgement was lost; termination requested to prevent ambiguous retries",
                ))
            }
        }
    }

    pub fn request_stop(&self, force: bool) {
        self.stopping
            .fetch_max(if force { 2 } else { 1 }, Ordering::AcqRel);
        self.changed.notify_all();
    }

    pub fn stop(&self, force: bool) -> Result<(), MonitorError> {
        self.request_stop(force);
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut inner = lock(&self.inner);
        while inner.status == ProcessStatus::Running {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(invalid(
                    "terminal cleanup has not completed; termination remains requested",
                ));
            }
            inner = self
                .changed
                .wait_timeout(inner, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        if inner.reason.as_deref() == Some("cleanup_failed") {
            return Err(invalid(
                "terminal process cleanup failed; inspect shell.read",
            ));
        }
        Ok(())
    }
}
