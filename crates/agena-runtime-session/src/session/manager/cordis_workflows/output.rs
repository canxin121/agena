use super::*;

pub(super) const SOURCE_GAP: &str = "[source output omitted]\n";

impl SessionManager {
    pub(super) async fn packet_output(
        &self,
        session: SessionId,
        activity: ActivityId,
        from: Option<u64>,
    ) -> Result<ReadView<Option<ActivityOutputPage>>, AppError> {
        read_with_busy_retry(session, "cordis_output_packets", || {
            self.kernel()
                .reader
                .activity_output(session, activity, from)
        })
        .await
    }

    pub(super) async fn append_job_output(
        &self,
        job: &RunningJob,
        total: u64,
        chunks: &[ActivityOutputChunk],
    ) -> Result<(), AppError> {
        let request = RequestId::new();
        persist(|| job.activity.append_output_chunks(request, total, chunks))
            .await
            .map_err(kernel_error)?;
        Ok(())
    }

    pub(super) async fn inline_job_output(
        &self,
        job: &RunningJob,
        room: usize,
        include_log: bool,
    ) -> Option<(String, ActivityOutputPosition)> {
        let session = job.activity.session_id();
        let id = job.activity.id();
        let position = self.cordis_output_position(session, id).await.ok()?;
        let from = position.packet_offset.unwrap_or_default();
        let output = self.packet_output(session, id, Some(from)).await.ok()?;
        let output = output.as_ref()?;
        if is_lossy(output, from) || position.outcome_read {
            return None;
        }
        let text = model_text(output, from, include_log);
        (text.len() <= room).then_some((
            text,
            ActivityOutputPosition {
                packet_offset: Some(output.head.total),
                outcome_read: true,
                ..Default::default()
            },
        ))
    }

    /// Older activity records have a single retained text capture. Its
    /// packet boundaries and original channels cannot be inferred from it.
    pub(super) async fn legacy_job_output(
        &self,
        activity: &BackgroundActivityView,
        from: Option<u64>,
        consume: bool,
    ) -> Result<(ActivityOutputPage, ActivityOutputPosition), AppError> {
        let session = activity.origin_session_id;
        let tool = read_with_busy_retry(session, "cordis_legacy_output", || {
            self.kernel().reader.tool(session, activity.launch_tool)
        })
        .await?;
        let empty = ActivityOutputPage {
            head: ActivityOutputHead::default(),
            chunks: Vec::new(),
            terminal: None,
            spill_paths: Vec::new(),
        };
        let Some(source) = tool.outputs.iter().find(|source| {
            source.activity_id == Some(activity.id)
                && source.kind == agena_domain::ContentKind::Text
        }) else {
            return Ok((empty, ActivityOutputPosition::default()));
        };
        let reader = &self.kernel().reader;
        let watch = read_with_busy_retry(session, "cordis_legacy_output_head", || {
            reader.watch_content(session, source.id)
        })
        .await?;
        let head = watch.head().map_err(kernel_error)?;
        let mut chunks = Vec::new();
        let start = head.retained_start;
        let end = head.committed;
        if start < end && from.is_some_and(|offset| offset < end) {
            let read_start = if consume {
                start.max(from.unwrap_or_default())
            } else {
                start
            };
            let cursor = TextCursor {
                content: head.id,
                epoch: head.epoch,
                offset: read_start,
            };
            let range = read_with_busy_retry(session, "cordis_legacy_output_text", || {
                reader.text_range(
                    session,
                    source.id,
                    Some(cursor),
                    (end - read_start) as usize,
                )
            })
            .await?;
            let first = range
                .bytes
                .iter()
                .position(|byte| byte & 0xc0 != 0x80)
                .unwrap_or(range.bytes.len());
            chunks.push(ActivityOutputChunk {
                at: read_start + first as u64,
                text: String::from_utf8_lossy(&range.bytes[first..]).into_owned(),
                channel: None,
                gap_before: start > 0 || head.capture_loss.committed_bytes > 0,
            });
        }
        let position = ActivityOutputPosition {
            cursor: Some(TextCursor {
                content: head.id,
                epoch: head.epoch,
                offset: end,
            }),
            lost_bytes: head.capture_loss.committed_bytes,
            ..Default::default()
        };
        Ok((
            ActivityOutputPage {
                head: ActivityOutputHead {
                    earliest: start,
                    total: end,
                },
                chunks,
                terminal: None,
                spill_paths: Vec::new(),
            },
            position,
        ))
    }
}

pub(super) fn is_lossy(output: &ActivityOutputPage, from: u64) -> bool {
    from < output.head.earliest
        || (from < output.head.total
            && output
                .chunks
                .last()
                .is_none_or(|chunk| chunk.at + (chunk.text.len() as u64) < output.head.total))
}

pub(super) fn model_text(output: &ActivityOutputPage, from: u64, include_log: bool) -> String {
    let mut text = String::new();
    let omitted = output.head.earliest.saturating_sub(from);
    let mut lossy = omitted > 0;
    if omitted > 0 {
        text.push_str(&format!("[{omitted} bytes of earlier progress omitted]\n"));
    }
    for chunk in &output.chunks {
        if !include_log && chunk.channel == Some(ActivityOutputChannel::Log) {
            continue;
        }
        if chunk.gap_before {
            lossy = true;
            text.push_str(SOURCE_GAP);
        }
        text.push_str(&chunk.text);
    }
    // A bounded bridge queue may lose the end of a stream before the producer
    // sends another packet to carry a gap marker.
    let last = output
        .chunks
        .last()
        .map_or(from, |chunk| chunk.at + chunk.text.len() as u64);
    if last < output.head.total {
        lossy = true;
        text.push_str(SOURCE_GAP);
    }
    if lossy && !output.spill_paths.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("[some output was dropped from memory; full output: ");
        text.push_str(&output.spill_paths.join(", "));
        text.push(']');
    }
    text
}

pub(super) fn model_tail(mut text: String, budget: usize) -> String {
    if text.len() <= budget {
        return text;
    }
    let marker = "[earlier progress omitted]\n";
    let keep = budget.saturating_sub(marker.len());
    let start = text.ceil_char_boundary(text.len() - keep);
    text.replace_range(..start, marker);
    results_text::truncate(&mut text, budget);
    text
}

pub(super) fn terminal_result(
    activity: &BackgroundActivityView,
    label: JobLabel<'_>,
    output: Option<&ActivityOutputPage>,
) -> Option<String> {
    if let Some(result) = output.and_then(|output| output.terminal.as_ref()?.result.as_ref()) {
        return Some(result.clone());
    }
    let outcome = activity.outcome.as_ref()?;
    if let Some(result) = producers::producer_result(outcome) {
        return Some(result.to_owned());
    }
    let BackgroundActivityOutcome::Succeeded { value } = outcome else {
        return None;
    };
    let result = stored_result(value)?;
    let max_chars = value
        .get("maxResultChars")
        .and_then(Value::as_u64)
        .unwrap_or(50_000);
    Some(render_result(label.text, max_chars as usize, &result))
}
