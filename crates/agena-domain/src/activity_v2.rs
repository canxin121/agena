//! Activity v2 —— 彻底重构后的领域核心（设计：07-comprehensive-redesign.md v3.1 + 08-plugin-contract.md）。
//!
//! 本模块承载重构的三大支柱：
//! - [`RawOutput`]：**单一事实源**。activity 唯一持久化的内容（payload/text/attachments/metadata）。
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
/// - `payload`：机器可读事实（模型投影主源；也是渲染函数输入）；
/// - `text`：文本事实（模型 fallback；也是渲染函数输入）；
/// - `attachments`/`metadata`：附随事实。
///
/// 不存在“人类副本 / 模型副本”：`for_model`/`render_human` 都是它的即时投影。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub truncated: bool,
}

impl RawOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.payload.is_none()
            && self.text.is_empty()
            && self.attachments.is_empty()
            && self.metadata.is_empty()
            && !self.truncated
    }

    /// 旧数据宽松读取：老 compact payload 形状（`{ "payload": … }`）可直接进入。
    pub fn from_legacy_json(value: Option<&serde_json::Value>) -> Self {
        match value {
            None | Some(serde_json::Value::Null) => Self::default(),
            Some(value) => match serde_json::from_value::<RawOutput>(value.clone()) {
                Ok(output) => output,
                Err(_) => Self {
                    payload: Some(value.clone()),
                    ..Self::default()
                },
            },
        }
    }
}

/// 统一视图：所有 activity 类型只读已存字段/事实，绝不从数据推导视图（Golden Invariant I1）。
///
/// 实现方（Operation/Resource/… 的 v2 活动类型）只需返回已持久化的标签与单一事实源；
/// 人类视图与模型视图由投影器/渲染函数生成，本 trait 不承担推导。
pub trait ActivityView {
    /// 已持久化标题（标签列）。
    fn title(&self) -> &str;
    /// 已持久化摘要（标签列）。
    fn summary(&self) -> &str;
    /// 单一事实源（可能为空：如仅标签的 Notice）。
    fn raw_output(&self) -> Option<&RawOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn view_block_serde_roundtrips_all_variants() {
        let blocks = vec![
            ViewBlock::Text {
                id: None,
                text: "hi".into(),
            },
            ViewBlock::Markdown {
                id: Some("m".into()),
                text: "# t".into(),
            },
            ViewBlock::Json {
                id: None,
                value: json!({ "a": 1 }),
            },
            ViewBlock::Table {
                id: None,
                columns: vec!["name".into()],
                rows: vec![vec![json!("x")]],
            },
            ViewBlock::log("out", CommandOutputStream::Stdout, "hello\n"),
            ViewBlock::Command {
                id: None,
                command: "cargo test".into(),
                cwd: None,
                exit_code: Some(0),
                stdout: "ok".into(),
                stderr: String::new(),
            },
            ViewBlock::FileChanges {
                id: None,
                changes: vec![FileChangeRecord {
                    path: "a.rs".into(),
                    kind: crate::FileChangeKind::Updated,
                    from_path: None,
                }],
            },
            ViewBlock::Diff {
                id: None,
                diff: "-a\n+b".into(),
                language: Some("diff".into()),
            },
            ViewBlock::SearchResults {
                id: None,
                items: vec![WebSearchResult {
                    title: "t".into(),
                    url: "https://e".into(),
                    snippet: None,
                }],
                total: Some(1),
            },
            ViewBlock::Media {
                id: None,
                artifact: ArtifactRef {
                    uri: "file:///a".into(),
                    mime: "text/plain".into(),
                    name: None,
                    size_bytes: None,
                    sha256: None,
                },
            },
            ViewBlock::Custom {
                id: None,
                kind: "k".into(),
                schema: json!({}),
                presentation: BTreeMap::new(),
            },
        ];
        for block in blocks {
            let encoded = serde_json::to_string(&block).unwrap();
            let decoded: ViewBlock = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, block);
        }
    }

    #[test]
    fn view_block_skips_absent_id_in_json() {
        let block = ViewBlock::Text {
            id: None,
            text: "hi".into(),
        };
        let encoded = serde_json::to_string(&block).unwrap();
        assert!(
            !encoded.contains("id"),
            "absent id must be skipped: {encoded}"
        );
    }

    #[test]
    fn render_delta_modes_and_ids() {
        let new = RenderDelta::new(ViewBlock::log("out", CommandOutputStream::Stdout, "a"));
        assert_eq!(new.mode, DeltaMode::New);
        assert_eq!(new.block_id.as_deref(), Some("out"));

        let append = RenderDelta::append(
            "out",
            ViewBlock::log("out", CommandOutputStream::Stdout, "b"),
        );
        assert_eq!(append.mode, DeltaMode::Append);
        assert_eq!(append.block_id.as_deref(), Some("out"));

        let replace = RenderDelta::replace(
            "out",
            ViewBlock::Text {
                id: Some("out".into()),
                text: "z".into(),
            },
        );
        assert_eq!(replace.mode, DeltaMode::Replace);
        assert_eq!(replace.block_id.as_deref(), Some("out"));

        let roundtrip: RenderDelta =
            serde_json::from_str(&serde_json::to_string(&append).unwrap()).unwrap();
        assert_eq!(roundtrip, append);
    }

    #[test]
    fn raw_output_serde_and_legacy_fallback() {
        let output = RawOutput {
            payload: Some(json!({ "exit_code": 0 })),
            text: "ok".into(),
            ..RawOutput::default()
        };
        let encoded = serde_json::to_string(&output).unwrap();
        let decoded: RawOutput = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, output);

        // 旧 compact payload（直接是机器事实）→ 宽松进入 payload 字段。
        let legacy = json!({ "result": { "count": 2 } });
        let from_legacy = RawOutput::from_legacy_json(Some(&legacy));
        assert_eq!(from_legacy.payload, Some(legacy));
        assert!(from_legacy.text.is_empty());
    }

    #[test]
    fn raw_output_empty_semantics() {
        assert!(RawOutput::default().is_empty());
        assert!(!RawOutput::text("x").is_empty());
    }
}
