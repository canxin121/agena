//! In-process handoff from a Cordis JobSpec to the session's activity runtime.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::AtomicU64},
};

use agena_domain::{ActivityOutputChunk, ActivityOutputRetention};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::{PluginCommandInvocation, Result};

pub enum CordisJobInvocation {
    Tool(i64),
    Command(PluginCommandInvocation),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CordisJobSpec {
    pub kind: String,
    pub label: String,
    pub output_limit_bytes: Option<usize>,
}

pub struct CordisJobStart {
    pub spec: CordisJobSpec,
    pub max_concurrent_per_owner: usize,
    pub report_completion: bool,
    pub wake_on_completion: bool,
    pub max_consecutive_wakes: Option<usize>,
    pub inline_output_bytes: usize,
    pub output_retention: ActivityOutputRetention,
}

pub enum CordisJobUpdate {
    Append {
        chunk: ActivityOutputChunk,
        _admission: OwnedSemaphorePermit,
    },
    Progress {
        line: String,
    },
    Flush(oneshot::Sender<Result<()>>),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CordisJobStatus {
    Completed,
    Failed,
    Killed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CordisJobOutcome {
    pub status: CordisJobStatus,
    pub detail: Option<String>,
    pub result: Option<String>,
}

pub enum CordisJobControl {
    Cancel(CordisJobCancellation),
    Teardown,
    Abandon,
}

pub struct CordisJobCancellation {
    pub reason: Option<String>,
    pub begin: oneshot::Sender<CordisJobCancelReply>,
}

pub struct CordisJobCancelReply {
    /// Confirms that the registry is still waiting for this callback.
    pub ready: oneshot::Sender<()>,
    /// False if the producer already released the job.
    pub result: oneshot::Receiver<std::result::Result<bool, String>>,
    /// The producer can resume its microtasks after the registry commits.
    pub applied: oneshot::Sender<()>,
}

pub type CordisJobControlSender = Arc<dyn Fn(CordisJobControl) + Send + Sync>;

pub struct CordisJobProducer {
    pub output: mpsc::Receiver<CordisJobUpdate>,
    pub output_end: Arc<AtomicU64>,
    pub spill_paths: watch::Receiver<Arc<[String]>>,
    pub outcome: oneshot::Receiver<CordisJobOutcome>,
    pub control: CordisJobControlSender,
}

/// The runtime invokes the starter only after verifying the owner and reserving
/// capacity. A rejected starter leaves no durable activity behind.
pub type CordisJobStarter = Box<
    dyn FnOnce(String) -> Pin<Box<dyn Future<Output = Result<CordisJobProducer>> + Send>> + Send,
>;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CordisJobQuery {
    List,
    Get {
        id: String,
    },
    Read {
        id: String,
    },
    ReadAt {
        id: String,
        from: u64,
    },
    Kill {
        id: String,
        reason: Option<String>,
    },
    Remove {
        id: String,
    },
    Wait {
        id: String,
        timeout_ms: u64,
        pre_aborted: bool,
    },
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CordisJobEvent {
    Registered {
        job: Value,
    },
    Progress {
        job: Value,
    },
    Stopping {
        job: Value,
    },
    Settled {
        job: Value,
        cause: CordisJobSettlementCause,
        awaited: bool,
    },
    Removed {
        job: Value,
    },
    Output {
        id: String,
        owner: String,
        total: u64,
    },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CordisJobSettlementCause {
    Producer,
    Kill,
    Teardown,
}

/// Job output capability for one plugin in one session. It does not grant the
/// creating invocation's process, network or tool permissions.
#[async_trait::async_trait]
pub trait CordisJobRegistry: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<Arc<CordisJobEvent>>;
    async fn query(&self, request: CordisJobQuery, cancel: CancellationToken) -> Result<Value>;
}
