# Agena 工具 Activity 实时流式重构（已实现）

## 问题诊断（根因）

### 1. Activity 更新不及时（核心根因）
`SessionStore::persist`（`crates/agena-runtime-session/src/session/store/history.rs`）构造
`MessagePartCheckpointed` 事件时 `turn_id`/`reply_id` **硬编码为 `None`**。

后果：`project_runtime_presentation_event`（`session/manager/history.rs:332`）收到工具执行
checkpoint 事件时，因缺 owner 无法映射到 assistant reply，只能降级为 `Refresh`（全量重拉），
而非增量 `TranscriptPatch::ContentUpserted`。TUI 端工具 activity 更新 = 轮询全量快照。

对比：模型 token 流走 `TranscriptPartUpserted`（非持久化、带 owner）→ 实时增量 patch。

### 2. 工具执行开始 activity 无 InProgress 推进
`OperationPart::pending` 在 tool call 流式到达时创建（标题 `"Run <name>"`，state Pending）。
工具真正执行时没有 InProgress 转场 + checkpoint，TUI 可能长时间显示 pending。

### 3. 流式结果只流"给AI的文本"，人类视图 = AI 视图
`append_output_delta` 只把 delta 追加进 `result.model_preview.text`。`details`/`sections` 只在
工具结束时一次性写入。TUI 的 "Result" 区渲染 `model_output_text`（AI 视图文本）。

### 4. shell 实时输出不进 transcript
shell 走 `CommandOutputDelta` 独立事件（`command_event_sink`），非持久化、不产生 transcript
patch，TUI 不消费 → 用户在工具结束前看不到 shell 输出。

## 三层视图模型

- **AI 输入**：`invocation`（tool 名 + input），流式 tool call 阶段就建立（`tool_calls.rs`）。
- **AI 结果**：`result.model_preview.text`（给模型读的扁平文本）。
- **人类结果**：`HumanResultBlock`/`HumanToolResult`（`agena-domain/src/human_result.rs`），
  `OperationActivity.human_result` 承载。运行时 `ToolResultEnvelope.human`（contracts）流式累积。

## 已实现改动（worktree-activity-tool-streaming 分支，16 文件 + 813 行）

1. **owner 传播修复**（store/history.rs）：新增 `conversation_identity_for_message`，对每个
   checkpoint message 按 message→reply_executions→assistant_replies→turns SQL 反查 turn/reply，
   填入事件。改动面最小（12 处 `MessageCheckpoint` 构造点无需改）。无 reply 的 message 返回
   None（降级 Refresh，仅极早期触发）。

2. **InProgress 立即 checkpoint**（replies_execution.rs）：
   - 串行单工具：`resolve_pending_tool` 权限通过后、执行前，Pending→InProgress + persist。
   - 并行批：`execute_pending_tools_concurrently` 前，按 message 分组置 InProgress + 一次 persist。

3. **人类结果流式**（contracts）：
   - `OperationPart::append_streamed_delta`：同时更新 model_preview + `result.human`。
   - `append_tool_output_delta` 路由到它（流式 checkpoint 时人类块实时累积）。
   - `ToolResultEnvelope::completed/failed/non_execution` 填充 human。
   - `human_result_from_operation`：从 result.content 投影人类块（command/diff/file_changes 等）。

4. **结构化人类块产出**（manager/helpers.rs `operation_blocks_from_tool_output`）：
   - shell run → `OperationBlock::Command`（$ cmd + stdout + exit_code）。
   - apply_patch → `OperationBlock::FileChanges` + `OperationBlock::Diff`（修改前后 diff）。

5. **TUI 渲染**：
   - `message_render.rs`（Canonical 路径）：优先渲染 `human_result.blocks`（markdown 化：
     command→```sh 卡片、diff→```diff、file_changes→人类清单），`model_output_text` 仅在无
     人类块时作 Result。
   - `operation_render.rs`（resource 路径）：同样优先 `tool.result.human.blocks`。
   - `HumanResultBlockResource`/`HumanToolResultResource` 加入 agena-api，跨边界序列化稳定。

## 通讯/性能
- 实时更新走带 owner 的增量 `TranscriptPatch`（复用模型 token 流的非持久化模式先例）。
- 高频流式 delta 累积内存，2s 批持久化（既有 `DELTA_BATCH_MS`），每批一次 DB owner 反查。
- TUI 每次 patch `invalidate_render` 全量重绘是既有瓶颈，本轮未处理（可选后续优化）。

## 测试
- 新增：`human_result_tests`（contracts，流式 delta + 结构化投影）、
  `conversation_identity_is_none_for_message_without_reply`（runtime-session）。
- 全 workspace 编译通过；domain/contracts/session/api/tui 441 测试全绿。

## 已知边界
- 流式过程中人类块 = Text block（model_preview 增长），结束时替换为结构化块。
- 并行工具（阻塞路径）不产流式人类块，但 InProgress 转场已覆盖。
- `OperationPartResource`/`ToolResultEnvelopeResource` 的 `human` 字段需与运行时 `OperationPart`
  serde 保持同步（均已添加）。
