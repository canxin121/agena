//! One mutable watch per process. Capture is independent of notification policy.
use super::*;
use std::hash::{Hash, Hasher};

#[derive(Debug)]
pub(super) struct WatchState {
    pub policy: Option<ShellWatchPolicy>,
    pub revision: u64,
    pub since_seq: u64,
    pub ready: bool,
    pub outcome: u8,
    pub(super) configured_at: Instant,
    ready_pattern: Option<Regex>,
    include: Option<Regex>,
    success: Option<Regex>,
    failure: Option<Regex>,
    notified: bool,
    last_notification: Option<Instant>,
    last_fingerprint: Option<u64>,
    pending: Option<(ProcessEvent, u64)>,
}

impl WatchState {
    pub fn compile(policy: Option<ShellWatchPolicy>, since_seq: u64) -> Result<Self, MonitorError> {
        let pattern = |value: Option<&String>| -> Result<Option<Regex>, MonitorError> {
            value
                .map(|value| {
                    if value.is_empty() || value.chars().count() > 16384 {
                        return Err(MonitorError::Invalid(
                            "watch patterns must have 1–16384 characters".into(),
                        ));
                    }
                    let expression = match policy.as_ref().map(|policy| policy.pattern_kind) {
                        Some(ShellMonitorPatternKind::Literal) => regex::escape(value),
                        _ => value.clone(),
                    };
                    Regex::new(&expression).map_err(MonitorError::from)
                })
                .transpose()
        };
        if policy.as_ref().is_some_and(|policy| {
            policy
                .quiet_period_ms
                .is_some_and(|ms| ms == 0 || ms > 3_600_000)
                || policy
                    .notification_interval_ms
                    .is_some_and(|ms| !(1_000..=3_600_000).contains(&ms))
        }) {
            return Err(MonitorError::Invalid(
                "invalid quiet period or notification interval".into(),
            ));
        }
        Ok(Self {
            ready_pattern: pattern(
                policy
                    .as_ref()
                    .and_then(|policy| policy.ready_pattern.as_ref()),
            )?,
            include: pattern(
                policy
                    .as_ref()
                    .and_then(|policy| policy.include_pattern.as_ref()),
            )?,
            success: pattern(
                policy
                    .as_ref()
                    .and_then(|policy| policy.success_pattern.as_ref()),
            )?,
            failure: pattern(
                policy
                    .as_ref()
                    .and_then(|policy| policy.failure_pattern.as_ref()),
            )?,
            policy,
            revision: 0,
            since_seq,
            ready: false,
            outcome: 0,
            configured_at: Instant::now(),
            notified: false,
            last_notification: None,
            last_fingerprint: None,
            pending: None,
        })
    }

    fn observe(
        &mut self,
        event: &ProcessEvent,
        line: &str,
        complete: bool,
    ) -> (Option<&'static str>, bool) {
        let Some(policy) = &self.policy else {
            return (None, false);
        };
        if event.seq <= self.since_seq {
            return (None, false);
        }
        let previous = self.outcome;
        if complete
            && self
                .failure
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(line))
        {
            self.outcome = 2;
        } else if complete
            && self
                .success
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(line))
        {
            self.outcome = self.outcome.max(1);
        }
        // A matching terminal condition produces the final completion notice.
        // Never announce a failed or deliberately stopped command as ready.
        if self.outcome != 0 {
            return (None, self.outcome != previous);
        }
        if !self.ready
            && self
                .ready_pattern
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(line))
        {
            self.ready = true;
            return (Some("ready"), false);
        }
        if !complete
            || !self
                .include
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(line))
        {
            return (None, false);
        }
        if policy.notifications == ShellWatchNotifications::Once && self.notified {
            return (None, false);
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        event.stream.to_string().hash(&mut hash);
        line.hash(&mut hash);
        let fingerprint = hash.finish();
        if self.last_fingerprint == Some(fingerprint) {
            // The latest state returned to the last delivered match before
            // the interval elapsed; discard a superseded pending change.
            return (None, self.pending.take().is_some());
        }
        let now = Instant::now();
        let interval = Duration::from_millis(policy.notification_interval_ms.unwrap_or(30_000));
        if self
            .last_notification
            .is_some_and(|last| now.duration_since(last) < interval)
        {
            if self
                .pending
                .as_ref()
                .is_some_and(|(_, hash)| *hash == fingerprint)
            {
                return (None, false);
            }
            let mut pending = event.clone();
            pending.line = line.into();
            self.pending = Some((pending, fingerprint));
            return (None, true);
        }
        self.notified = true;
        self.last_notification = Some(now);
        self.last_fingerprint = Some(fingerprint);
        self.pending = None;
        (Some("match"), false)
    }
}

#[derive(Default)]
pub(super) struct OutputMatcher {
    revision: u64,
    pending: String,
    last: Option<ProcessEvent>,
}

impl OutputMatcher {
    pub fn push(&mut self, state: &MonitorState, event: &ProcessEvent, eof: bool) {
        let (revision, enabled, since_seq) = {
            let watch = state.watch.lock().unwrap();
            (watch.revision, watch.policy.is_some(), watch.since_seq)
        };
        if self.revision != revision || !enabled || event.seq <= since_seq {
            self.pending.clear();
            self.revision = revision;
        }
        if !enabled || event.seq <= since_seq {
            self.last = None;
            return;
        }
        self.last = Some(event.clone());
        for part in event.line.split_inclusive('\n') {
            self.pending.push_str(part);
            if part.ends_with('\n') {
                let line = self.pending.trim_end_matches(['\r', '\n']);
                self.publish(state, event, line, true);
                self.pending.clear();
            } else if self.pending.len() >= READER_LINE_BYTE_CAP {
                // Bound pattern scanning without dropping the raw captured
                // chunk. The notification explicitly identifies the cut.
                let line = format!(
                    "{}\n[watch record split after {} bytes; full raw text is in shell.read/output_archive]",
                    self.pending,
                    self.pending.len()
                );
                self.publish(state, event, &line, true);
                self.pending.clear();
            } else {
                // A service may report readiness without a newline. Terminal
                // and recurring include matches require a completed record.
                self.publish(state, event, &self.pending, false);
            }
        }
        if eof {
            self.finish(state);
        }
    }

    fn publish(&self, state: &MonitorState, event: &ProcessEvent, line: &str, complete: bool) {
        let (notification, changed) = {
            let mut watch = state.watch.lock().unwrap();
            if watch.revision != self.revision {
                return;
            }
            watch.observe(event, line, complete)
        };
        if changed {
            state.watch_changed.notify_waiters();
        }
        if let Some(kind) = notification {
            let mut event = event.clone();
            event.line = line.to_owned();
            emit_notification(state, event, kind, self.revision);
        }
    }

    pub fn finish(&mut self, state: &MonitorState) {
        if let Some(last) = &self.last
            && !self.pending.is_empty()
        {
            self.publish(state, last, &self.pending, true);
        }
        self.pending.clear();
        self.last = None;
    }
}

fn emit_notification(state: &MonitorState, mut event: ProcessEvent, kind: &str, revision: u64) {
    // Assign and publish together. A delayed stdout observer cannot arrive
    // after a newer stderr notification has already advanced durable delivery.
    let _delivery = state.notification_delivery.lock().unwrap();
    {
        let watch = state.watch.lock().unwrap();
        // A terminal match in the other stream may have won after observe()
        // selected this notification but before it acquired delivery.
        if watch.revision != revision || watch.outcome != 0 {
            return;
        }
    }
    // A fast process may be draining its final READY record after it already
    // exited. Report final completion instead of waking the AI to use a
    // service whose termination has already been committed.
    if kind == "ready" && state.inner.lock().unwrap().ending {
        return;
    }
    if let Some(listener) = &state.listener {
        event.notification = Some(kind.into());
        event.notification_seq = Some(state.notification_seq.fetch_add(1, Ordering::AcqRel) + 1);
        listener.on_event(&event, &state.snapshot());
    }
}

pub(super) fn flush_pending(state: &MonitorState) {
    let next = {
        let mut watch = state.watch.lock().unwrap();
        if watch.outcome != 0 {
            watch.pending = None;
            return;
        }
        watch.pending.take().map(|(event, hash)| {
            watch.last_fingerprint = Some(hash);
            watch.last_notification = Some(Instant::now());
            (event, watch.revision)
        })
    };
    if let Some((event, revision)) = next {
        emit_notification(state, event, "match", revision);
    }
}

pub(super) async fn deliver_notifications(state: Arc<MonitorState>) {
    loop {
        let changed = state.watch_changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let remaining = {
            let watch = state.watch.lock().unwrap();
            if watch.pending.is_some() && watch.outcome == 0 {
                Some(
                    Duration::from_millis(
                        watch
                            .policy
                            .as_ref()
                            .and_then(|policy| policy.notification_interval_ms)
                            .unwrap_or(30_000),
                    )
                    .saturating_sub(
                        watch
                            .last_notification
                            .map_or(Duration::ZERO, |last| last.elapsed()),
                    ),
                )
            } else {
                None
            }
        };
        match remaining {
            None => changed.await,
            Some(remaining) if remaining.is_zero() => flush_pending(&state),
            Some(remaining) => tokio::select! {
                _ = changed => {},
                _ = tokio::time::sleep(remaining) => {},
            },
        }
    }
}

pub(super) async fn wait_for_condition(state: &MonitorState) -> (PatternOutcome, u64) {
    loop {
        let changed = state.watch_changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let outcome = {
            let watch = state.watch.lock().unwrap();
            (watch.outcome, watch.revision)
        };
        match outcome {
            (2, revision) => return (PatternOutcome::Failure, revision),
            (1, revision) => return (PatternOutcome::Success, revision),
            _ => {}
        }
        changed.await;
    }
}

pub(super) async fn wait_for_quiet(state: &MonitorState) -> u64 {
    loop {
        let changed = state.watch_changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let quiet = {
            let watch = state.watch.lock().unwrap();
            watch
                .policy
                .as_ref()
                .and_then(|policy| policy.quiet_period_ms)
                .map(|ms| {
                    (
                        Duration::from_millis(ms),
                        watch.configured_at,
                        watch.revision,
                    )
                })
        };
        let Some((period, configured, revision)) = quiet else {
            changed.await;
            continue;
        };
        let last = (*state.last_activity.lock().unwrap()).max(configured);
        let remaining = period.saturating_sub(last.elapsed());
        if remaining.is_zero() {
            if state
                .output_archive
                .as_ref()
                .is_some_and(|archive| archive.capture_busy())
            {
                // Archive pressure is not silence. Allow readers to release
                // staged output and observe pending pipe activity first.
                tokio::select! {
                    _ = changed => {},
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                }
                continue;
            }
            return revision;
        }
        tokio::select! {
            _ = changed => {},
            _ = tokio::time::sleep(remaining) => {},
        }
    }
}
