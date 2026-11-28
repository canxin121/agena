//! In-process capabilities used by the DeepSeek workflow adapter.

use std::{future::Future, pin::Pin, sync::Arc};

use agena_domain::ActivityOutputRetention;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{AgentSpawnRequest, Result};
use crate::ToolInvokeOutput;

pub struct CordisWorkflowStart {
    pub run_id: Uuid,
    pub name: String,
    pub report_completion: bool,
    pub wake_on_completion: bool,
    pub max_consecutive_wakes: Option<usize>,
    pub inline_output_bytes: usize,
    pub max_result_chars: usize,
    pub output_retention: ActivityOutputRetention,
    /// In-process observer invoked once by the activity's supervisor.
    pub on_settled: Box<dyn FnOnce(CordisWorkflowSettlement) + Send + Sync>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CordisWorkflowStopReason {
    Completed,
    Cancelled,
    Error,
}

#[derive(Debug)]
pub struct CordisWorkflowSettlement {
    pub reason: CordisWorkflowStopReason,
    pub error: Option<String>,
    pub agents_started: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CordisWorkflowStarted {
    pub job_id: agena_domain::ActivityId,
    pub run_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CordisWorkflowResult {
    pub value: Value,
    pub agents_started: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CordisWorkflowJobRequest {
    List,
    Output {
        job_id: String,
        wait_ms: Option<u64>,
    },
    Kill {
        job_id: String,
        reason: Option<String>,
    },
}

#[async_trait::async_trait]
pub trait CordisWorkflowJobHost: Send + Sync {
    async fn spawn_child(&self, request: AgentSpawnRequest) -> Result<i64>;
    async fn wait_child(&self, child: i64, timeout_ms: u64) -> Result<ToolInvokeOutput>;
    fn progress(&self, text: &str);
}

pub type CordisWorkflowRunner = Box<
    dyn FnOnce(
            Arc<dyn CordisWorkflowJobHost>,
            CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<CordisWorkflowResult>> + Send>>
        + Send,
>;
