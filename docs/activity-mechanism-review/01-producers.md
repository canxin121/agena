# 01 — 哪些地方会产生 Activity

## 1. 运行时生产点（`agena-runtime-session`）

所有运行时 activity 都先成为 `MessagePart`（`PartContent::Activity`，构造时自动分配 `activity_id`），经 `SessionStore::persist` 的 checkpoint 落到 `agena_content_nodes` 单表。

| # | 生产点 | 文件:行 | Payload | 说明 |
| --- | --- | --- | --- | --- |
| 1 | 工具调用（流式 tool call） | `session/processor/tool_calls.rs:47,240,304,344,428` | Operation | tool call 一到就建 part（Pending），状态随执行推进 |
| 2 | 工具执行 InProgress 转场 | `session/manager/replies/replies_execution.rs:1334-1370, 1848-1869` | Operation | 串行：权限通过后 Pending→InProgress 并立即 checkpoint；并行：批量置 InProgress 一次 persist |
| 3 | 工具失败 | `session/manager/replies/tool_failure.rs:406` | Operation + `error` | `OperationActivityError{problem}` 挂在 operation 上 |
| 4 | 工具非执行 | `session/manager/replies/tool_non_execution.rs:50` | Operation | policy_denied / user_declined / capability_unavailable / tool_unavailable |
| 5 | prompt 窗口关闭回填 | `session/prompt_window.rs:1029` | Operation(completed) | 窗口内残留工具收尾 |
| 6 | 推理流 | `session/processor/parts.rs:41` | Reasoning | 流式 reasoning 段 |
| 7 | 附件/资源 | `session/manager/mod.rs:780` | Resource | 用户或系统携带的附件 |
| 8 | Skill 引用 | `session/manager/runs.rs:606` | SkillReference | 每轮开头注入所选 skill |
| 9 | 交互（用户输入请求/回复） | `session/manager/replies.rs:1089` | Interaction::UserInput | 含 request + reply |
| 10 | Hook 执行 | `session/manager/replies/replies_execution.rs:754` | Hook | 例如 `agent.stop` autorun |
| 11 | 系统 Notice | `session/manager/replies/replies_execution.rs:812` | Notice | 例如 `max_turns_exhausted` |
| 12 | 重试进度（仅内存） | `session/manager/history.rs:439-497` | Progress | `ProviderRetry` 投影一个 live 节点；`ProviderRetryResolved{succeeded}` 投影 `ContentRemoved` 删除它 |
| 13 | Reply 级持久错误 | `session/history/store/mod.rs:791` `project_reply_error_activity` | Error | `ExecutionFinished{Failed}` 时写 durable Error 节点，id = `ActivityId::for_reply_error(reply_id)`（稳定 id，重复失败 upsert 不重复） |
| 14 | 压缩记录 | `session/history/store/mod.rs:1500` `project_compaction_completed` | Maintenance::Compaction | 自动压缩在 replies 上追加 completed 节点 |
| 15 | 助手正文分段 | `session/history/store/mod.rs:1184` `activity_payload` | TextSegment | `PartContent::Text` + `Role::Assistant`（工具之间的正文） |
| 16 | 用户文本活动 | `session/history/store/mod.rs:1187` `activity_payload` | TextArtifact | `PartContent::Text` + `Role::User`（粘贴文本等） |

## 2. TUI 乐观输入（`agena-tui-app`）

| 生产点 | 文件 | Payload |
| --- | --- | --- |
| 剪贴板粘贴 | `app_composer_state.rs:575` `text_artifact_composer_activity` | TextArtifact |
| 附件选择 | `app_composer_state.rs:105` | Resource |
| Skill 选择 | `app_skill_picker.rs:222` / `app_composer_state.rs:736` | SkillReference |

这些是 `ComposerActivity`，发送时经 `ComposerDocument::into_turn_input` 变成 `ActivityNode`（owner=TurnInput, actor=User, state=Completed）进入持久化。

## 3. 持久化投影层（谁把 activity 写进 `agena_content_nodes`）

- `project_part_content`（`session/history/store/mod.rs:1341`）：唯一的活动节点写路径，按 `node_id` upsert，守卫 `owner_kind/owner_id/position` + `revision_seq >=`。
- `project_reply_error_activity`、`project_compaction_completed`：两类特殊节点（错误、压缩）不走 part 投影。
- 文本段（v10/v11）也写入同一张表（`node_type='text'`）。

## 4. 后台活动（独立体系，非 transcript）

统一 `BackgroundActivity` 注册表（`ActivityRegistry`，上限 256 条，活跃记录保留）：

| 来源 | 接入方式 |
| --- | --- |
| `shell.run background` 监控 | `MonitorRegistry` 的 `MonitorListener` 钩子 |
| `tasks.*` 子任务 | bus `SubtaskStatusChangedEvent` 桥接 |
| 运行时任务（marketplace 同步等） | `RuntimeBackgroundTaskRegistry` listener |
| 插件长任务（web 浏览器会话等） | `HostClient::publish_activity` + `ActivitySourceAdapter` 按 kind 注册 |
| `cron.*` | scheduler |

服务：`RuntimeActivityService`（list/get/logs/stop/dismiss/clear_finished），REST `/api/v1/activities*`，事件 `background_activity_changed` → 呈现流 `ActivityChanged`。TUI `/activities` 面板 + Web `/activities` 页。

## 5. 汇总：一条 tool activity 的完整链路

```text
provider 流式 tool_call
  → tool_calls.rs 建 Operation part (Pending, activity_id)
  → replies_execution.rs 权限通过 → InProgress + checkpoint
  → SessionStore::persist → MessagePartCheckpointed(turn_id, reply_id 由 conversation_identity_for_message 反查)
  → history.rs 投影 TranscriptPatch::ContentUpserted (增量，带 owner)
  → store 投影 project_part_content → agena_content_nodes
  → 结果流式 append_streamed_delta (model_preview 更新) / CommandOutputDelta → OperationDetailDelta
  → 完成 → ToolResultEnvelope.completed → 同 id upsert (state=completed) + TranscriptPatch
  → TUI message_render / operation_render 渲染（优先 human markdown，model_output 去重）
  → 快照 transcript_snapshot → TranscriptSnapshot (turns + session_activities)
```
