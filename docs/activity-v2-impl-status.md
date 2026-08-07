# Activity v2 实施状态（完成）

> 分支: `agent/activity-v2`（基于 master `684181e2`，worktree: `.agena/worktrees/activity-v2`）
> 设计依据: `docs/activity-mechanism-review/07-comprehensive-redesign.md`（v3.1）+ `08-plugin-contract.md`
> 策略: **新代码先行、旧代码后撤**——新子系统每步独立可编译可测；旧路径并行保留至收尾阶段，人类视图 Golden 保持。

## 最终状态：P1–P7 全部完成，全 workspace 测试绿

| 提交 | 阶段 | 内容 | 测试 |
|---|---|---|---|
| `346c759c` | P1 领域层 | `agena-domain/src/activity_v2.rs`：`RawOutput`（单一事实源）、`ViewBlock`（11 种渲染契约）、`RenderDelta`/`DeltaMode`（实时增量）、`ActivityView` trait | domain 84 |
| `e0196b62` | P2 协议层 | `agena-tool/src/tool_activity.rs`：`ToolActivityEvent`、`ToolActivityResult`、`RenderContext`、`RenderError`、`ToolHumanRenderer` trait | agena-tool 27 |
| `5f88d170` | P3a 运行时状态机 | `agena-runtime-session/src/activity/mod.rs`：`ActivityKind`（9 收敛变体）、`ActivityStateNode`、`ActivityLiveEvent`（统一 wire）、`ActivityHandler` | runtime-session 161 |
| `083c6b85` | P3b 投影器 | `activity/projection.rs`：`fallback_human_view`、`for_model`（纯函数投影） | runtime-session 158 |
| `3998cacb` | P3c 接线 | `EventKind::ActivityV2` 桥接、流式执行 `RenderDelta`（首块 New + Append）、标题 2s 刷新、presentation/SSE 透传 | runtime-session / runtime / application / api-server 全绿 |
| `2693d5f7` | P3d 存储 | `session/store/activity_v2.rs`：`upsert_content_node`（终态 RawOutput 落库、revision guard）、`update_activity_label`（O(1) 列更新）；流式零写库 | runtime-session 161（+3） |
| `b03f32bd` | P4 SSE/Web | `event_projection.rs` 形状断言：`activity_v2` 经泛化投影直通 SSE，payload 含 `activity_id/block_id/mode/view` | application 12、api-server 5 |
| `b69ebf1c` | P5 TUI | `transcript_state.rs` 消费 `ActivityLiveEvent`（DetailDelta/TitleChanged/StateChanged/Upserted/Removed）+ `render_activity_block`（11 变体统一渲染）；旧路径并行 | agena-tui-app 168 |
| `fc44ebd0` | P6 工具迁移 | `agena-runtime-tools/src/tool/human_view.rs`：`BuiltinHumanRenderer`（ToolPayloadOutput → ViewBlock）；流式工具发送 RenderDelta；接线进 streaming ActivityHandler | runtime-tools 61、runtime-session 161 |
| （收尾） | P7 清理 | P3c 遗留注释错位修复；文档完成态；旧 `ActivityPayload` 保持（活跃持久化格式，非死代码） | 全 workspace 绿 |

## 架构（已落地）

```text
工具流 ── ToolActivityEvent 流 ──► ActivityHandler（内存累积 + 增量合并 + 标题接管）
                                    │ 产出 ActivityLiveEvent（统一 wire，非持久）
                                    ▼
                             EventKind::ActivityV2 ──► SSE / TUI 同一套消费
                                    │
工具 ToolActivityResult ── finish() ──► ActivityStateNode（title/summary/raw_output）
                                    │
                        upsert_content_node（终态一次写：RawOutput 落库）
                        render_human / fallback_human_view（投影，纯函数不落盘）
```

## 关键设计约束（已落实）

1. **单一事实源**：持久化只有 `RawOutput`；不存在人类/AI 副本字段；投影纯函数不落盘。
2. **渲染归属工具**：人类视图优先 `render_human`（`BuiltinHumanRenderer`）；无则 `fallback_human_view`。
3. **实时 ≠ 写库**：流式 `RenderDelta` 只进内存 + 广播；标题 O(1) 列更新（2s）；终态一次写。
4. **标题不推导**：`title()` 只读已存字段；工具接管后停止自动 `· Ns`。
5. **统一 wire**：TUI 与 Web 消费同一 `ActivityLiveEvent`；`ViewBlock` 一个渲染器。

## 验证基线（全绿）

- `cargo test -p agena-domain` → 84
- `cargo test -p agena-tool` → 27
- `cargo test -p agena-runtime-session` → 161（含 activity_v2 存储 3）
- `cargo test -p agena-runtime-tools` → 61（含 human_view 4）
- `cargo test -p agena-runtime` → 59
- `cargo test -p agena-application` → 12
- `cargo test -p agena-api-server` → 5
- `cargo test -p agena-tui-app` → 168（含 activity_v2 2）

## 备注（P7 收敛边界）

- 旧 `ActivityPayload`（18 变体）**保留**：仍是持久化 `content_nodes.payload_json` 与 TUI snapshot 渲染的活跃格式（ProviderRetry 投影、Hook/Maintenance 存储、message_render 分支均有引用），非死代码；v2 新体系（`ActivityKind` 9 变体）已并行上线。
- 旧 `operation_blocks_from_tool_output`（helpers.rs）保留：人类视图 Golden 保持，`BuiltinHumanRenderer` 为其 v2 等价下放。
- `Broadcast_activity_v2` 注释错位（P3c 遗留）已修复。
