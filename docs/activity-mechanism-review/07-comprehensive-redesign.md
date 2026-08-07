# 07 总设计：Activity 体系彻底重构（推翻重来）方案

> 状态：总设计提案（未实施）｜适用分支：`agent/activity-mechanism-review`（实施时开 `agent/activity-v2`）
> 本设计是**统一体**，不是补丁堆叠：一套活动模型、一套事件协议、一套实时通道、一套持久化策略、一套渲染契约。
> 明确两个硬目标：
> 1. **实时性**：工具（尤其 shell run）执行中，用户展开 activity 即可实时看到输出（TUI + Web）；
> 2. **零/有界写库**：实时性绝不转化为无限数据库写入，写放大有预算上限。

## 1. 现状问题的根因（为什么推倒重来）

已逐行验证的碎片化证据：

| # | 碎片 | 位置 | 后果 |
|---|---|---|---|
| 1 | 实时 detail 只达 TUI，Web 不消费 | `event_projection.rs` 无 `CommandOutputDelta`/`OperationDetailDelta` 投影；`sse.rs` 只转发投影过的事件 | Web 用户展开 shell 看不到实时输出 |
| 2 | 实时事件形状不统一 | `CommandOutputDelta`（命令专用）+ `OperationDetailDelta`（TUI 内部）+ `TranscriptPatch`（持久化增量）三套并行 | 每个客户端各写一套消费逻辑 |
| 3 | 流式期间工具只能发纯文本 | `ToolStreamChunk = { text_delta, metadata }`（`hooks/tool.rs:334`） | 无法实时推结构化块/标题/摘要 |
| 4 | 人类块结束才隐式派生 | `operation_blocks_from_tool_output`（`helpers.rs:153`）按 payload 形状猜块 | 工具不能声明自己要什么块 |
| 5 | 人类视图 vs AI 内容不能分别声明 | `project_operation_output`（`wire_message.rs:630`）隐式投影 | 工具无法控制“给人看”与“给模型读” |
| 6 | 三套并行渲染/标题推导 | snapshot `operation_activity_title`、resource `tool_display_label`、web `activityTitle` | 不一致风险 |
| 7 | 8 个 `INSERT INTO agena_content_nodes`、多套状态映射 | store 各文件 | 写路径不可控 |
| 8 | 18 个 ActivityPayload 变体含死变体 | `activity.rs:346` | 类型膨胀、语义不清 |

**结论**：不是单个点坏了，而是“事件、模型、存储、渲染”四层各自演化、互相手工映射。推倒重来 = 四层统一重建，保留已验证的好地基（单表、revision、seq 事件流）。

## 2. 目标架构总览

```text
┌─────────────┐   ToolActivityEvent 流（实时）   ┌──────────────────────────┐
│ 工具执行器   │ ──────────────────────────────► │ 运行时 Activity 处理器      │
│ shell/fs/…  │   ToolActivityResult（终态）     │  agena-runtime-session    │
└─────────────┘                                  └────────────┬─────────────┘
                                                              │
                     ┌────────────────────────────────────────┼───────────────────────────┐
                     │ 实时广播（内存总线，零写库）              │ 有界持久化                  │ 投影
                     ▼                                        ▼                           ▼
              ┌──────────────┐                        ┌──────────────┐          ┌──────────────┐
              │ 事件流服务    │                        │ 存储（单表）   │          │ 模型侧投影    │
              │ seq_global   │                        │ content_nodes│          │ wire_message │
              │ + Lagged     │                        │ + O(1) 列更新 │          └──────────────┘
              └──────┬───────┘                        └──────────────┘
                     │ 统一实时通道（SSE / WS）
                     ▼
        ┌───────────────────────┐
        │ TUI  /  Web            │  同一套 wire 事件：
        │ 同一套事件消费逻辑      │  activity.upserted / removed /
        └───────────────────────┘  operation.detail_delta / title_changed / …
```

## 3. 统一活动模型（领域层）

### 3.1 收敛后的 Activity 类型

- `ActivityPayload` 收敛为 **8 个活变体**（删除死变体 SkillExecution/Checklist/Search/FileChanges/NestedTask/Custom/Progress 或接线后保留）：
  `Operation`、`Resource`、`SkillReference`、`Reasoning`、`TextArtifact`、`TextSegment`、`Interaction`、`Error`、`Notice`（9 个，以生产端为准）。
- 每个变体实现统一视图 trait（**只读已存字段，绝不从 data 推导**）：
```rust
pub trait ActivityView {
    fn title(&self) -> &str;        // 已持久化标题（标签）
    fn summary(&self) -> &str;      // 已持久化摘要
    fn human_blocks(&self) -> &[ActivityBlock];  // 人类视图（显式优先）
    fn model_view(&self) -> Option<ModelOutput>; // 给 AI 的原文（显式优先）
}
```

### 3.2 统一渲染契约

- 唯一的 `ActivityBlock` 枚举（工具可声明、可持久化、三端可渲染）：Text/Markdown/Json/Table/Log/Command/FileChanges/Diff/SearchResults/Media/Custom。
- 唯一渲染入口：`render_activity_block(block) -> markdown`；不再有“从 payload 猜块”的双路径（隐式派生仅作迁移期 fallback）。

## 4. 统一事件协议（工具 → 运行时 → 客户端）

### 4.1 工具侧事件流（`agena-tool` 定义，SDK 兼容扩展）

```rust
pub enum ToolActivityEvent {
    Title(String),                      // 实时标题（工具接管后停自动秒数）
    TitleSuffix(String),                // 状态后缀（` · scanning`）
    TextDelta(String),                  // 展开文本增量（兼容现有 text_delta）
    Block(ActivityBlock),               // 展开结构化块增量（新）
    Summary(String),                    // 实时摘要
    Section(ToolPresentationSection),
    ModelDelta(String),                 // 给 AI 的原文增量（缺省=累积 TextDelta）
    Attachment(AttachmentItem),
    Metadata { key: String, value: String },
}

pub struct ToolActivityResult {          // 终态（全部可选，缺省走现状推导）
    pub title: Option<String>,
    pub summary: Option<String>,
    pub human_blocks: Option<Vec<ActivityBlock>>,
    pub sections: Vec<ToolPresentationSection>,
    pub model_output: Option<ModelOutput>,   // Text | Structured
    pub payload: Option<serde_json::Value>, // compact 持久化
    pub attachments: Vec<AttachmentItem>,
}
```

### 4.2 运行时 → 客户端 wire 事件（统一形状，SSE/WS 同构）

```json
{ "seq": 42, "type": "operation.detail_delta",
  "session_id": 7, "activity_id": "…", "block": { "kind": "log", "stream": "stdout", "text": "…” } }
{ "seq": 43, "type": "operation.title_changed", "session_id": 7, "activity_id": "…", "title": "cargo test · 12s" }
{ "seq": 44, "type": "activity.upserted", "session_id": 7, "node": { … } }   // 快照增量（持久化后）
{ "seq": 45, "type": "activity.removed", "session_id": 7, "activity_id": "…" }
{ "seq": 46, "type": "reply.status", "session_id": 7, "reply_id": 9, "status": "in_progress" }
{ "seq": 47, "type": "refresh", "reason": "ownerless-downgrade" }            // 兜底
```

- 事件按 `seq_global` 单调编号；客户端 `?since_seq=` 恢复（先重放持久化事件，再挂实时）；订阅过慢发 `lagged` 通知。
- **TUI 与 Web 消费同一套 wire 事件**，消除 `CommandOutputDelta`/`OperationDetailDelta`/`TranscriptPatch` 三套并行。

## 5. 实时通道与 API 设计

| 接口 | 方法/路径 | 说明 |
|---|---|---|
| 会话快照 | `GET /api/v1/sessions/{id}` | 全量（含 activity 树） |
| 实时事件流 | `GET /api/v1/sessions/{id}/stream?since_seq=&kinds=` | SSE；重放+实时；Lagged 通知 |
| 双向控制 | `WS /api/v1/sessions/{id}` | 取消/输入/订阅（可选，SSE+POST 可替代） |
| 展开详情（lazy） | `GET /api/v1/sessions/{id}/operations/{activity_id}/detail` | 展开时拉全量 detail |
| 实时详情增量 | 事件 `operation.detail_delta` | 展开时订阅，实时推送 |
| 发消息 | `POST /api/v1/sessions/{id}/replies` | 现有 |
| 工具权限回复 | `POST /api/v1/sessions/{id}/permission/reply` | 现有 |

**客户端订阅协议（简单版）**：
1. 打开会话 → `GET snapshot` 渲染初始；
2. 同时 `GET stream?since_seq=<快照seq>` 订阅；
3. 收到 `operation.detail_delta` 且 activity 展开 → 就地追加（Text 追加 / Block 追加卡片）；
4. 收到 `activity.upserted`（终态）→ 替换该节点；
5. 收到 `lagged` → 全量刷新。

## 6. 持久化策略：实时 ≠ 无限写库

### 6.1 写放大预算（硬约束）

| 阶段 | 写入 | 频率上限 |
|---|---|---|
| 流式期间（detail 文本/块） | **零写库**：只进内存广播 + 客户端本地累积 | — |
| 流式期间（标题） | `UPDATE agena_content_nodes SET title=?`（O(1) 单列） | 2s 一次（`TITLE_REFRESH_MS`） |
| 流式期间（摘要，可选） | `UPDATE … SET summary=?`（O(1) 单列） | 2s 一次 |
| 终态 | 一次性 `upsert_content_node`（整行） | 每工具一次 |
| 崩溃恢复 | 无（流式内容不落盘；恢复后显示“中断”，可重跑） | — |

- 核心原则：**detail 永不因实时而持久化**。展开实时视图 = 内存事件 + 客户端累积；终态 detail = 结束时一次性写入的 compact data，渲染时派生。
- 上限证明：一个长跑 10 分钟的 shell，数据库写入 ≈ 300 次标题列更新（2s×300）+ 1 次终态写；输出文本本身 0 次写。
- 可选进阶：detail checkpoint（每 10s 或每 1MB 阈值）用于超长任务崩溃续显，默认关闭。

### 6.2 存储收敛

- 保留单表 `agena_content_nodes`（已验证）；写路径收敛为两个内部函数：
  - `upsert_content_node(activity, owner, …)` —— 终态/节点创建（唯一 INSERT/UPDATE 入口）；
  - `update_activity_label(session, activity_id, title?, summary?, state?)` —— 流式期间的 O(1) 列更新（唯一列更新入口）。
- `data` 列 JSON 向后兼容扩展（`human_blocks`/`model_output` 可选字段，旧数据零迁移）。

## 7. 投影器（单一职责）

| 投影 | 输入 | 输出 | 现状位置 → 新位置 |
|---|---|---|---|
| 模型侧 | Operation/Resource/… | provider wire（ToolResult 原文） | `wire_message.rs project_persisted` → `ActivityProjector::for_model` |
| 人类侧 | ActivityBlock/sections | markdown/卡片 | `render_tool_payload_markdown` + `operation_blocks_from_tool_output` → `ActivityProjector::for_human` |
| 标签侧 | title/summary | 标题行 | 三端重复函数 → `ActivityView::title/summary` |

三个投影器都遵循“**显式优先、隐式兜底**”：工具声明了 `human_blocks`/`model_output` 就用声明的；否则用现状推导（保证迁移期 Golden Invariants）。

## 8. 落地路线（推翻重来，但分阶段切换）

| 阶段 | 内容 | 产出 | 验证 |
|---|---|---|---|
| P0 | 统一契约定稿（本文档） | 契约文档 | 评审 |
| P1 | 领域收敛：删死变体、`ActivityView`、`ActivityBlock` 统一 | 新 domain 类型 | `cargo test` |
| P2 | 事件协议：`ToolActivityEvent`/`ToolActivityResult`（agena-tool + SDK 兼容） | 协议类型 + wire 序列化 | SDK 旧插件测试 |
| P3 | 运行时处理器：事件路由、内存广播、有界持久化（两入口）、三投影器 | `ActivityHandler` | 单测 + 写放大断言 |
| P4 | 实时通道统一：SSE 事件形状（含 `operation.detail_delta`/`title_changed`）+ Web 消费 | Web 实时展开 | 端到端 shell 实时测试 |
| P5 | TUI 切到统一事件（删 `CommandOutputDelta`/`OperationDetailDelta` 特殊路径） | TUI 统一消费 | 渲染快照 |
| P6 | 工具迁移：shell/fs/web 声明实时标题/块/模型原文 | 内置工具新能力 | 端到端 |
| P7 | 清理：删除旧投影/双渲染/死代码，8 INSERT 收敛 | 干净架构 | 全量测试 + 黄金快照 |

每阶段可独立合并，但 P1–P5 属于统一体设计，接口在 P0 定死，避免中途打补丁。

## 9. 保留 vs 废弃清单

**保留（已验证的好地基）**：单表 `content_nodes`；revision_seq 收敛；`seq_global` 事件流 + Lagged；`normalize_tool_title/summary`；O(1) 列更新思想；lazy detail REST；owner 反查（streaming-refactor 分支）。
**废弃/替换**：8 个 INSERT → 1 个 `upsert_content_node`；三套并行渲染标题函数 → `ActivityView`；`CommandOutputDelta`/`OperationDetailDelta`/`TranscriptPatch` 三事件 → 统一 wire 事件；隐式块派生（仅作 fallback）；Web 无实时（补齐）；`MessagePartDetailResource` 缺 Notice（补齐）；死变体（删除）。

## 10. 简单工作流程（端到端示例：shell run）

```text
1. 用户 POST /replies ──────────────► 模型返回 tool_call(shell.run "cargo test")
2. 运行时创建 Operation Activity ────► 广播 activity.upserted（Pending）
3. 工具开始执行 ────────────────────► 广播 activity.upserted（InProgress）+ reply.status
4. 工具推事件流：
     Title("cargo test")            ► operation.title_changed（O(1) 列更新，2s 节流）
     Block(Log{stdout, "…编译…"})   ► operation.detail_delta（内存广播，200ms 节流）
     Block(Log{stdout, "…运行…"})   ► …
   ── 用户展开该 activity ──────────► TUI/Web 实时看到 Log 卡片逐条出现
5. 工具结束 ────────────────────────► 运行时组装 ToolActivityResult
     ├─ human_blocks（显式）→ 一次性 upsert_content_node（终态）
     ├─ model_output（显式/默认投影）→ 模型收到 ToolResult
     └─ 广播 activity.upserted（Completed）
6. 期间数据库写入：约 300 次标题列更新（10 分钟）+ 1 次终态写，0 次输出文本写。
```

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| 分阶段切换期间新旧事件并存 | P4 前保留旧路径为 fallback；P5 原子切换 TUI；统一 wire 事件是唯一前进契约 |
| `model_output` 改变模型输入 → 模型行为变化 | opt-in；P6 单独评审；默认保持现状投影 |
| 内存广播背压 | 订阅者队列有界 + `lagged` 通知 + detail 只发给展开该 activity 的订阅者 |
| 事件乱序 | `seq_global` 单调 + 客户端按 seq 丢弃旧事件 |
| 长任务崩溃丢流式内容 | 默认接受（显示中断）；可选 detail checkpoint（默认关） |
| 标题自动秒数 vs 工具接管 | 发过 `Title` 即接管；未发保持 `{base}·Ns`（Golden） |

## 12. 验收标准

1. **实时**：`shell run` 长任务，TUI 与 Web 展开 activity 均能实时看到输出（≤200ms 延迟）；
2. **写库有界**：流式期间 DB 写入 = 标题/摘要列更新（≤0.5 次/s），输出文本 0 写；有测试断言写放大；
3. **统一**：TUI 与 Web 消费同一套 wire 事件；渲染只有一个 `render_activity_block`；状态映射只有一个函数；
4. **兼容**：未声明新能力的工具渲染与模型输入与现状逐字一致（Golden Invariants I1–I4）；旧插件兼容；
5. 全量测试绿 + 黄金快照通过。
