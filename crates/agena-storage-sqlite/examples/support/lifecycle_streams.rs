//! B07: bounded producers and range readers over substantial real output.

use std::{collections::VecDeque, future::Future};

use agena_domain::{
    CommandOutputStream, ContentDocument, ContentId, ContentInput, DocumentMutation,
    TerminalCellRun, TerminalSnapshot, ViewBlock,
};
use sha2::{Digest, Sha256};

use super::*;

const MIXED_PACKET_BYTES: usize = 16 * 1024;

async fn admitted<T, F, Fut>(busy: &mut u64, mut operation: F) -> AnyResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = agena_storage::kernel::Result<T>>,
{
    let started = Instant::now();
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(KernelError::Busy) if started.elapsed() < Duration::from_secs(30) => {
                *busy += 1;
                // Busy means the packet was not admitted. Only that packet is
                // retried, preserving the external receive order exactly.
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn channel(index: u64) -> CommandOutputStream {
    if index.is_multiple_of(2) {
        CommandOutputStream::Stdout
    } else {
        CommandOutputStream::Stderr
    }
}

fn block(text: String) -> ViewBlock {
    ViewBlock::Text {
        id: Some("body".into()),
        text,
    }
}

fn screen(
    rows: &[String],
    cursor_row: u16,
    cursor_col: u16,
    only: Option<u16>,
) -> TerminalSnapshot {
    TerminalSnapshot {
        rows: rows.len() as u16,
        cols: 80,
        cursor_row,
        cursor_col,
        cursor_visible: true,
        alternate_screen: false,
        bracketed_paste: true,
        application_cursor: false,
        cells: rows
            .iter()
            .enumerate()
            .filter(|(row, _)| only.is_none_or(|only| *row == usize::from(only)))
            .map(|(row, text)| TerminalCellRun {
                row: row as u16,
                col: 0,
                text: text.clone(),
                attributes: Default::default(),
            })
            .collect(),
    }
}

async fn retained_cursor(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
) -> AnyResult<EventCursor> {
    let head = reader.watch_content(session, content).await?.head()?;
    Ok(EventCursor {
        content,
        epoch: head.epoch,
        sequence: head.retained_start,
    })
}

async fn verify_log(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
    expected: [u8; 32],
    expected_bytes: usize,
    expected_events: usize,
    alternating: bool,
) -> AnyResult<Value> {
    let started = Instant::now();
    let mut checksum = Sha256::new();
    let mut cursor = None;
    let mut bytes = 0;
    let mut events = 0;
    let mut pages = 0;
    let mut max_page_bytes = 0;
    loop {
        let page = reader.event_page(session, content, cursor, 32).await?;
        if page.records.is_empty() && page.has_more {
            return Err("B07 log page made no progress".into());
        }
        let mut page_bytes = 0;
        for record in &page.records {
            let EventPayload::Log {
                channel: observed,
                text,
            } = &record.payload
            else {
                return Err("B07 log exposed a non-log payload".into());
            };
            if record.sequence != events as u64 + 1
                || (alternating && *observed != channel(events as u64))
            {
                return Err("B07 stdout/stderr receive order or sequence changed".into());
            }
            checksum.update(text.as_bytes());
            bytes += text.len();
            page_bytes += text.len();
            events += 1;
        }
        pages += 1;
        max_page_bytes = max_page_bytes.max(page_bytes);
        cursor = Some(page.next);
        if !page.has_more {
            break;
        }
    }
    let digest: [u8; 32] = checksum.finalize().into();
    if digest != expected || bytes != expected_bytes || events != expected_events {
        return Err("B07 log range checksum/count mismatch".into());
    }
    let tail = reader
        .event_page(
            session,
            content,
            Some(EventCursor {
                content,
                epoch: cursor.ok_or("B07 log omitted its cursor")?.epoch,
                sequence: events.saturating_sub(8) as u64,
            }),
            8,
        )
        .await?;
    if tail.records.len() != events.min(8) || tail.has_more {
        return Err("B07 bounded log tail read mismatch".into());
    }
    Ok(
        json!({"bytes":bytes,"events":events,"pages":pages,"max_page_payload_bytes":max_page_bytes,"sha256":hex::encode(digest),"tail_events":tail.records.len(),"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}),
    )
}

async fn verify_retained_log(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
    total_events: usize,
    pty: bool,
) -> AnyResult<Value> {
    let started = Instant::now();
    let start = retained_cursor(reader, session, content).await?;
    let mut cursor = Some(start);
    let mut next_sequence = start.sequence + 1;
    let mut checksum = Sha256::new();
    let mut records = 0;
    let mut bytes = 0;
    let retained_start = start.sequence;
    loop {
        let page = reader.event_page(session, content, cursor, 32).await?;
        if page.records.is_empty() && page.has_more {
            return Err("B07 retained log failed to make progress".into());
        }
        for record in &page.records {
            let index = record.sequence.saturating_sub(1);
            let expected = if pty {
                format!("PTY frame {index}: {}\r\n", ascii(1024, 0xb0732000 ^ index))
            } else {
                ascii(PACKET_BYTES, 0xb0700000 ^ index)
            };
            let EventPayload::Log {
                channel: observed,
                text,
            } = &record.payload
            else {
                return Err("B07 retained log changed its payload kind".into());
            };
            if record.sequence != next_sequence
                || text != &expected
                || *observed
                    != if pty {
                        CommandOutputStream::Stdout
                    } else {
                        channel(index)
                    }
            {
                return Err(
                    "B07 retained log suffix changed its bytes, channel or sequence".into(),
                );
            }
            next_sequence += 1;
            records += 1;
            bytes += text.len();
            checksum.update(text.as_bytes());
        }
        cursor = Some(page.next);
        if !page.has_more {
            break;
        }
    }
    if records != total_events.saturating_sub(retained_start as usize) {
        return Err("B07 retention lost events beyond its declared prefix".into());
    }
    Ok(
        json!({"retained_start":retained_start,"suffix_events":records,"suffix_bytes":bytes,"suffix_sha256":hex::encode(checksum.finalize()),"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}),
    )
}

async fn verify_projection(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
    kind: EventKind,
    document: Option<&ContentDocument>,
    terminal: Option<&TerminalSnapshot>,
    expected_text: Option<([u8; 32], usize)>,
) -> AnyResult<Value> {
    let started = Instant::now();
    let mut projection = None;
    let mut checksum = Sha256::new();
    let mut text_bytes = 0;
    let mut cursor = Some(retained_cursor(reader, session, content).await?);
    let mut pages = 0;
    let mut records = 0;
    let mut checkpoints = 0;
    loop {
        let page = reader.event_page(session, content, cursor, 32).await?;
        if page.records.is_empty() && page.has_more {
            return Err("B07 projection range made no progress".into());
        }
        for record in &page.records {
            if let EventPayload::Mixed { input, .. } = &record.payload
                && let Some(text) = input.text_content()
            {
                checksum.update(text.as_bytes());
                text_bytes += text.len();
            }
            if record.payload.is_checkpoint() {
                checkpoints += 1;
            }
            if let Some(projection) = projection.as_mut() {
                EventProjection::apply(projection, record)?;
            } else if record.payload.is_checkpoint() {
                projection = Some(EventProjection::from_checkpoint(record)?);
            } else {
                let mut next = EventProjection::new(kind);
                next.apply(record)?;
                projection = Some(next);
            }
            records += 1;
        }
        pages += 1;
        cursor = Some(page.next);
        if !page.has_more {
            break;
        }
    }
    let projection: EventProjection = projection.ok_or("B07 projection had no checkpoint")?;
    if projection.document() != document || projection.terminal() != terminal {
        return Err("B07 replay projection differs from its producer state".into());
    }
    let snapshot = reader.event_snapshot(session, content).await?;
    if snapshot.document.as_deref() != document
        || snapshot.terminal.as_deref() != terminal
        || snapshot.through != cursor.ok_or("B07 projection omitted its cursor")?
    {
        return Err("B07 indexed snapshot differs from range replay".into());
    }
    let digest: [u8; 32] = checksum.finalize().into();
    if expected_text.is_some_and(|(expected, bytes)| expected != digest || bytes != text_bytes) {
        return Err("B07 mixed text/log receive checksum mismatch".into());
    }
    Ok(
        json!({"records_read":records,"pages":pages,"checkpoints_read":checkpoints,"through":snapshot.through,"text_bytes":text_bytes,"text_sha256":expected_text.map(|_|hex::encode(digest)),"document_bytes":document.map(ContentDocument::byte_len),"terminal_cells":terminal.map(|screen|usize::from(screen.rows)*usize::from(screen.cols)),"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}),
    )
}

async fn verify_losses(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
    expected_bytes: u64,
) -> AnyResult<Value> {
    let page = reader.capture_losses(session, content, None, 16).await?;
    let bytes = page.losses.iter().map(|loss| loss.bytes).sum::<u64>();
    if page.has_more
        || bytes != expected_bytes
        || page.stream.capture_loss.committed_bytes != expected_bytes
        || page.stream.capture_loss.committed_packets != 1
    {
        return Err("B07 capture loss was missing from durable range observations".into());
    }
    Ok(json!({"stream":page.stream,"windows":page.losses,"next":page.next}))
}

async fn retain(kernel: &SqliteKernel, session: SessionId, content: ContentId) -> AnyResult<Value> {
    let head = kernel
        .ports
        .reader
        .watch_content(session, content)
        .await?
        .head()?;
    let before = head.committed * 3 / 4;
    let epoch = head.epoch;
    let old_blocks = kernel.diagnostics.snapshot();
    let usage = resources::sample();
    let started = Instant::now();
    drop(head);
    let mut calls = 0;
    let mut blocks_removed = 0;
    let final_stream = loop {
        let receipt = kernel
            .maintenance
            .prune_content(RequestId::new(), session, content, before)
            .await?
            .value;
        calls += 1;
        blocks_removed += u64::from(receipt.blocks_removed);
        if !receipt.pending_reclamation {
            break receipt.stream;
        }
        if calls > 100000 {
            return Err("B07 retention failed to make bounded progress".into());
        }
    };
    let gap = match kernel
        .ports
        .reader
        .event_page(
            session,
            content,
            Some(EventCursor {
                content,
                epoch,
                sequence: 0,
            }),
            1,
        )
        .await
    {
        Err(KernelError::ContentGap {
            requested,
            retained_start,
            ..
        }) if requested == 0 && retained_start == final_stream.retained_start => true,
        Ok(_) if final_stream.retained_start == 0 => false,
        Err(error) => return Err(error.into()),
        Ok(_) => return Err("B07 old cursor silently bypassed its retention gap".into()),
    };
    Ok(
        json!({"content":content,"requested_prefix":before,"actual_retained_start":final_stream.retained_start,"old_cursor_reported_gap":gap,"checkpoint_alignment_may_preserve_earlier_prefix":true,"bounded_reclamation_calls":calls,"blocks_removed":blocks_removed,"stream":final_stream,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"io":io_delta(usage,resources::sample()),"counters":metric_delta(&old_blocks,&kernel.diagnostics.snapshot())?}),
    )
}

pub(super) async fn run(
    log_mib: usize,
    mixed_mib: usize,
    frames: usize,
    document_events: usize,
) -> AnyResult<()> {
    if log_mib == 0
        || mixed_mib == 0
        || log_mib > 4096
        || mixed_mib > 1024
        || frames == 0
        || frames > 100000
        || document_events == 0
        || document_events > 100000
    {
        return Err(
            "B07 expects 1..4096 log MiB, 1..1024 mixed MiB and 1..100000 PTY/document events"
                .into(),
        );
    }
    let root = owned_root()?;
    let kernel = open_kernel_runtime(&database_url(root.path())).await?;
    let inspect = inspect(root.path()).await?;
    let (session, execution, attempt) =
        start_attempt(&kernel, root.path(), "B07 long output").await?;
    let round = attempt
        .begin_round(RequestId::new(), vec![execution.message_id])
        .await?
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "B07-producer".into(),
                name: "resource-probe".into(),
                input: json!({"synthetic_kernel_producer":true}),
            },
        )
        .await?
        .value;
    let log = tool.begin_log(RequestId::new()).await?.value;
    let mixed = tool.begin_mixed(RequestId::new()).await?.value;
    let document = tool.begin_document(RequestId::new()).await?.value;
    let activity = tool
        .begin_activity(
            RequestId::new(),
            BackgroundActivityStart {
                kind: BackgroundActivityKind::Terminal,
                label: "B07 synthetic PTY frame producer".into(),
            },
        )
        .await?
        .value;
    let terminal = activity.begin_terminal(RequestId::new()).await?.value;
    let pty_log = activity.begin_log(RequestId::new()).await?.value;
    let initial = resources::sample();
    let baseline = stored(&inspect, root.path()).await?;
    let sampler = Sampler::start();
    let started = Instant::now();
    let mut busy = 0;
    emit(
        json!({"case":"b07","phase":"baseline","log_mib":log_mib,"mixed_mib":mixed_mib,"pty_frames":frames,"document_mutations":document_events,"producer":"real typed kernel capabilities with deterministic synthetic bytes/terminal frames; no provider or external OS PTY is included","packet_bytes":PACKET_BYTES,"mixed_packet_bytes":MIXED_PACKET_BYTES,"database":baseline,"diagnostics":diagnostics(&kernel),"usage":initial}),
    )?;

    let log_packets = log_mib * 1024 * 1024 / PACKET_BYTES;
    let mut expected_log = Sha256::new();
    let mut slow = VecDeque::new();
    let phase_start = Instant::now();
    let phase_usage = resources::sample();
    for index in 0..log_packets {
        let packet = ascii(PACKET_BYTES, 0xb0700000 ^ index as u64);
        let cursor = admitted(&mut busy, || log.append(channel(index as u64), &packet)).await?;
        if cursor.sequence != index as u64 + 1 {
            return Err("B07 accepted log sequence mismatch".into());
        }
        expected_log.update(packet.as_bytes());
        if index == log_packets / 2 {
            log.record_loss(12345, "B07 deliberate bounded external log capture loss")?;
        }
        if index % (log_packets / 8).max(1) == 0 {
            slow.push_back(
                kernel
                    .ports
                    .reader
                    .event_page(
                        session,
                        log.id(),
                        Some(EventCursor {
                            content: log.id(),
                            epoch: log.epoch(),
                            sequence: index as u64,
                        }),
                        1,
                    )
                    .await?,
            );
            while slow.len() > 4 {
                slow.pop_front();
            }
        }
        if (index + 1) % (log_packets / 4).max(1) == 0 {
            emit(
                json!({"case":"b07","phase":"log_progress","accepted_payload_bytes":(index+1)*PACKET_BYTES,"accepted_events":index+1,"slow_reader_views":slow.len(),"slow_reader_modeled_bytes":slow.iter().map(ReadView::modeled_bytes).sum::<usize>(),"usage":resources::sample(),"sampling":sampler.summary(),"diagnostics":diagnostics(&kernel)}),
            )?;
        }
    }
    let log_head = log
        .interrupt(
            RequestId::new(),
            "B07 capture completed with deliberate loss; business outcome is recorded separately"
                .into(),
        )
        .await?
        .value;
    let expected_log: [u8; 32] = expected_log.finalize().into();
    let log_read = verify_log(
        &kernel.ports.reader,
        session,
        log.id(),
        expected_log,
        log_mib * 1024 * 1024,
        log_packets,
        true,
    )
    .await?;
    let log_loss = verify_losses(&kernel.ports.reader, session, log.id(), 12345).await?;
    emit(
        json!({"case":"b07","phase":"log_sealed","elapsed_ms":phase_start.elapsed().as_secs_f64()*1000.0,"io":io_delta(phase_usage,resources::sample()),"head":log_head,"range_verification":log_read,"capture_loss":log_loss,"held_slow_reader_views":slow.len(),"diagnostics":diagnostics(&kernel),"database":stored(&inspect,root.path()).await?,"sampling":sampler.summary()}),
    )?;
    slow.clear();

    let mut mixed_document = ContentDocument::default();
    let insert = DocumentMutation::Insert {
        after: None,
        block: block(ascii(8192, 0xb071)),
    };
    mixed_document = mixed_document.updated(&insert)?;
    admitted(&mut busy, || {
        mixed.append(ContentInput::Structured {
            event: insert.clone(),
        })
    })
    .await?;
    let mixed_packets = mixed_mib * 1024 * 1024 / MIXED_PACKET_BYTES;
    let mut expected_mixed = Sha256::new();
    let phase_start = Instant::now();
    for index in 0..mixed_packets {
        let packet = ascii(MIXED_PACKET_BYTES, 0xb0710000 ^ index as u64);
        let input = if index.is_multiple_of(3) {
            ContentInput::Text {
                text: packet.clone(),
            }
        } else {
            ContentInput::Log {
                stream: channel((index % 3 - 1) as u64),
                text: packet.clone(),
            }
        };
        admitted(&mut busy, || mixed.append(input.clone())).await?;
        expected_mixed.update(packet.as_bytes());
        if index.is_multiple_of(16) {
            let mutation = DocumentMutation::Replace {
                block: block(ascii(8192, 0xb0711000 ^ index as u64)),
            };
            mixed_document = mixed_document.updated(&mutation)?;
            admitted(&mut busy, || {
                mixed.append(ContentInput::Structured {
                    event: mutation.clone(),
                })
            })
            .await?;
        }
    }
    mixed.finish(RequestId::new()).await?;
    let expected_mixed: [u8; 32] = expected_mixed.finalize().into();
    let mixed_read = verify_projection(
        &kernel.ports.reader,
        session,
        mixed.id(),
        EventKind::Mixed,
        Some(&mixed_document),
        None,
        Some((expected_mixed, mixed_mib * 1024 * 1024)),
    )
    .await?;
    emit(
        json!({"case":"b07","phase":"mixed_sealed","elapsed_ms":phase_start.elapsed().as_secs_f64()*1000.0,"range_verification":mixed_read,"document_bytes":mixed_document.byte_len(),"diagnostics":diagnostics(&kernel),"database":stored(&inspect,root.path()).await?,"sampling":sampler.summary()}),
    )?;

    let mut structured_document = ContentDocument::default();
    let phase_start = Instant::now();
    for index in 0..document_events {
        let value = block(ascii(8192, 0xb0720000 ^ index as u64));
        let mutation = if index == 0 {
            DocumentMutation::Insert {
                after: None,
                block: value,
            }
        } else {
            DocumentMutation::Replace { block: value }
        };
        structured_document = structured_document.updated(&mutation)?;
        admitted(&mut busy, || document.apply(mutation.clone())).await?;
    }
    document.finish(RequestId::new()).await?;
    let document_read = verify_projection(
        &kernel.ports.reader,
        session,
        document.id(),
        EventKind::Structured,
        Some(&structured_document),
        None,
        None,
    )
    .await?;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"exit_code":0,"capture_loss_recorded_separately":true}),
        },
    )
    .await?;
    round.finish(RequestId::new(), None, true).await?;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await?;
    if kernel
        .ports
        .reader
        .active_activities(session, None, 8)
        .await?
        .activities
        .len()
        != 1
        || !kernel
            .ports
            .reader
            .active_executions(session, None, 8)
            .await?
            .executions
            .is_empty()
    {
        return Err("B07 background terminal did not outlive its completed launch attempt".into());
    }
    emit(
        json!({"case":"b07","phase":"parent_succeeded_background_open","elapsed_ms":phase_start.elapsed().as_secs_f64()*1000.0,"tool_phase":kernel.ports.reader.tool(session,tool.id()).await?.phase,"execution_phase":kernel.ports.reader.execution(session,execution.id).await?.phase,"structured_range_verification":document_read,"live_activities":1,"diagnostics":diagnostics(&kernel),"sampling":sampler.summary()}),
    )?;

    let mut rows = (0..24)
        .map(|row| ascii(80, 0xb0730000 ^ row))
        .collect::<Vec<_>>();
    let mut previous = None;
    let mut expected_pty = Sha256::new();
    let mut pty_bytes = 0;
    let mut final_screen = None;
    let phase_start = Instant::now();
    for index in 0..frames {
        let row = (index % 24) as u16;
        rows[usize::from(row)] = ascii(80, 0xb0731000 ^ index as u64);
        let complete = screen(&rows, row, (index % 80) as u16, None);
        let cursor = if index.is_multiple_of(16) {
            admitted(&mut busy, || terminal.snapshot(complete.clone())).await?
        } else {
            let patch = screen(&rows, row, (index % 80) as u16, Some(row));
            let base = previous.ok_or("B07 terminal patch lost its base")?;
            admitted(&mut busy, || terminal.patch(base, patch.clone(), vec![row])).await?
        };
        previous = Some(cursor);
        final_screen = Some(complete);
        let packet = format!(
            "PTY frame {index}: {}\r\n",
            ascii(1024, 0xb0732000 ^ index as u64)
        );
        admitted(&mut busy, || {
            pty_log.append(CommandOutputStream::Stdout, &packet)
        })
        .await?;
        expected_pty.update(packet.as_bytes());
        pty_bytes += packet.len();
        if index == frames / 2 {
            terminal.record_loss(42, "B07 deliberate terminal input capture loss")?;
        }
    }
    let terminal_head = terminal
        .interrupt(
            RequestId::new(),
            "B07 terminal view lost an external packet".into(),
        )
        .await?
        .value;
    let raw_head = pty_log.finish(RequestId::new()).await?.value;
    activity
        .end(
            RequestId::new(),
            BackgroundActivityOutcome::Succeeded {
                value: json!({"exit_code":0,"frames":frames}),
            },
        )
        .await?;
    let terminal_read = verify_projection(
        &kernel.ports.reader,
        session,
        terminal.id(),
        EventKind::Terminal,
        None,
        final_screen.as_ref(),
        None,
    )
    .await?;
    let expected_pty: [u8; 32] = expected_pty.finalize().into();
    let raw_read = verify_log(
        &kernel.ports.reader,
        session,
        pty_log.id(),
        expected_pty,
        pty_bytes,
        frames,
        false,
    )
    .await?;
    let terminal_loss = verify_losses(&kernel.ports.reader, session, terminal.id(), 42).await?;
    let activity_view = kernel.ports.reader.activity(session, activity.id()).await?;
    if !matches!(
        &activity_view.outcome,
        Some(BackgroundActivityOutcome::Succeeded { .. })
    ) || terminal_head.state != CaptureState::Interrupted
        || raw_head.state != CaptureState::Complete
    {
        return Err("B07 terminal capture and process outcomes were conflated".into());
    }
    emit(
        json!({"case":"b07","phase":"background_succeeded_capture_interrupted","elapsed_ms":phase_start.elapsed().as_secs_f64()*1000.0,"terminal_head":terminal_head,"raw_head":raw_head,"activity":*activity_view,"terminal_range_verification":terminal_read,"raw_log_range_verification":raw_read,"capture_loss":terminal_loss,"busy_retries":busy,"diagnostics":diagnostics(&kernel),"database":stored(&inspect,root.path()).await?,"sampling":sampler.summary()}),
    )?;
    drop(activity_view);

    let before_retention = stored(&inspect, root.path()).await?;
    for content in [
        log.id(),
        mixed.id(),
        document.id(),
        terminal.id(),
        pty_log.id(),
    ] {
        let retention = retain(&kernel, session, content).await?;
        emit(
            json!({"case":"b07","phase":"retention_source","retention":retention,"diagnostics":diagnostics(&kernel),"sampling":sampler.summary()}),
        )?;
    }
    let mixed_retained = verify_projection(
        &kernel.ports.reader,
        session,
        mixed.id(),
        EventKind::Mixed,
        Some(&mixed_document),
        None,
        None,
    )
    .await?;
    let document_retained = verify_projection(
        &kernel.ports.reader,
        session,
        document.id(),
        EventKind::Structured,
        Some(&structured_document),
        None,
        None,
    )
    .await?;
    let terminal_retained = verify_projection(
        &kernel.ports.reader,
        session,
        terminal.id(),
        EventKind::Terminal,
        None,
        final_screen.as_ref(),
        None,
    )
    .await?;
    let log_retained =
        verify_retained_log(&kernel.ports.reader, session, log.id(), log_packets, false).await?;
    let raw_retained =
        verify_retained_log(&kernel.ports.reader, session, pty_log.id(), frames, true).await?;
    emit(
        json!({"case":"b07","phase":"retention_complete","database_before":before_retention,"database_after":stored(&inspect,root.path()).await?,"retained_mixed_projection":mixed_retained,"retained_document_projection":document_retained,"retained_terminal_projection":terminal_retained,"retained_log":log_retained,"retained_raw_pty_log":raw_retained,"diagnostics":diagnostics(&kernel),"sampling":sampler.summary(),"physical_reclamation":"removed blocks become SQLite freelist pages; database file truncation/VACUUM is not claimed"}),
    )?;
    drop((
        log, mixed, document, terminal, pty_log, activity, tool, round, attempt,
    ));
    let final_diagnostics = kernel.diagnostics.clone();
    kernel.ports.clone().shutdown().await?;
    inspect.close().await;
    drop(kernel);
    emit(
        json!({"case":"b07","phase":"closed","elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"usage":resources::sample(),"sampling":sampler.summary(),"total_io":io_delta(initial,resources::sample()),"read_budget_after_close":final_diagnostics.read_budget(),"wal_checkpoint_after_close":closed_checkpoint(root.path()).await?}),
    )?;
    Ok(())
}
