//! Activity rendering and model/human projection domain core.
//!
//! 本模块承载重构的三大支柱：
//! - [`RawOutput`]：**单一事实源**。activity 唯一持久化的内容（payload/text/attachments/managed_outputs）。
//!   给 AI 看与给人看都是它的即时投影/渲染，视图永不落盘。
//! - [`ViewBlock`]：**统一渲染契约**。工具渲染函数与实时渲染增量的输出，TUI/Web 共享同一类型。
//! - [`RenderDelta`]/[`DeltaMode`]：**实时渲染增量**。流式期间“给人看的实时更新”的最小单元。
//!
//! 原则：视图是纯函数投影（`for_model`/`for_human`），绝不另存副本；
//! 渲染职责下沉给工具（`render_human`），运行时只做调度与 fallback。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ArtifactRef, CommandOutputStream, FileChangeRecord, WebSearchResult};

/// 实时渲染增量的更新模式。
///
/// - [`DeltaMode::New`]：新增一块卡片；
/// - [`DeltaMode::Append`]：追加到同 `block_id` 的块（Log 文本追加、Markdown 拼接）；
/// - [`DeltaMode::Replace`]：整块替换（Table/Json 更新、Command 完成态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DeltaMode {
    #[default]
    New,
    Append,
    Replace,
}

/// 视图描述块：工具 `render_human` 与流式 `RenderDelta` 共用的渲染契约。
///
/// `id` 是客户端合并的稳定块标识：同 id 的 `Append`/`Replace` 更新同一块，
/// 新 id（或 `None`）新增一块。TUI 与 Web 共用这一个类型渲染。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewBlock {
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        text: String,
    },
    Markdown {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        text: String,
    },
    Json {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        value: serde_json::Value,
    },
    Table {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        columns: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    },
    Log {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        stream: CommandOutputStream,
        text: String,
    },
    Command {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stdout: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stderr: String,
    },
    FileChanges {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        changes: Vec<FileChangeRecord>,
    },
    Diff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        diff: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    SearchResults {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        items: Vec<WebSearchResult>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<u64>,
    },
    Media {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        artifact: ArtifactRef,
    },
    Content {
        id: String,
        resource: crate::ContentRef,
        #[serde(default)]
        format: crate::ContentFormat,
    },
    Progress {
        id: String,
        phase: String,
        completed: u64,
        total: Option<u64>,
        unit: Option<String>,
    },
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        kind: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        schema: serde_json::Value,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        presentation: BTreeMap<String, String>,
    },
}

impl ViewBlock {
    /// 稳定块标识（客户端合并键）。
    pub fn block_id(&self) -> Option<&str> {
        match self {
            Self::Text { id, .. }
            | Self::Markdown { id, .. }
            | Self::Json { id, .. }
            | Self::Table { id, .. }
            | Self::Log { id, .. }
            | Self::Command { id, .. }
            | Self::FileChanges { id, .. }
            | Self::Diff { id, .. }
            | Self::SearchResults { id, .. }
            | Self::Media { id, .. }
            | Self::Custom { id, .. } => id.as_deref(),
            Self::Content { id, .. } | Self::Progress { id, .. } => Some(id),
        }
    }

    /// 纯文本值（Text/Markdown/Log/Diff 的正文），对应旧 `OperationBlock::text_value`。
    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. }
            | Self::Markdown { text, .. }
            | Self::Log { text, .. }
            | Self::Diff { diff: text, .. } => Some(text.as_str()),
            _ => None,
        }
    }

    /// 便捷：带 id 的 Log 块。
    pub fn log(
        id: impl Into<String>,
        stream: CommandOutputStream,
        text: impl Into<String>,
    ) -> Self {
        Self::Log {
            id: Some(id.into()),
            stream,
            text: text.into(),
        }
    }
}

/// 实时渲染增量：工具流式推送“给人看”的最小单元（07 §5.1 / 08 §2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderDelta {
    /// 稳定块 id；`None` = 新块。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default)]
    pub mode: DeltaMode,
    pub view: ViewBlock,
}

impl RenderDelta {
    /// 新块。
    pub fn new(view: ViewBlock) -> Self {
        Self {
            block_id: view.block_id().map(ToOwned::to_owned),
            mode: DeltaMode::New,
            view,
        }
    }

    /// 追加到已有块。
    pub fn append(block_id: impl Into<String>, view: ViewBlock) -> Self {
        Self {
            block_id: Some(block_id.into()),
            mode: DeltaMode::Append,
            view,
        }
    }

    /// 整块替换。
    pub fn replace(block_id: impl Into<String>, view: ViewBlock) -> Self {
        Self {
            block_id: Some(block_id.into()),
            mode: DeltaMode::Replace,
            view,
        }
    }
}

/// 单一事实源：activity 唯一持久化的内容（07 §4.2 / 08 §2）。
///
/// - `payload`：工具定义的唯一原始结果，包括正文、退出码、分页等结果信息；
/// - `attachments`/`managed_outputs`：结果资源。
///
/// 不存在“人类副本 / 模型副本”：`for_model`/`render_human` 都是它的即时投影。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<crate::ContentField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::AttachmentItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_outputs: Vec<crate::ToolManagedOutput>,
    #[serde(default)]
    pub truncated: bool,
}

impl RawOutput {
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            payload: (!text.is_empty()).then(|| serde_json::json!({"text": text})),
            ..Self::default()
        }
    }

    pub fn text_content(&self) -> &str {
        self.payload
            .as_ref()
            .and_then(|payload| {
                payload
                    .as_str()
                    .or_else(|| payload.get("text").and_then(serde_json::Value::as_str))
            })
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.payload.is_none()
            && self.fields.is_empty()
            && self.attachments.is_empty()
            && self.managed_outputs.is_empty()
            && !self.truncated
    }

    /// A structured tool result is authoritative; formatted previews are not
    /// another durable result. Text-only tools keep their text in the payload.
    pub fn from_parts(
        payload: Option<serde_json::Value>,
        text: impl Into<String>,
        attachments: Vec<crate::AttachmentItem>,
        managed_outputs: Vec<crate::ToolManagedOutput>,
        truncated: bool,
    ) -> Self {
        let payload = payload.or_else(|| Self::text(text).payload);
        Self {
            payload,
            fields: Vec::new(),
            attachments,
            managed_outputs,
            truncated,
        }
    }
}

/// 统一视图：所有 activity 类型只读已存字段/事实，绝不从数据推导视图（Golden Invariant I1）。
///
/// 实现方（Operation/Resource/… 活动类型）只需返回已持久化的标签与单一事实源；
/// 人类视图与模型视图由投影器/渲染函数生成，本 trait 不承担推导。
pub trait ActivityView {
    /// 已持久化标题（标签列）。
    fn title(&self) -> &str;
    /// 已持久化摘要（标签列）。
    fn summary(&self) -> &str;
    /// 单一事实源（可能为空：如仅标签的 Notice）。
    fn raw_output(&self) -> Option<&RawOutput>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Human presentation of one part's durable facts. It is kind-agnostic:
/// every part kind may carry one, and clients read it through the part-level
/// `presentation` field instead of probing `content`.
pub struct PartDocument {
    pub title: String,
    pub summary: String,
    pub blocks: Vec<ViewBlock>,
}
