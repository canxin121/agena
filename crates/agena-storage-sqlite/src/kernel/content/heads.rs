use std::sync::LazyLock;

use sqlx::sqlite::SqliteRow;

use super::*;

pub(in crate::kernel) const COLUMNS: &str =
    "head_epoch,committed_head,retained_start,capture_state,
    lost_bytes,lost_packets,committed_lost_bytes,committed_lost_packets,capture_error,head_bytes";

// Both read shapes use c for the stream identity and h for its frozen state.
pub(in crate::kernel) const READ_COLUMNS: &str = "c.id,c.entry_id,c.kind,c.activity_id,c.ordinal,
    h.head_epoch AS epoch,h.committed_head,h.retained_start,h.capture_state,h.capture_error,
    h.lost_bytes,h.lost_packets,h.committed_lost_bytes,h.committed_lost_packets";

fn wire_json(head: &str, epoch: &str, committed_loss: &str) -> String {
    format!(
        "json_object(
            'id',c.id,'entry',c.entry_id,'epoch',{head}.{epoch},'kind',c.kind,
            'activity_id',c.activity_id,'ordinal',c.ordinal,
            'capture_loss',json_object(
                'bytes',{head}.lost_bytes,'packets',{head}.lost_packets,
                'committed_bytes',{head}.{committed_loss}lost_bytes,
                'committed_packets',{head}.{committed_loss}lost_packets
            ),
            'accepted',{head}.committed_head,'committed',{head}.committed_head,
            'retained_start',{head}.retained_start,'state',{head}.capture_state,
            'capture_error',{head}.capture_error
        )"
    )
}

pub(in crate::kernel) static WIRE_JSON: LazyLock<String> =
    LazyLock::new(|| wire_json("h", "head_epoch", "committed_"));

pub(in crate::kernel) static LIVE_HEAD_BYTES: LazyLock<String> =
    LazyLock::new(|| format!("length(CAST({} AS BLOB))", wire_json("c", "epoch", "")));

// A nested capture inherits the earlier cut, including an absent diagnostic.
pub(in crate::kernel) static CAPTURE_VALUES: LazyLock<String> = LazyLock::new(|| {
    let live_head_bytes = &*LIVE_HEAD_BYTES;
    format!(
        "COALESCE(h.head_epoch,c.epoch),COALESCE(h.committed_head,c.committed_head),
         COALESCE(h.retained_start,c.retained_start),COALESCE(h.capture_state,c.capture_state),
         COALESCE(h.lost_bytes,c.lost_bytes),COALESCE(h.lost_packets,c.lost_packets),
         COALESCE(h.committed_lost_bytes,c.lost_bytes),COALESCE(h.committed_lost_packets,c.lost_packets),
         CASE WHEN h.source_key IS NULL THEN c.capture_error ELSE h.capture_error END,
         COALESCE(h.head_bytes,{live_head_bytes})"
    )
});

pub(in crate::kernel) fn read(content: ContentId, row: &SqliteRow) -> Result<ContentStreamView> {
    let count = |column| {
        let value: i64 = row.try_get(column).map_err(storage)?;
        u64::try_from(value).map_err(storage)
    };
    super::head_row(
        content,
        row,
        CaptureLossCounters {
            bytes: count("lost_bytes")?,
            packets: count("lost_packets")?,
            committed_bytes: count("committed_lost_bytes")?,
            committed_packets: count("committed_lost_packets")?,
        },
    )
}
