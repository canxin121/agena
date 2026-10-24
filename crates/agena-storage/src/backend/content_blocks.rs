//! Immutable UTF-8 blocks for the session-kernel storage prototype.
//!
//! Physical boundaries depend on stream bytes and checkpoint policy, never on
//! producer packet boundaries. This is an infrastructure primitive, not a
//! writer capability: the kernel must validate ownership and attempt generation
//! before admitting bytes, then commit blocks and the durable head together.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::store::StoreError;

pub const DEFAULT_TEXT_BLOCK_BYTES: usize = 32 * 1024;
const MAX_TEXT_BLOCK_BYTES: usize = 1024 * 1024;

/// `start..end` is a byte range, with an exclusive end. A block is immutable
/// after construction; storage adapters can verify a loaded block before use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableTextBlock {
    start: u64,
    end: u64,
    bytes: Arc<[u8]>,
    digest: [u8; 32],
}

impl ImmutableTextBlock {
    pub fn from_stored(
        start: u64,
        end: u64,
        bytes: Vec<u8>,
        digest: [u8; 32],
    ) -> Result<Self, StoreError> {
        if bytes.is_empty()
            || bytes.len() > MAX_TEXT_BLOCK_BYTES
            || start.checked_add(bytes.len() as u64) != Some(end)
            || std::str::from_utf8(&bytes).is_err()
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != digest
        {
            return Err(StoreError::Constraint(
                "invalid text block range, UTF-8 or checksum".into(),
            ));
        }
        Ok(Self {
            start,
            end,
            bytes: bytes.into(),
            digest,
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

    /// Byte readers may request ranges inside a code point. Text readers build
    /// valid UTF-8 pages at their own boundary, without changing these bytes.
    pub fn range(&self, start: u64, end: u64) -> Result<&[u8], StoreError> {
        if start > end || start < self.start || end > self.end {
            return Err(StoreError::Constraint(
                "range is outside its content block".into(),
            ));
        }
        Ok(&self.bytes[(start - self.start) as usize..(end - self.start) as usize])
    }
}

/// One admission step consumes at most one block's remaining capacity. The
/// caller explicitly advances by `consumed_bytes`; large inputs need no second
/// large allocation or collection of all output blocks.
#[derive(Debug)]
pub struct TextBlockPush {
    pub consumed_bytes: usize,
    pub block: Option<ImmutableTextBlock>,
}

pub struct TextBlockAssembler {
    target_bytes: usize,
    emitted_head: u64,
    pending: String,
}

impl TextBlockAssembler {
    pub fn new(start: u64, target_bytes: usize) -> Result<Self, StoreError> {
        if !(4..=MAX_TEXT_BLOCK_BYTES).contains(&target_bytes) {
            return Err(StoreError::Constraint(
                "text block size must be 4 bytes to 1 MiB".into(),
            ));
        }
        Ok(Self {
            target_bytes,
            emitted_head: start,
            pending: String::with_capacity(target_bytes),
        })
    }

    pub fn pending_bytes(&self) -> usize {
        self.pending.len()
    }
    pub fn pending_text(&self) -> &str {
        &self.pending
    }
    /// This is an assembler position, not a durable commit receipt.
    pub fn accepted_head(&self) -> u64 {
        self.emitted_head + self.pending.len() as u64
    }

    pub fn push(&mut self, input: &str) -> Result<TextBlockPush, StoreError> {
        let mut take = input.len().min(self.target_bytes - self.pending.len());
        while !input.is_char_boundary(take) {
            take -= 1;
        }
        if self.accepted_head().checked_add(take as u64).is_none() {
            return Err(StoreError::Constraint(
                "text content position overflow".into(),
            ));
        }
        self.pending.push_str(&input[..take]);
        // Flush just before a code point which cannot fit in the remaining
        // space. If a packet ended here, the next push makes the same choice.
        let emit = self.pending.len() == self.target_bytes || take < input.len();
        Ok(TextBlockPush {
            consumed_bytes: take,
            block: if emit { self.checkpoint() } else { None },
        })
    }

    /// A timed checkpoint or finish seals only the pending suffix. The caller
    /// persists this block with the same transaction as its committed head.
    pub fn checkpoint(&mut self) -> Option<ImmutableTextBlock> {
        if self.pending.is_empty() {
            return None;
        }
        let bytes = std::mem::replace(&mut self.pending, String::with_capacity(self.target_bytes))
            .into_bytes();
        let start = self.emitted_head;
        self.emitted_head += bytes.len() as u64;
        let digest = Sha256::digest(&bytes).into();
        Some(ImmutableTextBlock {
            start,
            end: self.emitted_head,
            bytes: bytes.into(),
            digest,
        })
    }
}
