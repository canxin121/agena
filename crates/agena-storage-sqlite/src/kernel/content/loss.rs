//! Bounded capture observations. Durable windows are immutable and remain
//! available after byte/event prefix retention.
use super::*;

pub(in crate::kernel) fn record(
    backend: &Backend,
    scope: OutputScope,
    content: ContentId,
    epoch: ContentEpoch,
    bytes: u64,
    diagnostic: &str,
) -> Result<()> {
    backend.cache.ensure_open()?;
    let mut registry = backend.live.0.lock().unwrap_or_else(|e| e.into_inner());
    backend.cache.ensure_open()?;
    let source = registry
        .sources
        .get_mut(&content)
        .ok_or_else(|| KernelError::Conflict("content capture is closed".into()))?;
    source.scope.check(scope)?;
    if source.head.epoch != epoch {
        return Err(KernelError::ContentReset);
    }
    if source.head.state != CaptureState::Open || source.abandoned {
        return Err(KernelError::Conflict("content capture is closed".into()));
    }
    if source.terminal_gate.is_some() {
        return Err(KernelError::Busy);
    }
    let counters = &mut source.head.capture_loss;
    let packets = counters
        .packets
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| invalid("capture loss packet counter overflow"))?;
    let total_bytes = counters
        .bytes
        .checked_add(bytes)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| invalid("capture loss byte counter overflow"))?;
    let mut end = diagnostic.len().min(256);
    while !diagnostic.is_char_boundary(end) {
        end -= 1;
    }
    let diagnostic = if diagnostic.is_empty() {
        "external capture input was discarded"
    } else {
        &diagnostic[..end]
    };
    let position = source.head.accepted;
    if let Some(window) = source
        .losses
        .back_mut()
        .filter(|window| window.after_position == position)
    {
        window.last_packet = packets;
        window.bytes += bytes;
    } else {
        // At this bound admission is fenced, keeping the accepted position
        // fixed so all later reports coalesce into the final window.
        if source.losses.len() >= MAX_PENDING_LOSS_WINDOWS {
            return Err(KernelError::Busy);
        }
        source.losses.push_back(CaptureLoss {
            first_packet: packets,
            last_packet: packets,
            after_position: position,
            bytes,
            diagnostic: diagnostic.into(),
        });
    }
    counters.bytes = total_bytes;
    counters.packets = packets;
    source
        .head
        .capture_error
        .get_or_insert_with(|| diagnostic.into());
    source.dirty_since.get_or_insert_with(Instant::now);
    source.sender.send_replace(source.head.clone());
    backend.live.2.notify_one();
    Ok(())
}

pub(super) async fn persist(
    connection: &mut SqliteConnection,
    source: &StagedSource,
    current: &ContentStreamView,
) -> Result<()> {
    let mut packets = current.capture_loss.committed_packets;
    let mut bytes = current.capture_loss.committed_bytes;
    for window in &source.losses {
        if window.first_packet != packets + 1
            || window.last_packet < window.first_packet
            || window.after_position > source.head.committed
        {
            return Err(storage(
                "capture loss windows are not contiguous within their committed head",
            ));
        }
        bytes = bytes
            .checked_add(window.bytes)
            .ok_or_else(|| storage("capture loss byte overflow"))?;
        packets = window.last_packet;
        sqlx::query("INSERT INTO agena_content_capture_losses (content_id,first_packet,last_packet,after_position,byte_count,diagnostic) VALUES (?,?,?,?,?,?)")
            .bind(source.head.id.to_string()).bind(window.first_packet as i64).bind(window.last_packet as i64).bind(window.after_position as i64).bind(window.bytes as i64).bind(&window.diagnostic)
            .execute(&mut *connection).await.map_err(sql_error)?;
    }
    if bytes != source.head.capture_loss.committed_bytes
        || packets != source.head.capture_loss.committed_packets
    {
        return Err(storage(
            "capture loss summary does not match its immutable windows",
        ));
    }
    Ok(())
}

pub(crate) async fn records(
    connection: &mut SqliteConnection,
    content: ContentId,
    after: u64,
    through: u64,
    limit: usize,
) -> Result<Vec<CaptureLoss>> {
    let rows = sqlx::query("SELECT first_packet,last_packet,after_position,byte_count,diagnostic FROM agena_content_capture_losses WHERE content_id=? AND first_packet>? AND last_packet<=? ORDER BY first_packet LIMIT ?")
        .bind(content.to_string()).bind(after as i64).bind(through as i64).bind(limit.min(257) as i64).fetch_all(connection).await.map_err(sql_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(CaptureLoss {
                first_packet: row.try_get::<i64, _>("first_packet").map_err(storage)? as u64,
                last_packet: row.try_get::<i64, _>("last_packet").map_err(storage)? as u64,
                after_position: row.try_get::<i64, _>("after_position").map_err(storage)? as u64,
                bytes: row.try_get::<i64, _>("byte_count").map_err(storage)? as u64,
                diagnostic: row.try_get("diagnostic").map_err(storage)?,
            })
        })
        .collect()
}

pub(in crate::kernel) async fn page(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    after: Option<CaptureLossCursor>,
    limit: usize,
) -> Result<CaptureLossPage> {
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut tx = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    authorize(&mut tx, session, content).await?;
    let head = stored_head(&mut tx, content).await?;
    let packet = after.map_or(0, |cursor| cursor.packet);
    if after.is_some_and(|cursor| cursor.content != content || cursor.epoch != head.epoch)
        || packet > head.capture_loss.committed_packets
    {
        return Err(KernelError::ContentReset);
    }
    if packet != 0 {
        let boundary: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_content_capture_losses WHERE content_id=? AND last_packet=?)")
            .bind(content.to_string()).bind(packet as i64).fetch_one(&mut *tx).await.map_err(sql_error)?;
        if !boundary {
            return Err(invalid("capture loss cursor is not a window boundary"));
        }
    }
    let limit = limit.clamp(1, 256);
    let mut losses = records(
        &mut tx,
        content,
        packet,
        head.capture_loss.committed_packets,
        limit + 1,
    )
    .await?;
    let has_more = losses.len() > limit;
    losses.truncate(limit);
    let next = CaptureLossCursor {
        content,
        epoch: head.epoch,
        packet: losses.last().map_or(packet, |window| window.last_packet),
    };
    tx.rollback().await.map_err(storage)?;
    Ok(CaptureLossPage {
        stream: head,
        losses,
        next,
        has_more,
    })
}
