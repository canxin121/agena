//! Deterministic, bounded fixtures shared by both experimental adapters.
//! Physical payloads use production codecs; this module grants no kernel port.

use std::sync::Arc;

use agena_domain::{
    CommandOutputStream, ContentDocument, DocumentMutation, TerminalAttributes, TerminalCellRun,
    TerminalColor, TerminalSnapshot, ViewBlock,
};
use agena_storage::{
    backend::{
        content_blocks::{ImmutableTextBlock, TextBlockAssembler},
        content_events::{EncodedEvent, EventBlockAssembler, ImmutableEventBlock},
    },
    kernel::{EventKind, EventPayload, EventProjection, EventRecord},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::AnyResult;

pub const BLOCK_BYTES: usize = 32 * 1024;
pub const COMMIT_BYTES: usize = 256 * 1024;
pub const BLOB_CHUNK_BYTES: usize = 128 * 1024;
pub const TEXT_BYTES: usize = 8192 * 17;
pub const EVENT_PAGE: usize = 256;
pub const BYTE_PAGE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    TextSmall,
    TextLarge,
    Log,
    Structured,
    Terminal,
    Blob,
}

impl Workload {
    pub fn parse(value: &str) -> AnyResult<Self> {
        match value {
            "text-small" => Ok(Self::TextSmall),
            "text-large" => Ok(Self::TextLarge),
            "log" => Ok(Self::Log),
            "structured" => Ok(Self::Structured),
            "terminal" => Ok(Self::Terminal),
            "blob" => Ok(Self::Blob),
            _ => Err("unknown comparison workload".into()),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::TextSmall => "text-small",
            Self::TextLarge => "text-large",
            Self::Log => "log",
            Self::Structured => "structured",
            Self::Terminal => "terminal",
            Self::Blob => "blob",
        }
    }
    pub fn event_kind(self) -> Option<EventKind> {
        match self {
            Self::Log => Some(EventKind::Log),
            Self::Structured => Some(EventKind::Structured),
            Self::Terminal => Some(EventKind::Terminal),
            _ => None,
        }
    }
    pub fn page_units(self) -> usize {
        match self {
            Self::Blob => BLOB_CHUNK_BYTES,
            _ if self.event_kind().is_some() => EVENT_PAGE,
            _ => BYTE_PAGE,
        }
    }
}

#[derive(Clone)]
pub struct Spec {
    pub workload: Workload,
    pub smoke: bool,
}

impl Spec {
    pub fn steps(&self) -> usize {
        match (self.workload, self.smoke) {
            (Workload::TextSmall, _) => 8192,
            (Workload::TextLarge, _) => 4,
            (Workload::Log, false) => 65_536,
            (Workload::Structured, false) => 1024,
            (Workload::Terminal, false) => 2048,
            (Workload::Blob, false) => 64,
            (Workload::Log, true) => 256,
            (Workload::Structured | Workload::Terminal, true) => 64,
            (Workload::Blob, true) => 4,
        }
    }
    pub fn packet_bytes(&self) -> usize {
        match self.workload {
            Workload::TextSmall | Workload::Log => 17,
            Workload::TextLarge => TEXT_BYTES / 4,
            Workload::Blob => BLOB_CHUNK_BYTES,
            _ => 0,
        }
    }
    pub fn head(&self) -> u64 {
        if self.workload.event_kind().is_some() {
            self.steps() as u64
        } else {
            (self.steps() * self.packet_bytes()) as u64
        }
    }
    pub fn bytes(&self, source: usize, offset: u64, length: usize) -> Vec<u8> {
        (offset..offset + length as u64)
            .map(|index| {
                let value = mix(index ^ (source as u64 + 1).wrapping_mul(0x9e37_79b9));
                if self.workload == Workload::Blob {
                    value as u8
                } else {
                    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \n"
                        [value as usize % 64]
                }
            })
            .collect()
    }
    /// Random-access expected events avoid retaining the entire event stream.
    /// `base=index` always refers to the immediately preceding native sequence.
    pub fn record(&self, source: usize, index: u64) -> AnyResult<EventRecord> {
        let payload = match self.workload {
            Workload::Log => EventPayload::Log {
                channel: if index.is_multiple_of(3) {
                    CommandOutputStream::Stderr
                } else {
                    CommandOutputStream::Stdout
                },
                text: String::from_utf8(self.bytes(source, index * 17, 17))?,
            },
            Workload::Structured => {
                let block = ViewBlock::Text {
                    id: Some("body".into()),
                    text: ascii(1024, source as u64 ^ index.wrapping_mul(0xa076_1d65)),
                };
                if index.is_multiple_of(32) {
                    EventPayload::DocumentCheckpoint {
                        document: Box::new(ContentDocument {
                            blocks: vec![block],
                        }),
                    }
                } else {
                    EventPayload::DocumentPatch {
                        base: index,
                        mutation: Box::new(DocumentMutation::Replace { block }),
                    }
                }
            }
            Workload::Terminal if index.is_multiple_of(16) => EventPayload::TerminalCheckpoint {
                screen: Box::new(screen(source, index, None)),
            },
            Workload::Terminal => {
                let row = (index % 24) as u16;
                EventPayload::TerminalPatch {
                    base: index,
                    screen: Box::new(screen(source, index, Some(row))),
                    rows_changed: vec![row],
                }
            }
            _ => return Err("byte workload has no typed events".into()),
        };
        Ok(EventRecord {
            sequence: index + 1,
            timestamp_ms: 1_729_209_600_000 + source as i64 * 1_000_000 + index as i64 * 17,
            payload,
        })
    }
}

fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn ascii(length: usize, seed: u64) -> String {
    (0..length)
        .map(|index| {
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"
                [mix(seed ^ index as u64) as usize % 64] as char
        })
        .collect()
}

fn screen(source: usize, index: u64, only: Option<u16>) -> TerminalSnapshot {
    TerminalSnapshot {
        rows: 24,
        cols: 80,
        cursor_row: (index % 24) as u16,
        cursor_col: (index % 80) as u16,
        cursor_visible: !index.is_multiple_of(5),
        alternate_screen: true,
        bracketed_paste: true,
        application_cursor: !index.is_multiple_of(7),
        cells: (0..24u16)
            .filter(|row| only.is_none_or(|only| *row == only))
            .map(|row| {
                let last = if index >= u64::from(row) {
                    index - (index + 24 - u64::from(row)) % 24
                } else {
                    0x10000 + u64::from(row)
                };
                let prefix = format!("s{source:02}:r{row:02}:i{last:06} ");
                TerminalCellRun {
                    row,
                    col: 0,
                    text: format!("{prefix}{}", ascii(80 - prefix.len(), last ^ source as u64)),
                    attributes: TerminalAttributes {
                        foreground: Some(TerminalColor::Indexed {
                            index: (last % 16) as u8,
                        }),
                        bold: last.is_multiple_of(2),
                        underline: last.is_multiple_of(3),
                        ..TerminalAttributes::default()
                    },
                }
            })
            .collect(),
    }
}

#[derive(Clone)]
pub enum Block {
    Text(ImmutableTextBlock),
    Events(ImmutableEventBlock),
    Blob {
        start: u64,
        bytes: Arc<[u8]>,
        digest: [u8; 32],
    },
}

impl Block {
    pub fn start(&self) -> u64 {
        match self {
            Self::Text(b) => b.start(),
            Self::Events(b) => b.start(),
            Self::Blob { start, .. } => *start,
        }
    }
    pub fn end(&self) -> u64 {
        match self {
            Self::Text(b) => b.end(),
            Self::Events(b) => b.end(),
            Self::Blob { start, bytes, .. } => *start + bytes.len() as u64,
        }
    }
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(b) => b.bytes(),
            Self::Events(b) => b.bytes(),
            Self::Blob { bytes, .. } => bytes,
        }
    }
    pub fn digest(&self) -> &[u8; 32] {
        match self {
            Self::Text(b) => b.digest(),
            Self::Events(b) => b.digest(),
            Self::Blob { digest, .. } => digest,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        match self {
            Self::Events(b) => b.retained_bytes(),
            _ => self.bytes().len() + 256,
        }
    }
    pub fn from_stored(
        spec: &Spec,
        start: u64,
        end: u64,
        bytes: Vec<u8>,
        digest: [u8; 32],
    ) -> AnyResult<Self> {
        if let Some(kind) = spec.workload.event_kind() {
            let block = ImmutableEventBlock::from_stored(start, end, bytes, digest)?;
            block.validate_kind(kind)?;
            Ok(Self::Events(block))
        } else if spec.workload == Workload::Blob {
            if bytes.is_empty()
                || bytes.len() > BLOB_CHUNK_BYTES
                || start + bytes.len() as u64 != end
                || <[u8; 32]>::from(Sha256::digest(&bytes)) != digest
            {
                return Err("invalid experimental blob chunk".into());
            }
            Ok(Self::Blob {
                start,
                bytes: bytes.into(),
                digest,
            })
        } else {
            Ok(Self::Text(ImmutableTextBlock::from_stored(
                start, end, bytes, digest,
            )?))
        }
    }
}

#[derive(Clone)]
pub struct Item {
    pub source: usize,
    pub block: Block,
}

enum Assembler {
    Text(TextBlockAssembler),
    Events(EventBlockAssembler),
    Blob,
}

pub struct Generator {
    pub spec: Spec,
    pub source: usize,
    pub step: usize,
    pub hash: Sha256,
    pub projection: Option<EventProjection>,
    pub event_payload_bytes: usize,
    assembler: Assembler,
}

impl Generator {
    pub fn new(spec: Spec, source: usize) -> AnyResult<Self> {
        let kind = spec.workload.event_kind();
        let assembler = if kind.is_some() {
            Assembler::Events(EventBlockAssembler::new(0, BLOCK_BYTES)?)
        } else if spec.workload == Workload::Blob {
            Assembler::Blob
        } else {
            Assembler::Text(TextBlockAssembler::new(0, BLOCK_BYTES)?)
        };
        Ok(Self {
            spec,
            source,
            step: 0,
            hash: Sha256::new(),
            projection: kind.map(EventProjection::new),
            event_payload_bytes: 0,
            assembler,
        })
    }
    pub fn next_blocks(&mut self) -> AnyResult<Vec<Item>> {
        let mut blocks = Vec::new();
        match &mut self.assembler {
            Assembler::Events(assembler) => {
                let record = self.spec.record(self.source, self.step as u64)?;
                self.projection
                    .as_mut()
                    .ok_or("missing producer projection")?
                    .apply(&record)?;
                self.hash.update(serde_json::to_vec(&record)?);
                let encoded = EncodedEvent::new(record.timestamp_ms, &record.payload)?;
                self.event_payload_bytes += encoded.encoded_bytes() - 13;
                let pushed = assembler.push(encoded)?;
                blocks.extend(
                    pushed
                        .before
                        .into_iter()
                        .chain(pushed.after)
                        .map(Block::Events),
                );
            }
            Assembler::Text(assembler) => {
                let bytes = self.spec.bytes(
                    self.source,
                    (self.step * self.spec.packet_bytes()) as u64,
                    self.spec.packet_bytes(),
                );
                self.hash.update(&bytes);
                let mut text = std::str::from_utf8(&bytes)?;
                while !text.is_empty() {
                    let pushed = assembler.push(text)?;
                    text = &text[pushed.consumed_bytes..];
                    blocks.extend(pushed.block.map(Block::Text));
                }
            }
            Assembler::Blob => {
                let start = (self.step * BLOB_CHUNK_BYTES) as u64;
                let bytes = self.spec.bytes(self.source, start, BLOB_CHUNK_BYTES);
                self.hash.update(&bytes);
                blocks.push(Block::Blob {
                    start,
                    digest: Sha256::digest(&bytes).into(),
                    bytes: bytes.into(),
                });
            }
        }
        self.step += 1;
        Ok(blocks
            .into_iter()
            .map(|block| Item {
                source: self.source,
                block,
            })
            .collect())
    }
    pub fn finish(&mut self) -> Option<Item> {
        let block = match &mut self.assembler {
            Assembler::Text(a) => a.checkpoint().map(Block::Text),
            Assembler::Events(a) => a.checkpoint().map(Block::Events),
            Assembler::Blob => None,
        };
        block.map(|block| Item {
            source: self.source,
            block,
        })
    }
    pub fn summary(&self) -> AnyResult<Value> {
        let document = self
            .projection
            .as_ref()
            .and_then(EventProjection::document)
            .map(serde_json::to_vec)
            .transpose()?
            .map(|bytes| hex::encode(Sha256::digest(bytes)));
        let terminal = self
            .projection
            .as_ref()
            .and_then(EventProjection::terminal)
            .map(serde_json::to_vec)
            .transpose()?
            .map(|bytes| hex::encode(Sha256::digest(bytes)));
        Ok(
            json!({"source":self.source,"head":self.spec.head(),"sha256":hex::encode(self.hash.clone().finalize()),"document_sha256":document,"terminal_sha256":terminal}),
        )
    }
}

pub fn batch_digest(items: &[Item]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for item in items {
        hash.update((item.source as u64).to_le_bytes());
        hash.update(item.block.start().to_le_bytes());
        hash.update(item.block.end().to_le_bytes());
        hash.update((item.block.bytes().len() as u64).to_le_bytes());
        hash.update(item.block.digest());
    }
    hash.finalize().into()
}

pub fn fault_blocks(spec: &Spec) -> AnyResult<Vec<Item>> {
    // Always use the full generator: smoke Log has less than one 32 KiB block.
    let mut generator = Generator::new(
        Spec {
            workload: spec.workload,
            smoke: false,
        },
        0,
    )?;
    let mut items = Vec::new();
    while items.len() < 2 && generator.step < generator.spec.steps() {
        items.extend(generator.next_blocks()?);
    }
    if items.len() < 2 {
        items.extend(generator.finish());
    }
    if items.len() < 2 {
        return Err("fault fixture needs two blocks".into());
    }
    items.truncate(2);
    Ok(items)
}
