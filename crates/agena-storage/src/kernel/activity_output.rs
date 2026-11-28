//! Byte-addressed packet output for owner-scoped background activities.

use agena_domain::{ActivityId, ActivityOutputChunk};

use super::*;

pub use agena_domain::{ActivityOutputRetention, MAX_ACTIVITY_OUTPUT_BYTES};
pub const MAX_ACTIVITY_OUTPUT_CHUNKS: usize = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityOutputHead {
    pub earliest: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityOutputTerminal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityOutputPage {
    pub head: ActivityOutputHead,
    pub chunks: Vec<ActivityOutputChunk>,
    pub terminal: Option<ActivityOutputTerminal>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spill_paths: Vec<String>,
}

impl ActivityHandle {
    /// Publish ordered packets together with the producer's observed byte
    /// position. Gaps between packets represent bytes lost before admission.
    pub async fn append_output_chunks(
        &self,
        request: RequestId,
        total: u64,
        chunks: &[ActivityOutputChunk],
    ) -> Result<CommitReceipt<ActivityOutputHead>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::AppendActivityOutput {
                    scope: self.scope,
                    total,
                    chunks: chunks.to_vec(),
                },
            )
            .await?;
        match receipt.value {
            Mutation::ActivityOutput(head) => Ok(CommitReceipt {
                position: receipt.position,
                value: head,
            }),
            _ => Err(unexpected()),
        }
    }

    /// Replace the full-stream file references independently of packet retention.
    pub async fn set_output_spill_paths(
        &self,
        request: RequestId,
        paths: &[String],
    ) -> Result<CommitReceipt<ActivityOutputHead>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::SetActivityOutputSpillPaths {
                    scope: self.scope,
                    paths: paths.to_vec(),
                },
            )
            .await?;
        match receipt.value {
            Mutation::ActivityOutput(head) => Ok(CommitReceipt {
                position: receipt.position,
                value: head,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl KernelReader {
    /// None for activities recorded without a packet stream. With no offset
    /// this reads the stream head and metadata.
    pub async fn activity_output(
        &self,
        session: SessionId,
        activity: ActivityId,
        from: Option<u64>,
    ) -> Result<ReadView<Option<ActivityOutputPage>>> {
        self.0
            .backend
            .activity_output(session, activity, from)
            .await
    }
}

impl SessionCommands {
    /// A direct activity reader acknowledges once before returning output to
    /// its caller. Model tool results can acknowledge in their own commit.
    pub async fn acknowledge_activity_output(
        &self,
        request: RequestId,
        session: SessionId,
        read: ActivityOutputRead,
        expected: ActivityOutputPosition,
    ) -> Result<CommitReceipt<bool>> {
        let receipt = self
            .0
            .command(
                request,
                Command::AcknowledgeActivityOutput {
                    session,
                    read,
                    expected,
                },
            )
            .await?;
        match receipt.value {
            Mutation::ActivityOutputAcknowledged(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}
