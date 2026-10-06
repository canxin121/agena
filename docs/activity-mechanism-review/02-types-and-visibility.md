# 02 — 类型体系与 AI/用户可见性分离

> **历史研究归档：2026-08-07，基线 `acaeaf76`。** 本文保留当时的调查和设计推演；文中的路径、类型、接口与结论属于该基线，不能直接作为当前实现的契约。2026-10-07 整合时保留原文及分支历史，未将这些旧设计直接应用到运行代码。
>
> 当前代码入口：[Activity runtime](../../crates/agena-runtime/src/activity/mod.rs)、[Session store](../../crates/agena-storage/src/store/mod.rs)、[Plugin tool contracts](../../crates/agena-tool/src/lib.rs)。整合记录见 [本地分支整合记录](../research/local-branch-integration-2026-10-07.md)。

## 1. 三套并行 taxonomy（同一逻辑活动在不同边界的表示）

| 层 | 类型 | 变体数 | 用途 |
| --- | --- | --- | --- |
| 运行时 Part（`agena-runtime-contracts`） | `PartContent::Activity(RuntimeActivity)` | 8 | 内存/事件/持久化的消息 part；`RuntimeActivity = Reasoning, Operation, Resource, SkillReference, Interaction, Hook, Error, Notice` |
| 领域内容（`agena-domain`） | `ContentNode` / `ActivityNode` / `ActivityPayload` | 18 | transcript 文档模型（可持久化、可渲染、可 patch） |
| 人类结果块（`agena-runtime-contracts`） | `OperationBlock` | ~20 | 工具结果的人类可读结构化块（Command/Diff/FileChanges/SearchResults/Checklist/NestedTask/...） |
| 后台活动（`agena-domain`） | `BackgroundActivity`（kind: shell/task/runtime/browser） | 4 kinds | 与 transcript 无关的后台工作管理 |

`ActivityPayload` 18 个变体：`Resource, SkillReference, SkillExecution, TextArtifact, Reasoning, TextSegment, Operation, Interaction, Progress, Checklist, Search, FileChanges, NestedTask, Maintenance, Hook, Error, Custom, Notice`。

映射关系（手工）：`RuntimeActivity` → `ActivityPayload`（`session/history/store/mod.rs:1169 activity_payload`，共 7 个分支 + Text→TextSegment/TextArtifact）；`ToolResultEnvelope.content` → `OperationBlock`（`manager/helpers.rs operation_blocks_from_tool_output`）。

## 2. 死变体（有定义 + TUI 渲染，但生产代码没有任何构造点）

用 `grep -rn 'X {'` 跨 crates 非测试代码验证：

| 变体 | 生产点 | 消费点 | 结论 |
| --- | --- | --- | --- |
| `SkillExecution` | 无（仅 domain 定义/导出） | TUI `snapshot.rs:381,451`、`message_render.rs:387` | 死变体 |
| `Checklist` | 无 | TUI `snapshot.rs:520`、`message_render.rs:429` | 死变体（人类块有 `OperationBlock::Checklist`，但 payload 层没人产出） |
| `Search` | 无 | TUI `snapshot.rs:526`、`message_render.rs:459` | 死变体 |
| `FileChanges` | 无 | TUI `snapshot.rs:532`、`message_render.rs:472` | 死变体 |
| `NestedTask` | 无 | TUI `snapshot.rs:538`、`message_render.rs:485` | 死变体 |
| `Custom` | 仅测试（`agena-tui-transcript/src/snapshot.rs:1207`） | TUI `snapshot.rs` | 死变体（插件自定义 transcript 活动的通道没实现；插件只能发 `BackgroundActivity`） |
| `Progress` | 仅重试 live 节点（`manager/history.rs:456`，不持久化） | TUI `snapshot.rs` | 半死变体：只在内存中出现 |
| `RuntimeActivity::Error` | 无（`ErrorPart` 全仓库无构造点） | 投影/渲染有防御性匹配 | 死变体：运行时的错误走 `OperationActivity.error` 或 reply 级 `ErrorActivity` |

另：`ActivityOwner::Session` 有存储 schema（`schema_invariants.rs`）、读取（`transcript_snapshot`）、TUI 渲染（`session_activities`），但**运行时没有任何写者**（唯一构造在测试里）。

## 3. AI 服务器 vs 用户呈现的分离机制（总体成熟）

### 3.1 模型侧投影（`agena-runtime-provider/src/provider/wire_message.rs:104`）

`project_persisted` 对 `PartContent::Activity` 按变体处理：

| RuntimeActivity | 模型侧 | 说明 |
| --- | --- | --- |
| `Text` | `WirePart::Text` | |
| `Resource` | `WirePart::Attachment` | 附件进模型 |
| `SkillReference` | `WirePart::Text`（`model_context_text()` 包装的 `<agena_skill_reference>` JSON） | 模型可见但被结构包装，防止伪造 |
| `Operation` | `WirePart::ToolCall` + `WirePart::ToolResult` | `project_operation_output` 输出模型可读文本/结构化结果 |
| `Reasoning` | `WirePart::Reasoning` | |
| `Interaction / Error / Hook / Notice` | **丢弃** | 纯用户侧，绝不进 provider |

`normalize_prompt_messages`（`prompt_window.rs:266`）+ `message_has_visible_prompt_payload` 进一步把“没有可见 payload 的消息”整个剔除（token 估算、digest、父消息计算同样基于投影后消息）。这符合 `docs/activity-and-model-visibility.md` 的不变式（注意：文档的“PartContent::Activity vs PartContent::Operation”术语是重构前的，现在 Operation 已在 RuntimeActivity 内，行为不变但措辞过时）。

### 3.2 工具结果的双轨表示（同一 operation 的两份内容）

- `ToolResultEnvelope.model_preview`（`ModelVisibleOutput{text,attachments,truncated}`）→ 模型读的扁平文本。
- `ToolResultEnvelope.content`（`OperationBlock` 人类块）、`display{title,summary,sections}`、`managed_outputs` → 用户读的。
- `OperationActivity.data`（`details.to_json_payload()` 压缩 ToolResult）→ **唯一持久化的工具结果**；`OperationActivity.markdown`（人类可读详情）→ **派生、从不持久化**，在 snapshot load / live patch / lazy detail 三处由 `derive_operation_markdown` 生成。
- 惰性详情接口：`GET /api/v1/sessions/{session_id}/operations/{activity_id}/detail` → `OperationDetailResource{activity_id, markdown, streaming}`。
- TUI 渲染去重启发式 `should_render_tool_model_output`（`transcript_text.rs:137`）：human markdown/blocks 与 model_output 相同则不重复渲染。
- `ToolResultEnvelope.failed()` 分别构造 `model_preview`（`model_visible_failure_text`）与人类 `summary`，错误文本两轨不同。

### 3.3 已知问题（分离机制上的洞）

1. **Web 端引用不存在的字段**：`packages/agena-web-ui/src/agena/pages/chatRenderModel.ts:102` 读 `activity.payload.model_output_text`，该字段在 `ActivityPayload::Operation` 上不存在（领域只有 `data/markdown/summary/title`）。→ Web 上 operation 的模型输出恒为空。TUI 无此问题（读 `markdown`/`data`）。
2. **API 占位未接通**：`agena-api/src/message_part.rs:451` 的 `ToolResultEnvelopeResource.human`（`HumanToolResultResource`）在 master 上没有任何生产者（`grep 'human:'` 无赋值点）；真正的 `OperationBlock`→human 流式实现只在未合并分支 `worktree-activity-tool-streaming`（16 文件 + 813 行，见 `docs/activity-streaming-refactor.md`）。
3. **三处错误表示策略不同**：`ActivityPayload::Error`（reply 级持久错误，恢复后保留）、`OperationActivity.error`（工具失败，模型重试会新建 operation）、`AssistantReplySnapshot.failure`（回复失败投影，`reply_waits_for_user` 恢复时清 NULL）——语义重叠但生命周期不同，无文档说明。

## 4. 类型完善度小结

- **够用**：核心场景（工具、推理、附件、skill、交互、hook、notice、错误、压缩、进度、正文分段）都有类型。
- **过时/冗余**：6-7 个死变体 + `Session` owner 无写者，说明类型是“先定义后实现”，部分场景后来被 `OperationBlock` 人类块方案取代（checklist/search/file_changes/nested_task 现在以 block 形式出现，payload 变体成了遗留）。
- **缺口**：插件没有向 transcript 发自定义活动的通道（`Custom` 无生产端）；`RuntimeActivity::Error` 无生产端（错误统一由 operation/reply 承担，这是合理简化，但应删掉死变体）。
