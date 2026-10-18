//! Capture loss is an observation about discarded external input, not a hole
//! in the sequence of canonical accepted bytes/events. Windows identify where
//! that input was observed relative to the accepted stream.

use super::*;
use agena_domain::ContentEpoch;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureLossCounters {
    pub bytes: u64,
    pub packets: u64,
    pub committed_bytes: u64,
    pub committed_packets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureLoss {
    pub first_packet: u64,
    pub last_packet: u64,
    /// Byte offset for text, event sequence for other sources. The discarded
    /// external packet follows this accepted position; it never consumes a
    /// fabricated canonical cursor. Several packets may share one window.
    pub after_position: u64,
    pub bytes: u64,
    /// First diagnostic in this window, bounded to 256 UTF-8 bytes.
    pub diagnostic: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureLossCursor {
    pub content: ContentId,
    pub epoch: ContentEpoch,
    pub packet: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureLossPage {
    /// Durable scope only. Live watch counters may be ahead; a subsequent
    /// checkpoint notification makes those windows available on this port.
    pub stream: ContentStreamView,
    pub losses: Vec<CaptureLoss>,
    pub next: CaptureLossCursor,
    pub has_more: bool,
}

impl KernelReader {
    pub async fn capture_losses(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<CaptureLossCursor>,
        limit: usize,
    ) -> Result<ReadView<CaptureLossPage>> {
        self.0
            .backend
            .capture_losses(session, content, after, limit)
            .await
    }
}
