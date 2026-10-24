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
