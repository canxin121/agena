# 03 — 创建/更新/删除机制、代码组织与问题清单

> **历史研究归档：2026-08-07，基线 `acaeaf76`。** 本文保留当时的调查和设计推演；文中的路径、类型、接口与结论属于该基线，不能直接作为当前实现的契约。2026-10-07 整合时保留原文及分支历史，未将这些旧设计直接应用到运行代码。
>
> 当前代码入口：[Activity runtime](../../crates/agena-runtime/src/activity/mod.rs)、[Session store](../../crates/agena-storage/src/store/mod.rs)、[Plugin tool contracts](../../crates/agena-tool/src/lib.rs)。整合记录见 [本地分支整合记录](../research/local-branch-integration-2026-10-07.md)。

## 1. 创建机制

- 运行时：`MessagePart::new` 对 `PartContent::Activity` 自动分配 `ActivityId`（`runtime-contracts/message/part/message_part.rs:62`）；`build_message` 保留 id。
- 持久化：`project_part_content` 对 `agena_content_nodes` 按 `node_id` upsert（`ON CONFLICT(node_id) DO UPDATE`），守卫 `owner_kind/owner_id/position` 且 `excluded.revision_seq >= current.revision_seq`。
- 稳定 id：`ActivityId::for_reply_error(reply_id)` 把 reply uuid 翻转一字节——live 重试节点与 durable 错误节点共享同一 id，保证“先出现 live 节点、失败后刷成 durable 错误节点”不重复。
- 乐观输入：TUI `ComposerActivity`（TextArtifact/Resource/SkillReference）→ `ComposerDocument::into_turn_input` → 直接成 `ActivityNode`（owner=TurnInput, actor=User, Completed）。

## 2. 更新机制

- 同 id upsert + revision 守卫（幂等、按序收敛）。
- `update_part_title` / `refresh_streaming_title`：两列 O(1) 更新 title，不重写 payload（`store/history.rs:305`，`replies_execution.rs:2779`）。
- InProgress 立即 checkpoint：串行工具权限通过后、并行批执行前（`replies_execution.rs:1334-1370, 1848-1869`），配合 `conversation_identity_for_message`（`store/history.rs:20`）在 checkpoint 时反查 turn/reply owner，让 live patch 是增量 `TranscriptPatch::ContentUpserted` 而不是全量 `Refresh`。
- 流式 delta：`CommandOutputDelta` → 呈现流 `OperationDetailDelta`（仅内存，不持久化）。

## 3. 删除机制（重点：错误重试成功后的删除）

| 场景 | 机制 | 位置 |
| --- | --- | --- |
| **provider 重试成功** | 重试开始投影 live `Progress` 节点（id=`for_reply_error`，**不持久化**）；`ProviderRetryResolved{succeeded:true}` 投影 `TranscriptPatch::ContentRemoved` 删除该节点，回复上不再有任何错误痕迹 | `manager/history.rs:439-497` |
| provider 重试最终失败 | 不投影删除；`ExecutionFinished{Failed}` 持久化同一 id 的 durable `ErrorActivity`（upsert，替换 live 节点） | `store/mod.rs:791` |
| **回复恢复后**（`reply_waits_for_user` 续跑成功） | durable 错误节点**保留**（历史错误仍可见，设计如此）；`AssistantReplySnapshot.failure` 清 NULL | `store/mod.rs:874-889` |
| 工具失败后模型重试 | 模型发新 tool call → **新** operation（新 id），旧 failed operation 保留为历史 | tool_calls/replies 路径 |
| 会话删除 | `schema_invariants.rs` delete 触发器级联删除 `agena_content_nodes`（含 session 与 reply 的节点） | |
| 后台活动 | `dismiss` / `clear_finished`（`RuntimeActivityService`，REST `POST /activities/{id}/dismiss`、`/activities/clear-finished`） | |

**缺口**：transcript 活动没有面向用户的删除/编辑 API；`ContentRemoved` 只被重试成功路径使用；除了随会话删除外没有内容节点 GC/过期机制（对长期会话是存储增长点，但当前量级可接受）。

## 4. 代码组织与复用评估

### 优点（设计良好的部分）
1. **单一内容表** `agena_content_nodes`（v10/v11 统一 text+activity），写路径收敛在 `project_part_content`。
2. **复用良好**：`activity_payload`（Part→Payload 转换）被 store 投影和 live `transcript_part_patch` 共用；`derive_operation_markdown` 是唯一 human-markdown 派生入口，snapshot load / live patch / lazy detail 三处复用；`TranscriptSnapshot::apply/merge` 用 identity+revision 统一收敛，避免时间戳/位置猜测。
3. **增量优先**：`RuntimePresentationEventKind` 统一了增量 patch（TranscriptPatch / OperationDetailDelta / ActivityChanged）与 `Refresh` 降级路径。
4. **模型/用户边界清晰**：模型侧投影按 payload 类型白名单化，用户专属类型彻底不进 provider（详见 02 §3）。
5. **稳定 id + revision**：`for_reply_error`、`revision_seq` 守卫、owner 反查，使“live 先于 durable”成为可靠模式。

### 弱点
1. **三套并行 taxonomy 手工映射**：`RuntimeActivity`(8) / `ActivityPayload`(18) / `OperationBlock`(~20) 各自演进，新增一种内容要在 3 处加变体 + 映射 + 投影 + 渲染；缺少“变体覆盖”编译期或测试期守护（死变体 6-7 个就是结果）。
2. **未合并功能造成前后端断链**：human blocks 流式（`OperationBlock`→human）在 `worktree-activity-tool-streaming` 分支，master 只有 `HumanToolResultResource` 空占位；`docs/activity-streaming-refactor.md` 描述的是未合入 master 的实现。
3. **Web 端重复推导且不兼容**：`chatRenderModel.ts` 自己实现 title/summary 映射并引用不存在的 `model_output_text`（与 TUI 的 `activity_presentation` 是两份平行实现，漂移风险已验证存在）。
4. **大文件**：`activity.rs` 1609 行、`manager/history.rs` 1845 行、`history/store/mod.rs` 也很大——类型定义、投影、快照、错误处理混在一个文件。
5. **文档漂移**：`activity-and-model-visibility.md` 的 PartContent 术语是重构前（Operation 已并入 RuntimeActivity）；死变体/无写者（`Session` owner）在文档中未说明。
6. **错误表示三处重叠**（ErrorActivity / OperationActivity.error / reply.failure），生命周期策略不同但无集中文档。

## 5. 问题清单（按严重度）

### P1（会出错的缺陷）
1. Web `chatRenderModel.ts:102` 读 `payload.model_output_text`，字段不存在 → operation 模型输出在 Web 恒为空。
2. `ToolResultEnvelopeResource.human` / `HumanToolResultResource` 在 master 无生产者（占位），若 Web/API 消费者依赖它则拿不到数据。

### P2（架构债）
3. 死变体 7 个（SkillExecution / Checklist / Search / FileChanges / NestedTask / Custom / RuntimeActivity::Error 无生产端），要么补生产端要么删除。
4. `ActivityOwner::Session` 无运行时写者。
5. 三套 taxonomy 手工映射无守护测试（建议加“每个 ActivityPayload 变体至少有一个端到端生产测试”的枚举覆盖测试）。
6. 插件无向 transcript 发自定义活动的通道（`Custom` 空转；插件只有 `BackgroundActivity` 通道）。

### P3（一致性/可维护性）
7. 文档与代码漂移（streaming 分支未合并、visibility 文档术语过时）。
8. 无 transcript 活动删除 API / GC。
9. 大文件拆分（可选）。

## 6. 建议（按优先级）

1. **合并 `worktree-activity-tool-streaming`**（人类块流式），并接通 `HumanToolResultResource` 的生产端；这是 02 §3.3 与 P1-2 的根修。
2. **修复 Web `model_output_text`**：改为读 `markdown`/`data`，或让快照投影给 operation 增加 model_output 字段（与 TUI 对齐）。
3. **清理死变体**：删除或实现 SkillExecution/Checklist/Search/FileChanges/NestedTask/Custom；明确 `RuntimeActivity::Error` 的去留（建议删，错误已由 operation/reply 承担）。
4. **决定 `ActivityOwner::Session` 的去留**：要么加写者（如系统级 notice、会话级摘要），要么删掉存储/渲染分支。
5. **加枚举覆盖测试**：每个 `ActivityPayload` 变体至少一个“生产→持久化→快照→渲染”测试，防止新的死变体。
6. **更新文档**：把 `activity-streaming-refactor.md` 与 master 对齐；改写 `activity-and-model-visibility.md` 的术语；补充错误三表示的生命周期说明。
7. 视产品需要决定是否提供 transcript 活动删除 API（目前 durable 节点只随会话删除）。
