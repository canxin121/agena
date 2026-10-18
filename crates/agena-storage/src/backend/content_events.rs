//! Versioned, checksummed event blocks. Physical envelopes depend on encoded
//! bytes/checkpoints; channel, timestamp and mutation order remain lossless.
//! No content identity, server epoch or arbitrary JSON envelope is repeated
//! for each log packet. The kernel supplies and validates source ownership.

use std::sync::Arc;

use agena_domain::CommandOutputStream;
use sha2::{Digest, Sha256};

use crate::kernel::{EventKind, EventPayload, EventRecord, KernelError, Result};

pub const DEFAULT_EVENT_BLOCK_BYTES: usize = 32 * 1024;
pub const MAX_EVENT_PAYLOAD_BYTES: usize = agena_domain::MAX_TERMINAL_FRAME_BYTES;
const MAX_EVENTS_PER_BLOCK: u64 = 4096;
const MAGIC: &[u8; 4] = b"AGE1";
const FRAME_HEADER: usize = 13;
const MAX_EVENT_BLOCK_BYTES: usize = MAX_EVENT_PAYLOAD_BYTES + FRAME_HEADER + MAGIC.len();

fn invalid(message: impl Into<String>) -> KernelError {
    KernelError::Invalid(message.into())
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_EVENT_PAYLOAD_BYTES - self.0.len() {
                return Err(std::io::Error::other("event payload exceeds 16 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value).map_err(|e| invalid(e.to_string()))?;
    Ok(writer.0)
}

/// Fully encoded and size-checked before admission mutates any source state.
pub struct EncodedEvent {
    tag: u8,
    timestamp_ms: i64,
    payload: Vec<u8>,
}

impl EncodedEvent {
    pub fn new(timestamp_ms: i64, payload: &EventPayload) -> Result<Self> {
        payload.validate()?;
        let (tag, bytes) = match payload {
            EventPayload::Log { channel, text } => (
                match channel {
                    CommandOutputStream::Stdout => 1,
                    CommandOutputStream::Stderr => 2,
                },
                {
                    if text.len() > MAX_EVENT_PAYLOAD_BYTES {
                        return Err(invalid("log event exceeds 16 MiB"));
                    }
                    text.as_bytes().to_vec()
                },
            ),
            EventPayload::Mixed { input, checkpoint } => (7, encode_json(&(input, checkpoint))?),
            EventPayload::DocumentCheckpoint { document } => (3, encode_json(document)?),
            EventPayload::DocumentPatch { base, mutation } => (4, encode_json(&(base, mutation))?),
            EventPayload::TerminalCheckpoint { screen } => (5, encode_json(screen)?),
            EventPayload::TerminalPatch {
                base,
                screen,
                rows_changed,
            } => (6, encode_json(&(base, screen, rows_changed))?),
        };
        if bytes.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(invalid("event payload exceeds 16 MiB"));
        }
        Ok(Self {
            tag,
            timestamp_ms,
            payload: bytes,
        })
    }
    pub fn encoded_bytes(&self) -> usize {
        self.payload.len() + FRAME_HEADER
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableEventBlock {
    start: u64,
    end: u64,
    bytes: Arc<[u8]>,
    digest: [u8; 32],
    offsets: Arc<[u32]>,
    kinds: u8,
}

impl ImmutableEventBlock {
    pub fn from_stored(start: u64, end: u64, bytes: Vec<u8>, digest: [u8; 32]) -> Result<Self> {
        if end <= start
            || end > i64::MAX as u64
            || end - start > MAX_EVENTS_PER_BLOCK
            || bytes.len() > MAX_EVENT_BLOCK_BYTES
            || !bytes.starts_with(MAGIC)
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != digest
        {
            return Err(invalid(
                "invalid event block range, length, version or checksum",
            ));
        }
        let mut offsets = Vec::with_capacity((end - start) as usize);
        let mut offset = MAGIC.len();
        let mut kinds = 0;
        for _ in start..end {
            offsets.push(offset as u32);
            let (tag, _, payload, next) = frame(&bytes, offset)?;
            kinds |= kind_bit(tag)?;
            if matches!(tag, 1 | 2) {
                // Log validation needs UTF-8, not a separately allocated String
                // for every packet in a block that a small page may never use.
                std::str::from_utf8(payload).map_err(|error| invalid(error.to_string()))?;
            } else {
                decode_payload(tag, payload)?;
            }
            offset = next;
        }
        if offset != bytes.len() {
            return Err(invalid("event block record count/trailer mismatch"));
        }
        Ok(Self {
            start,
            end,
            bytes: bytes.into(),
            digest,
            offsets: offsets.into(),
            kinds,
        })
    }
    pub fn start(&self) -> u64 {
        self.start
    }
    pub fn end(&self) -> u64 {
        self.end
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    /// Encoded storage and the constant-time record index, without decoding
    /// records into separately owned payloads. Arc control blocks are included.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.bytes.len() + std::mem::size_of_val(&*self.offsets) + 128
    }
    pub fn validate_kind(&self, kind: EventKind) -> Result<()> {
        let expected = match kind {
            EventKind::Log => 1,
            EventKind::Structured => 2,
            EventKind::Terminal => 4,
            EventKind::Mixed => 8,
        };
        if self.kinds == expected {
            Ok(())
        } else {
            Err(invalid("event block kind does not match its source"))
        }
    }
    pub fn iter(&self) -> EventIter<'_> {
        self.iter_after(self.start)
    }
    /// Seek by native sequence in O(1), then decode only requested records.
    /// `after` at either side of this block naturally selects its suffix/EOF.
    pub fn iter_after(&self, after: u64) -> EventIter<'_> {
        let sequence = after.clamp(self.start, self.end);
        let offset = self
            .offsets
            .get((sequence - self.start) as usize)
            .map_or(self.bytes.len(), |offset| *offset as usize);
        EventIter {
            block: self,
            offset,
            sequence,
            failed: false,
        }
    }
}

pub struct EventIter<'a> {
    block: &'a ImmutableEventBlock,
    offset: usize,
    sequence: u64,
    failed: bool,
}

impl Iterator for EventIter<'_> {
    type Item = Result<EventRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        if self.sequence == self.block.end && self.offset == self.block.bytes.len() {
            return None;
        }
        let result = self.decode();
        if result.is_err() {
            self.failed = true;
        }
        Some(result)
    }
}

impl EventIter<'_> {
    fn decode(&mut self) -> Result<EventRecord> {
        if self.sequence >= self.block.end {
            return Err(invalid("event block record count/trailer mismatch"));
        }
        let (tag, timestamp_ms, bytes, next) = frame(&self.block.bytes, self.offset)?;
        #[cfg(test)]
        DECODED_RECORDS.with(|count| count.set(count.get() + 1));
        let payload = decode_payload(tag, bytes)?;
        self.offset = next;
        self.sequence += 1;
        Ok(EventRecord {
            sequence: self.sequence,
            timestamp_ms,
            payload,
        })
    }
}

fn kind_bit(tag: u8) -> Result<u8> {
    match tag {
        1 | 2 => Ok(1),
        3 | 4 => Ok(2),
        5 | 6 => Ok(4),
        7 => Ok(8),
        _ => Err(invalid("unknown event record codec tag")),
    }
}

fn frame(bytes: &[u8], offset: usize) -> Result<(u8, i64, &[u8], usize)> {
    let header = bytes
        .get(offset..offset.saturating_add(FRAME_HEADER))
        .ok_or_else(|| invalid("event block record count/trailer mismatch"))?;
    let length = u32::from_le_bytes(header[9..13].try_into().expect("frame header")) as usize;
    let start = offset + FRAME_HEADER;
    if length > MAX_EVENT_PAYLOAD_BYTES || length > bytes.len() - start {
        return Err(invalid("invalid event payload length"));
    }
    let timestamp = i64::from_le_bytes(header[1..9].try_into().expect("frame header"));
    Ok((
        header[0],
        timestamp,
        &bytes[start..start + length],
        start + length,
    ))
}

fn decode_payload(tag: u8, bytes: &[u8]) -> Result<EventPayload> {
    let payload = match tag {
        1 | 2 => EventPayload::Log {
            channel: if tag == 1 {
                CommandOutputStream::Stdout
            } else {
                CommandOutputStream::Stderr
            },
            text: std::str::from_utf8(bytes)
                .map_err(|e| invalid(e.to_string()))?
                .to_owned(),
        },
        3 => EventPayload::DocumentCheckpoint {
            document: serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?,
        },
        4 => {
            let (base, mutation) =
                serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
            EventPayload::DocumentPatch { base, mutation }
        }
        5 => EventPayload::TerminalCheckpoint {
            screen: serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?,
        },
        6 => {
            let (base, screen, rows_changed) =
                serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
            EventPayload::TerminalPatch {
                base,
                screen,
                rows_changed,
            }
        }
        7 => {
            let (input, checkpoint) =
                serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
            EventPayload::Mixed { input, checkpoint }
        }
        _ => return Err(invalid("unknown event record codec tag")),
    };
    payload.validate()?;
    Ok(payload)
}

#[cfg(test)]
thread_local! {
    static DECODED_RECORDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct EventBlockPush {
    pub before: Option<ImmutableEventBlock>,
    pub after: Option<ImmutableEventBlock>,
}

pub struct EventBlockAssembler {
    target: usize,
    emitted: u64,
    count: u64,
    bytes: Vec<u8>,
    offsets: Vec<u32>,
    kinds: u8,
}

impl EventBlockAssembler {
    pub fn new(start: u64, target: usize) -> Result<Self> {
        if !(64..=MAX_EVENT_BLOCK_BYTES).contains(&target) || start > i64::MAX as u64 {
            return Err(invalid("invalid event assembler range or target"));
        }
        let mut bytes = Vec::with_capacity(target);
        bytes.extend_from_slice(MAGIC);
        Ok(Self {
            target,
            emitted: start,
            count: 0,
            bytes,
            offsets: Vec::new(),
            kinds: 0,
        })
    }
    pub fn accepted_head(&self) -> u64 {
        self.emitted + self.count
    }
    pub fn pending_bytes(&self) -> usize {
        self.bytes.len() - MAGIC.len()
    }
    pub fn push(&mut self, event: EncodedEvent) -> Result<EventBlockPush> {
        if self.accepted_head() == i64::MAX as u64 {
            return Err(invalid("event position overflow"));
        }
        let kind = kind_bit(event.tag)?;
        let before = if self.count > 0
            && (self.bytes.len() + event.encoded_bytes() > self.target
                || self.count == MAX_EVENTS_PER_BLOCK)
        {
            self.checkpoint()
        } else {
            None
        };
        self.offsets.push(self.bytes.len() as u32);
        self.kinds |= kind;
        self.bytes.push(event.tag);
        self.bytes
            .extend_from_slice(&event.timestamp_ms.to_le_bytes());
        self.bytes
            .extend_from_slice(&(event.payload.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(&event.payload);
        self.count += 1;
        let after = if self.bytes.len() >= self.target {
            self.checkpoint()
        } else {
            None
        };
        Ok(EventBlockPush { before, after })
    }
    pub fn checkpoint(&mut self) -> Option<ImmutableEventBlock> {
        if self.count == 0 {
            return None;
        }
        let mut next = Vec::with_capacity(self.target);
        next.extend_from_slice(MAGIC);
        let bytes = std::mem::replace(&mut self.bytes, next);
        let start = self.emitted;
        self.emitted += self.count;
        self.count = 0;
        Some(ImmutableEventBlock {
            start,
            end: self.emitted,
            digest: Sha256::digest(&bytes).into(),
            bytes: bytes.into(),
            offsets: std::mem::take(&mut self.offsets).into(),
            kinds: std::mem::take(&mut self.kinds),
        })
    }

    /// Capture at most `limit` pending records after a native sequence without
    /// copying or hashing the rest of the live tail. Positions before the
    /// pending block select its prefix; positions at or beyond its end are EOF.
    pub fn snapshot_range(&self, after: u64, limit: usize) -> Option<ImmutableEventBlock> {
        let first = after.saturating_sub(self.emitted).min(self.count) as usize;
        let count = limit.min(self.offsets.len() - first);
        if count == 0 {
            return None;
        }
        let last = first + count;
        let from = self.offsets[first] as usize;
        let to = self
            .offsets
            .get(last)
            .map_or(self.bytes.len(), |offset| *offset as usize);
        let mut bytes = Vec::with_capacity(MAGIC.len() + to - from);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.bytes[from..to]);
        let mut offsets = Vec::with_capacity(count);
        let mut kinds = 0;
        for offset in &self.offsets[first..last] {
            offsets.push(*offset - from as u32 + MAGIC.len() as u32);
            kinds |= kind_bit(self.bytes[*offset as usize])
                .expect("assembler contains validated event tags");
        }
        let start = self.emitted + first as u64;
        Some(ImmutableEventBlock {
            start,
            end: start + count as u64,
            digest: Sha256::digest(&bytes).into(),
            bytes: bytes.into(),
            offsets: offsets.into(),
            kinds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{EventKind, EventProjection};
    use agena_domain::{
        ContentDocument, DocumentMutation, TerminalAttributes, TerminalCellRun, TerminalSnapshot,
        ViewBlock,
    };

    fn screen(text: &str) -> TerminalSnapshot {
        TerminalSnapshot {
            rows: 2,
            cols: 80,
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
            alternate_screen: false,
            bracketed_paste: true,
            application_cursor: false,
            cells: vec![
                TerminalCellRun {
                    row: 0,
                    col: 0,
                    text: text.into(),
                    attributes: TerminalAttributes::default(),
                },
                TerminalCellRun {
                    row: 1,
                    col: 0,
                    text: "unchanged row".into(),
                    attributes: TerminalAttributes::default(),
                },
            ],
        }
    }

    #[test]
    fn compact_log_blocks_preserve_bytes_channels_timestamps_and_order() {
        let mut assembler = EventBlockAssembler::new(0, DEFAULT_EVENT_BLOCK_BYTES).unwrap();
        let mut blocks = Vec::new();
        let mut expected = Vec::new();
        for sequence in 1..=8192 {
            let text = if sequence == 2048 {
                "界🙂 \r\n\0\u{1b}[31m".repeat(10000)
            } else {
                "界🙂 \r\n\0\u{1b}[31m".into()
            };
            let payload = EventPayload::Log {
                channel: if sequence % 2 == 0 {
                    CommandOutputStream::Stdout
                } else {
                    CommandOutputStream::Stderr
                },
                text,
            };
            let timestamp_ms = 100000 + sequence as i64 / 3;
            let push = assembler
                .push(EncodedEvent::new(timestamp_ms, &payload).unwrap())
                .unwrap();
            blocks.extend(push.before);
            blocks.extend(push.after);
            expected.push(EventRecord {
                sequence,
                timestamp_ms,
                payload,
            });
        }
        blocks.extend(assembler.checkpoint());
        assert!(blocks.len() < 32);
        let mut actual = Vec::new();
        for block in &blocks {
            let verified = ImmutableEventBlock::from_stored(
                block.start(),
                block.end(),
                block.bytes().to_vec(),
                *block.digest(),
            )
            .unwrap();
            actual.extend(verified.iter().map(Result::unwrap));
        }
        assert_eq!(actual, expected);
        assert_eq!(assembler.accepted_head(), 8192);
    }

    #[test]
    fn indexed_small_pages_decode_the_suffix_without_materializing_prior_log_records() {
        let start = 73;
        let mut assembler = EventBlockAssembler::new(start, 1024 * 1024).unwrap();
        let mut expected = Vec::new();
        for ordinal in 0..4096 {
            let payload = EventPayload::Log {
                channel: if ordinal % 2 == 0 {
                    CommandOutputStream::Stdout
                } else {
                    CommandOutputStream::Stderr
                },
                text: format!("{ordinal:04}: 界\0\r\n"),
            };
            let timestamp_ms = ordinal as i64 - 200;
            assembler
                .push(EncodedEvent::new(timestamp_ms, &payload).unwrap())
                .unwrap();
            expected.push(EventRecord {
                sequence: start + ordinal + 1,
                timestamp_ms,
                payload,
            });
        }
        let live = assembler.snapshot_range(0, usize::MAX).unwrap();
        let sealed = assembler.checkpoint().unwrap();
        assert_eq!(live, sealed);
        DECODED_RECORDS.with(|count| count.set(0));
        let stored = ImmutableEventBlock::from_stored(
            sealed.start(),
            sealed.end(),
            sealed.bytes().to_vec(),
            *sealed.digest(),
        )
        .unwrap();
        assert_eq!(DECODED_RECORDS.with(|count| count.get()), 0);
        let shared = stored.clone();
        assert!(Arc::ptr_eq(&shared.offsets, &stored.offsets));
        assert!(stored.retained_bytes() <= stored.bytes().len() + 4 * 4096 + 512);
        stored.validate_kind(EventKind::Log).unwrap();
        assert!(stored.validate_kind(EventKind::Terminal).is_err());

        let mut actual = Vec::new();
        let mut after = start;
        while after < stored.end() {
            let page = stored
                .iter_after(after)
                .take(32)
                .collect::<Result<Vec<_>>>()
                .unwrap();
            assert_eq!(page.len(), 32);
            after = page.last().unwrap().sequence;
            actual.extend(page);
        }
        assert_eq!(actual, expected);
        // This would exceed 260,000 decodes if every page skipped its prefix
        // by materializing records from the beginning of the block.
        assert_eq!(DECODED_RECORDS.with(|count| count.get()), 4096);
        assert!(stored.iter_after(u64::MAX).next().is_none());
        assert_eq!(stored.iter_after(0).next().unwrap().unwrap(), expected[0]);
        assert_eq!(
            stored
                .iter_after(start + 4000)
                .take(3)
                .collect::<Result<Vec<_>>>()
                .unwrap(),
            expected[4000..4003]
        );
        assert_eq!(DECODED_RECORDS.with(|count| count.get()), 4100);
    }

    #[test]
    fn live_snapshot_ranges_preserve_native_boundaries_and_complete_utf8_records() {
        let start = 73;
        let mut assembler = EventBlockAssembler::new(start, 1024 * 1024).unwrap();
        assert!(assembler.snapshot_range(0, usize::MAX).is_none());
        let mut expected = Vec::new();
        for ordinal in 0..8 {
            let payload = EventPayload::Log {
                channel: if ordinal % 2 == 0 {
                    CommandOutputStream::Stdout
                } else {
                    CommandOutputStream::Stderr
                },
                text: format!("{ordinal}: 界🙂e\u{301}\0\r\n"),
            };
            let timestamp_ms = ordinal as i64 - 4;
            assembler
                .push(EncodedEvent::new(timestamp_ms, &payload).unwrap())
                .unwrap();
            expected.push(EventRecord {
                sequence: start + ordinal + 1,
                timestamp_ms,
                payload,
            });
        }
        assert!(assembler.snapshot_range(0, 0).is_none());
        assert!(assembler.snapshot_range(start + 8, 1).is_none());
        assert!(assembler.snapshot_range(u64::MAX, usize::MAX).is_none());
        for (after, limit, first, last) in [
            (0, 2, 0, 2),
            (start, usize::MAX, 0, 8),
            (start + 3, 2, 3, 5),
            (start + 7, usize::MAX, 7, 8),
        ] {
            let block = assembler.snapshot_range(after, limit).unwrap();
            assert_eq!(block.start(), start + first as u64);
            assert_eq!(block.end(), start + last as u64);
            assert_eq!(block.offsets.len(), last - first);
            assert_eq!(block.offsets[0], MAGIC.len() as u32);
            let reopened = ImmutableEventBlock::from_stored(
                block.start(),
                block.end(),
                block.bytes().to_vec(),
                *block.digest(),
            )
            .unwrap();
            assert_eq!(
                reopened.iter().collect::<Result<Vec<_>>>().unwrap(),
                expected[first..last]
            );
            assert!(reopened.iter_after(block.end()).next().is_none());
        }

        let sealed = assembler.checkpoint().unwrap();
        assert!(assembler.snapshot_range(0, usize::MAX).is_none());
        let payload = EventPayload::Log {
            channel: CommandOutputStream::Stderr,
            text: "after checkpoint 界".into(),
        };
        assembler
            .push(EncodedEvent::new(99, &payload).unwrap())
            .unwrap();
        let block = assembler.snapshot_range(0, usize::MAX).unwrap();
        assert_eq!(block.start(), sealed.end());
        assert_eq!(block.end(), sealed.end() + 1);
        assert_eq!(block.iter().next().unwrap().unwrap().payload, payload);

        let maximum = i64::MAX as u64;
        let mut assembler = EventBlockAssembler::new(maximum - 1, 1024).unwrap();
        assembler
            .push(EncodedEvent::new(0, &payload).unwrap())
            .unwrap();
        let block = assembler.snapshot_range(0, usize::MAX).unwrap();
        assert_eq!((block.start(), block.end()), (maximum - 1, maximum));
        assert!(assembler.snapshot_range(maximum, usize::MAX).is_none());
    }

    #[test]
    fn live_snapshot_range_kind_comes_only_from_the_selected_frames() {
        let payloads = [
            EventPayload::Log {
                channel: CommandOutputStream::Stdout,
                text: "stdout".into(),
            },
            EventPayload::Log {
                channel: CommandOutputStream::Stderr,
                text: "stderr".into(),
            },
            EventPayload::DocumentCheckpoint {
                document: Box::new(ContentDocument::default()),
            },
            EventPayload::TerminalCheckpoint {
                screen: Box::new(screen("terminal 界")),
            },
            EventPayload::Mixed {
                input: Box::new(agena_domain::ContentInput::Text {
                    text: "mixed text".into(),
                }),
                checkpoint: None,
            },
        ];
        let mut assembler = EventBlockAssembler::new(20, 1024 * 1024).unwrap();
        for payload in &payloads {
            assembler
                .push(EncodedEvent::new(0, payload).unwrap())
                .unwrap();
        }
        for (ordinal, payload) in payloads.iter().enumerate() {
            let block = assembler.snapshot_range(20 + ordinal as u64, 1).unwrap();
            block.validate_kind(payload.kind()).unwrap();
            for kind in [
                EventKind::Log,
                EventKind::Structured,
                EventKind::Terminal,
                EventKind::Mixed,
            ] {
                assert_eq!(block.validate_kind(kind).is_ok(), kind == payload.kind());
            }
            assert_eq!(&block.iter().next().unwrap().unwrap().payload, payload);
        }
        assembler
            .snapshot_range(20, 2)
            .unwrap()
            .validate_kind(EventKind::Log)
            .unwrap();
        let mixed = assembler.snapshot_range(21, 3).unwrap();
        for kind in [
            EventKind::Log,
            EventKind::Structured,
            EventKind::Terminal,
            EventKind::Mixed,
        ] {
            assert!(mixed.validate_kind(kind).is_err());
        }
    }

    #[test]
    fn live_snapshot_range_copies_only_the_requested_frame_bytes_and_seek_index() {
        let mut assembler = EventBlockAssembler::new(0, 1024 * 1024).unwrap();
        let mut expected = Vec::new();
        let mut selected_bytes = MAGIC.len();
        for ordinal in 0..4096 {
            let payload = EventPayload::Log {
                channel: CommandOutputStream::Stdout,
                text: format!("{ordinal:04} 界🙂\0\r\n").repeat(8),
            };
            let timestamp_ms = ordinal as i64;
            let encoded = EncodedEvent::new(timestamp_ms, &payload).unwrap();
            if (4000..4032).contains(&ordinal) {
                selected_bytes += encoded.encoded_bytes();
                expected.push(EventRecord {
                    sequence: ordinal + 1,
                    timestamp_ms,
                    payload,
                });
            }
            let pushed = assembler.push(encoded).unwrap();
            assert!(pushed.before.is_none() && pushed.after.is_none());
        }
        DECODED_RECORDS.with(|count| count.set(0));
        let block = assembler.snapshot_range(4000, 32).unwrap();
        assert_eq!(DECODED_RECORDS.with(|count| count.get()), 0);
        assert_eq!((block.start(), block.end()), (4000, 4032));
        assert_eq!(block.bytes().len(), selected_bytes);
        assert!(block.bytes().len() < assembler.pending_bytes() / 100);
        assert_eq!(block.offsets.len(), 32);
        assert!(block.retained_bytes() <= selected_bytes + 4 * 32 + 512);
        let reopened = ImmutableEventBlock::from_stored(
            block.start(),
            block.end(),
            block.bytes().to_vec(),
            *block.digest(),
        )
        .unwrap();
        assert_eq!(
            reopened.iter().collect::<Result<Vec<_>>>().unwrap(),
            expected
        );
        assert_eq!(DECODED_RECORDS.with(|count| count.get()), 32);
        assert_eq!(
            reopened
                .iter_after(4020)
                .take(2)
                .collect::<Result<Vec<_>>>()
                .unwrap(),
            expected[20..22]
        );
    }

    #[test]
    fn indexed_blocks_validate_unread_log_utf8_and_the_whole_source_kind() {
        let mut assembler = EventBlockAssembler::new(0, 1024).unwrap();
        for _ in 0..2 {
            assembler
                .push(
                    EncodedEvent::new(
                        0,
                        &EventPayload::Log {
                            channel: CommandOutputStream::Stdout,
                            text: "valid".into(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let block = assembler.checkpoint().unwrap();
        let mut corrupt = block.bytes().to_vec();
        corrupt[block.offsets[1] as usize + FRAME_HEADER] = 0xff;
        let digest = Sha256::digest(&corrupt).into();
        assert!(ImmutableEventBlock::from_stored(0, 2, corrupt, digest).is_err());

        let mut assembler = EventBlockAssembler::new(0, 1024).unwrap();
        assembler
            .push(
                EncodedEvent::new(
                    0,
                    &EventPayload::Log {
                        channel: CommandOutputStream::Stdout,
                        text: "log".into(),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        assembler
            .push(
                EncodedEvent::new(
                    0,
                    &EventPayload::DocumentCheckpoint {
                        document: Box::new(ContentDocument::default()),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let block = assembler.checkpoint().unwrap();
        let stored = ImmutableEventBlock::from_stored(
            block.start(),
            block.end(),
            block.bytes().to_vec(),
            *block.digest(),
        )
        .unwrap();
        assert!(stored.validate_kind(EventKind::Log).is_err());
        assert!(stored.validate_kind(EventKind::Structured).is_err());
    }

    #[test]
    fn document_and_terminal_replay_require_exact_bases_and_valid_checkpoints() {
        let checkpoint = EventRecord {
            sequence: 9,
            timestamp_ms: 1,
            payload: EventPayload::DocumentCheckpoint {
                document: Box::new(ContentDocument::default()),
            },
        };
        let mut document = EventProjection::from_checkpoint(&checkpoint).unwrap();
        let mutation = EventRecord {
            sequence: 10,
            timestamp_ms: 2,
            payload: EventPayload::DocumentPatch {
                base: 8,
                mutation: Box::new(DocumentMutation::Insert {
                    after: None,
                    block: ViewBlock::Text {
                        id: Some("body".into()),
                        text: "  世界\n\0".into(),
                    },
                }),
            },
        };
        assert!(document.apply(&mutation).is_err());
        assert_eq!(document.sequence(), 9);
        let mut valid = mutation;
        if let EventPayload::DocumentPatch { base, .. } = &mut valid.payload {
            *base = 9;
        }
        document.apply(&valid).unwrap();
        assert_eq!(document.document().unwrap().blocks.len(), 1);
        assert!(EventProjection::from_checkpoint(&valid).is_err());
        assert!(
            EventProjection::new(EventKind::Structured)
                .apply(&valid)
                .is_err()
        );

        let base = EventRecord {
            sequence: 1,
            timestamp_ms: 1,
            payload: EventPayload::TerminalCheckpoint {
                screen: Box::new(screen("before")),
            },
        };
        let mut terminal = EventProjection::from_checkpoint(&base).unwrap();
        let mut next = screen("after界");
        next.cells.truncate(1);
        next.cursor_row = 1;
        let patch = EventRecord {
            sequence: 2,
            timestamp_ms: 2,
            payload: EventPayload::TerminalPatch {
                base: 1,
                screen: Box::new(next.clone()),
                rows_changed: vec![0],
            },
        };
        let mut no_base = EventProjection::new(EventKind::Terminal);
        assert!(no_base.apply(&patch).is_err());
        next.cols = 81;
        let wrong_geometry = EventRecord {
            sequence: 2,
            timestamp_ms: 2,
            payload: EventPayload::TerminalPatch {
                base: 1,
                screen: Box::new(next),
                rows_changed: vec![0],
            },
        };
        assert!(terminal.apply(&wrong_geometry).is_err());
        assert_eq!(terminal.terminal().unwrap().cells[0].text, "before");
        terminal.apply(&patch).unwrap();
        let restored = terminal.terminal().unwrap();
        assert_eq!(restored.cursor_row, 1);
        assert_eq!(restored.cells[0].text, "after界");
        assert_eq!(restored.cells[1].text, "unchanged row");
        let mut assembler = EventBlockAssembler::new(0, 256).unwrap();
        let mut blocks = Vec::new();
        for record in [&base, &patch] {
            let push = assembler
                .push(EncodedEvent::new(record.timestamp_ms, &record.payload).unwrap())
                .unwrap();
            blocks.extend(push.before);
            blocks.extend(push.after);
        }
        blocks.extend(assembler.checkpoint());
        let mut replay = EventProjection::new(EventKind::Terminal);
        for block in blocks {
            for record in block.iter() {
                replay.apply(&record.unwrap()).unwrap();
            }
        }
        assert_eq!(replay.terminal(), terminal.terminal());
    }

    #[test]
    fn corrupt_lengths_counts_tags_checksums_and_overflow_are_rejected() {
        let payload = EventPayload::Log {
            channel: CommandOutputStream::Stdout,
            text: "exact\0界\r\n".into(),
        };
        let mut assembler = EventBlockAssembler::new(0, 64).unwrap();
        assembler
            .push(EncodedEvent::new(-1, &payload).unwrap())
            .unwrap();
        let block = assembler.checkpoint().unwrap();
        assert!(
            ImmutableEventBlock::from_stored(0, 2, block.bytes().to_vec(), *block.digest())
                .is_err()
        );
        let mut corrupt = block.bytes().to_vec();
        corrupt[4] = 255;
        assert!(ImmutableEventBlock::from_stored(0, 1, corrupt.clone(), *block.digest()).is_err());
        let digest = Sha256::digest(&corrupt).into();
        assert!(ImmutableEventBlock::from_stored(0, 1, corrupt, digest).is_err());
        let mut corrupt = block.bytes().to_vec();
        corrupt[13..17].copy_from_slice(&u32::MAX.to_le_bytes());
        let digest = Sha256::digest(&corrupt).into();
        assert!(ImmutableEventBlock::from_stored(0, 1, corrupt, digest).is_err());
        let mut overflow = EventBlockAssembler::new(i64::MAX as u64, 64).unwrap();
        assert!(
            overflow
                .push(EncodedEvent::new(0, &payload).unwrap())
                .is_err()
        );
        assert_eq!(overflow.pending_bytes(), 0);
    }
}
