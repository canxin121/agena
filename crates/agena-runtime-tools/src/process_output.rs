//! Bounded, resumable output slices shared by pipes and PTYs.

use crate::MonitorError;
use agena_domain::ProcessEvent;

pub(crate) const DEFAULT_OUTPUT_BYTES: usize = 16 * 1024;

pub(crate) fn output_budget(requested: Option<u32>) -> usize {
    requested
        .map_or(DEFAULT_OUTPUT_BYTES, |value| value as usize)
        .clamp(1024, DEFAULT_OUTPUT_BYTES)
}

/// Keep metadata, cursor and recovery notices outside the captured-text slice.
/// The complete result stays comfortably below the global history limit.
pub(crate) fn text_budget(requested: Option<u32>, screen: bool) -> usize {
    let budget = output_budget(requested);
    let content = budget.saturating_sub((budget / 4).clamp(256, 2048));
    if screen { content / 2 } else { content }
}

pub(crate) fn escaped_bytes(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            '\u{0}'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        })
        .sum()
}

pub(crate) fn prefix_end(text: &str, budget: usize) -> usize {
    let mut used = 0;
    let mut end = 0;
    for ch in text.chars() {
        let cost = escaped_bytes(ch.encode_utf8(&mut [0; 4]));
        if used + cost > budget {
            break;
        }
        used += cost;
        end += ch.len_utf8();
    }
    end
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OutputCursor {
    // Latest fully consumed event. A partial event is seq + 1.
    pub seq: u64,
    pub offset: u32,
}

pub(crate) struct OutputSlice {
    pub events: Vec<ProcessEvent>,
    pub output: String,
    pub cursor: OutputCursor,
    pub has_more: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum OutputSelection {
    Resumable,
    // Older readers only persist a sequence. Always finish the first event,
    // even when it exceeds the preferred budget, so their cursor progresses.
    WholeEvents,
}

pub(crate) fn validate_cursor(since: Option<u64>, offset: u32) -> Result<(), MonitorError> {
    if offset != 0 && since.is_none() {
        return Err(MonitorError::Invalid(
            "event_offset requires since_seq; omit both for unread output".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn select_events<'a>(
    events: impl Iterator<Item = &'a ProcessEvent>,
    mut cursor: OutputCursor,
    global_last: u64,
    limit: usize,
    byte_budget: usize,
    event_metadata: bool,
    explicit: bool,
    selection: OutputSelection,
) -> Result<OutputSlice, MonitorError> {
    let mut selected = Vec::new();
    let mut output = String::new();
    let mut used = 0;
    let mut lines = 0;
    let start_seq = cursor.seq;
    let mut events = events.filter(|event| event.seq > start_seq).peekable();
    if events.peek().is_none() {
        if cursor.offset != 0 && explicit {
            return Err(MonitorError::Invalid(
                "partially read event was evicted; inspect output_archive when available".into(),
            ));
        }
        cursor = OutputCursor {
            seq: global_last.max(cursor.seq),
            offset: 0,
        };
    }
    for event in events.take(limit) {
        let mut start = cursor.offset as usize;
        if start != 0 && event.seq != cursor.seq.saturating_add(1) {
            if explicit {
                return Err(MonitorError::Invalid("partially read event was evicted; use output_archive segments to recover older output".into()));
            }
            start = 0; // automatic cursor recovers to the oldest retained event
        }
        if start > event.line.len() || !event.line.is_char_boundary(start) {
            return Err(MonitorError::Invalid(
                "event_offset is outside an event or splits a UTF-8 character".into(),
            ));
        }
        let overhead = if event_metadata {
            let empty = ProcessEvent {
                seq: event.seq,
                stream: event.stream,
                ts_ms: event.ts_ms,
                chunk: event.chunk,
                line: String::new(),
                notification: event.notification.clone(),
                notification_seq: event.notification_seq,
            };
            serde_json::to_vec(&empty)
                .expect("process event serializes")
                .len()
                + 1
        } else {
            0
        };
        let available = byte_budget.saturating_sub(used + overhead);
        let fragment = &event.line[start..];
        let length = match selection {
            OutputSelection::Resumable => {
                let mut length = prefix_end(fragment, available);
                let remaining_lines = 1000_usize.saturating_sub(lines);
                if let Some((index, _)) =
                    fragment[..length].match_indices('\n').nth(remaining_lines)
                {
                    length = index;
                }
                length
            }
            OutputSelection::WholeEvents => {
                if !selected.is_empty()
                    && (escaped_bytes(fragment) > available
                        || lines + fragment.bytes().filter(|byte| *byte == b'\n').count() > 1000)
                {
                    break;
                }
                fragment.len()
            }
        };
        let end = start + length;
        if end == start && start < event.line.len() {
            break;
        }
        let mut piece = event.clone();
        piece.line = event.line[start..end].to_owned();
        used += overhead + escaped_bytes(&piece.line);
        lines += piece.line.bytes().filter(|byte| *byte == b'\n').count();
        output.push_str(&piece.line);
        selected.push(piece);
        if end == event.line.len() {
            cursor = OutputCursor {
                seq: event.seq,
                offset: 0,
            };
        } else {
            cursor = OutputCursor {
                seq: event.seq - 1,
                offset: end as u32,
            };
            break;
        }
    }
    Ok(OutputSlice {
        events: selected,
        output,
        has_more: global_last > cursor.seq || cursor.offset != 0,
        cursor,
    })
}
