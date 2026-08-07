# Activity v2 实施状态与剩余路线

> 分支: `agent/activity-v2`（基于最新 master `684181e2`，worktree: `.agena/worktrees/activity-v2`）
> 设计依据: `docs/activity-mechanism-review/07-comprehensive-redesign.md`（v3.1）+ `08-plugin-contract.md`
> 策略: **新代码先行、旧代码后撤**——新子系统每步独立可编译可测，最终删除全部旧 activity 代码；过程不留不可编译状态（防止返工）。

## 已完成（4 个提交，全部测试绿，零回归）

| 提交 | 阶段 | 内容 | 测试 |
|---|---|---|---|
| `346c759c` | P1 领域层 | `agena-domain/src/activity_v2.rs`：`RawOutput`（单一事实源）、`ViewBlock`（11 种渲染契约）、`RenderDelta`/`DeltaMode`（实时增量）、`ActivityView` trait | domain 84 通过（含新 5） |
| `e0196b62` | P2 协议层 | `agena-tool/src/tool_activity.rs`：`ToolActivityEvent`（Render/Title/TitleSuffix/Summary/Section/Attachment/Metadata）、`ToolActivityResult`、`RenderContext`、`RenderError`、`ToolHumanRenderer` trait | agena-tool 27 通过（含新 5） |
| `5f88d170` | P3a 运行时状态机 | `agena-runtime-session/src/activity/mod.rs`：`ActivityKind`（9 收敛变体）、`ActivityStateNode`、`ActivityLiveEvent`（统一 wire：DetailDelta/TitleChanged/SummaryChanged/StateChanged/Upserted/Removed）、`ActivityHandler`（增量合并、标题接管、终态组装） | runtime-session 154 通过（含新 5） |
| `083c6b85` | P3b 投影器 | `activity/projection.rs`：`fallback_human_view(raw)`（无渲染函数时直接渲染原始输出）、`for_model(raw)`（模型侧投影，structured JSON 优先） | runtime-session 158 通过（含新 4） |

## 新子系统架构（已立起）

```text
工具（未来）── ToolActivityEvent 流 ──► ActivityHandler（内存累积 + 增量合并 + 标题接管）
                                          │ 产出 ActivityLiveEvent（统一 wire）
                                          ▼
                                   TUI / Web（未来同一套消费）
                                          │
工具 ToolActivityResult ──► finish() ──► ActivityStateNode（title/summary/raw_output）
                                          │
                       fallback_human_view / for_model（投影，纯函数不落盘）
```

## 剩余路线（按依赖顺序，每步可独立合并、测试绿）

| 步骤 | 内容 | 入口 | 风险 |
|---|---|---|---|
| P3c 执行流接线 | 把 `ActivityHandler` 接入真实工具执行：改造 `replies_execution.rs` 流式循环（`execute_pending_tools` 流式路径），工具事件流 → handler；先接 shell 一个工具 | `session/manager/replies/replies_execution.rs:2530-2630`（流式循环）、`tool_calls.rs`（Operation 创建） | 高：最大改造；保持旧路径并行输出，验证后切换 |
| P3d 存储接线 | `data` 列 = `RawOutput`；写路径收敛 `upsert_content_node` + `update_activity_label`（O(1) 列更新）；删除 8 个 INSERT | `session/history/store/mod.rs`、`session/store/history.rs` | 中 |
| P4 实时通道 | 统一 wire 事件接入 SSE：`sse.rs` 事件形状、`event_projection.rs` 补 `ActivityLiveEvent` 投影（Web 实时展开）、`GET detail?live=1` | `agena-api-server/src/sse.rs`、`agena-application/src/event_projection.rs` | 中 |
| P5 TUI | `transcript_state.rs` 消费 `ActivityLiveEvent`（替换 `OperationDetailDelta`/`TranscriptPatch` 旧路径）；渲染 `ViewBlock`（统一 `render_activity_block`） | `agena-tui-app/src/transcript_state.rs`、`agena-tui-transcript/src/renderer/*` | 中 |
| P6 工具迁移 | 内置工具（shell/fs/web/apply_patch…）各自实现 `ToolHumanRenderer` + 流式 `RenderDelta`；`operation_blocks_from_tool_output` 下放为各工具 `render_human` | `agena-runtime-tools/src/tool/*`、`agena-plugin-sdk/src/hooks/tool.rs` | 中；人类视图 Golden 保持 |
| P7 清理 | 删除旧 `activity.rs` 死变体、旧投影、双渲染路径；`ActivityPayload` 收敛为 9 变体；文档更新 | 全域 | 低 |

## 关键设计约束（实现时不得违反）

1. **单一事实源**：持久化只有 `RawOutput`；不存在人类/AI 副本字段；投影纯函数不落盘（07 P1/P2）。
2. **渲染归属工具**：人类视图优先 `render_human`；无则 `fallback_human_view`（与模型同源）（07 P3）。
3. **实时 ≠ 写库**：流式 `RenderDelta` 只进内存 + 广播；标题 O(1) 列更新（2s）；终态一次写（07 §8）。
4. **标题不推导**：`title()` 只读已存字段；工具接管后停止自动 `· Ns`（Golden I1）。
5. **统一 wire**：TUI 与 Web 消费同一 `ActivityLiveEvent`；`ViewBlock` 一个渲染器。

## 验证基线

- `cargo test -p agena-domain` → 84 通过
- `cargo test -p agena-tool` → 27 通过
- `cargo test -p agena-runtime-session` → 158 通过
- 每步新增测试：serde roundtrip、增量合并、标题接管、投影、写放大断言（P3d 后）
