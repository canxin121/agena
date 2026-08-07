# 07 总设计：Activity 体系彻底重构（推翻重来）方案 · v3.1（工具自渲染 + 实时增量渲染）

> 状态：总设计提案 v3.1（未实施）｜适用分支：`agent/activity-mechanism-review`（实施时开 `agent/activity-v2`）
> v3.1 相对 v3 的补充：**明确“人类视图实时更新”的机制**——
> 实时视图 = 工具流式推送的**渲染增量事件**（客户端展开时实时追加，零写库）；
> 终态权威视图 = 工具 `render_human(raw)` 一次性生成，替换预览。两者内容一致由工具负责 + 快照测试保证。
> 保留 v3：渲染职责下沉给工具；无渲染函数时 fallback 直接渲染原始输出（即给 AI 的内容）；单一事实源只存 raw_output。
> 硬目标不变：1) 实时性（shell run 展开实时看输出，TUI+Web）；2) 实时 ≠ 无限写库。

## 1. 现状问题的根因（为什么推倒重来）

已逐行验证的碎片化证据：

| # | 碎片 | 位置 | 后果 |
|---|---|---|---|
| 1 | 实时 detail 只达 TUI，Web 不消费 | `event_projection.rs` 无 `CommandOutputDelta`/`OperationDetailDelta` 投影；`sse.rs` 只转发投影过的事件 | Web 用户展开 shell 看不到实时输出 |
| 2 | 实时事件形状不统一 | `CommandOutputDelta` + `OperationDetailDelta` + `TranscriptPatch` 三套并行 | 每个客户端各写一套消费逻辑 |
| 3 | 流式期间工具只能发纯文本 | `ToolStreamChunk = { text_delta, metadata }` | 无法实时推结构化块/标题/摘要 |
| 4 | 人类块由**运行时**按工具名猜 | `operation_blocks_from_tool_output`（`helpers.rs:153`）if-match payload 形状 | 工具作者无法自定义“给人看的样子”；运行时维护每类工具的渲染约定 |
| 5 | 人类视图 vs AI 内容不能分别控制 | `project_operation_output`（`wire_message.rs:630`）隐式投影 | 工具无法控制两方 |
| 6 | 三套并行渲染/标题推导 | snapshot/resource/web 各自函数 | 不一致风险 |
| 7 | 8 个 INSERT、多套状态映射 | store 各文件 | 写路径不可控 |
| 8 | 18 个 ActivityPayload 变体含死变体 | `activity.rs:346` | 类型膨胀、语义不清 |

**结论**：核心矛盾是“渲染归属错了”——给人看的样子由运行时猜，而不是由最了解输出的工具自己决定。推倒重来 = 把渲染职责还给工具，运行时只管事件、存储与 fallback。

## 2. 核心原则

### P1 单一事实源
每个 activity 只存一份**原始事实（raw output）**：`payload`（机器事实）+ `text`（文本事实）+ `attachments` + `metadata`。不存在“人类副本”“AI 副本”“块副本”。

### P2 视图是即时投影/渲染，永不落盘
- **给 AI 看**：`for_model(raw)` 纯函数投影（payload 优先 → text）。
- **给人看**：**工具自己的渲染函数** `render_human(raw) -> ViewBlocks`（v3 核心）。
- **给标签**：`title/summary` 列（已持久化）。
三个视图都不进数据库。

### P3 渲染职责下沉给工具
“如何把输出渲染给人类”是**工具的职责**：工具注册时声明渲染函数；运行时需要人类视图时调用它。**工具没有渲染函数时，运行时才直接渲染原始输出（即给 AI 的内容）作为 fallback**。

### P4 流式内容不进库，实时由渲染增量事件承载（v3.1 强调）
实时视图不是“反复调用 render_human”，而是**工具流式推送渲染增量**（`RenderDelta`），客户端展开时实时追加/更新；终态一次性写 raw_output。

### P5 显式 = 控制事实与渲染，不是提供副本
工具控制“两方不同”的两条途径：1) 控制**事实维度**（payload/text）；2) 控制**渲染函数与渲染增量**。

## 3. 目标架构总览

```text
┌─────────────┐ RenderDelta 流（实时渲染增量）  ┌──────────────────────────┐
│ 工具执行器   │ ──────────────────────────────► │ 运行时 Activity 处理器      │
│ shell/fs/…  │ ToolActivityResult（终态原始事实）│ agena-runtime-session    │
└──────┬──────┘                                  └────────────┬─────────────┘
       │ 注册 render_human 渲染函数                            │
       ▼                                                     │
 ┌──────────────────────┐        ┌────────────────────────────┼───────────────┐
 │ 工具渲染函数           │        │ 内存累积 raw（零写库）        │ 终态一次写 raw │
 │ render_human(raw)    │        ▼                            ▼               ▼
 │ → ViewBlocks         │  ┌──────────────┐ 统一 wire 事件  ┌──────────────┐  ┌──────────────┐
 │（无则运行时 fallback  │  │ 事件流服务     │───────────────► │ 存储（单表）   │  │ for_model     │
 │  渲染原始输出）        │  │ seq + Lagged  │                │ content_nodes│  │（原始事实投影）│
 └──────────────────────┘  └──────┬───────┘                └──────────────┘  └──────┬───────┘
                                  │ SSE/WS                                     │
                                  ▼                                             ▼
                        ┌──────────────────────┐                     模型读到 ToolResult
                        │ TUI / Web（同一套消费） │
                        │ 展开时：
                        │   · 进行中：detail?live=1 拉内存累积视图 + 订阅 detail_delta 实时追加
                        │   · 终态：detail = render_human(raw) 或 fallback(raw)
                        └──────────────────────┘
```

## 4. 统一活动模型（领域层）

### 4.1 收敛后的 Activity 类型

- `ActivityPayload` 收敛为 **9 个活变体**：`Operation`、`Resource`、`SkillReference`、`Reasoning`、`TextArtifact`、`TextSegment`、`Interaction`、`Error`、`Notice`。
- 统一视图 trait（只读，不另存）：
```rust
pub trait ActivityView {
    fn title(&self) -> &str;                 // 标签列
    fn summary(&self) -> &str;               // 标签列
    fn raw_output(&self) -> &RawOutput;      // 单一事实源
}
```

### 4.2 单一事实源（raw output）

```rust
pub struct RawOutput {
    pub payload: Option<serde_json::Value>,   // 机器可读事实（模型投影主源；也是渲染函数输入）
    pub text: String,                         // 文本事实（模型 fallback；也是渲染函数输入）
    pub attachments: Vec<AttachmentItem>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub truncated: bool,
}
```

> 不再存储 blocks。结构化人类块是渲染函数的**输出**（ViewBlocks），即时计算、不落盘。

### 4.3 工具渲染函数与视图描述

```rust
/// 视图描述块：可序列化到 wire，TUI/Web 复用同一渲染器。
pub enum ViewBlock {
    Text { id: Option<String>, text: String },              // 可增量追加
    Markdown { id: Option<String>, text: String },
    Json { id: Option<String>, value: serde_json::Value },
    Table { id: Option<String>, columns: Vec<String>, rows: Vec<Vec<serde_json::Value>> },
    Log { id: Option<String>, stream: LogStream, text: String },  // 流式追加的载体
    Command { id: Option<String>, command: String, cwd: Option<String>, exit_code: Option<i32>, stdout: String, stderr: String },
    FileChanges { id: Option<String>, changes: Vec<FileChange> },
    Diff { id: Option<String>, diff: String, language: Option<String> },
    SearchResults { id: Option<String>, /* … */ },
    Media { id: Option<String>, mime_type: String, artifact: ArtifactRef },
    Custom { id: Option<String>, kind: String, schema: serde_json::Value, presentation: BTreeMap<String, String> },
}

/// 工具声明的人类渲染函数（注册时提供）。
pub trait ToolHumanRenderer {
    /// 从原始输出生成完整权威人类视图。要求确定性、无副作用。
    fn render_human(&self, ctx: &RenderContext, raw: &RawOutput) -> Result<Vec<ViewBlock>, RenderError>;
}
```

## 5. 统一事件协议（工具 → 运行时 → 客户端）

### 5.1 工具侧：实时渲染增量 + 终态事实

```rust
/// 渲染增量：实时“给人看”的最小单元（v3.1 核心）。
pub struct RenderDelta {
    pub block_id: Option<String>,   // 稳定块 id：None = 新块；Some(id) = 更新已有块
    pub mode: DeltaMode,
    pub view: ViewBlock,
}
pub enum DeltaMode {
    New,      // 新增卡片
    Append,   // 追加到同 id 块（Log 文本追加、Markdown 拼接）
    Replace,  // 整块替换（Table/Json 更新、Command 完成态）
}

pub enum ToolActivityEvent {
    Title(String),                      // 实时标题
    TitleSuffix(String),
    Render(RenderDelta),                // 实时渲染增量（替代纯 text_delta）
    Summary(String),
    Section(ToolPresentationSection),
    Attachment(AttachmentItem),
    Metadata { key: String, value: String },
}

pub struct ToolActivityResult {          // 终态（全部可选，缺省走现状推导）
    pub title: Option<String>,
    pub summary: Option<String>,
    pub raw_output: RawOutput,           // 唯一存储
    pub sections: Vec<ToolPresentationSection>,
}
```

**增量语义示例（shell run）**：
```text
工具推 Render{ block_id: None, New, Log{stream: stdout, text: "编译中…\n" } }   → 新建 Log 卡片
工具推 Render{ block_id: "out", Append, Log{stream: stdout, text: "✓ 完成\n" } } → 同卡片追加
工具推 Render{ block_id: "out", Replace, Command{ stdout: "…全部…", exit_code: 0 } } → 终态替换为完整 Command 卡片
```

> 工具责任：流式 RenderDelta 是 `render_human(raw)` 的**实时切片**——终态渲染结果必须包含全部流式增量内容（一致性由快照测试断言）。

### 5.2 运行时 → 客户端 wire 事件（统一形状，SSE/WS 同构）

```json
{ "seq": 42, "type": "operation.detail_delta", "session_id": 7, "activity_id": "…", "mode": "append", "block_id": "out", "view": { "kind": "log", "stream": "stdout", "text": "…" } }
{ "seq": 43, "type": "operation.title_changed", "session_id": 7, "activity_id": "…", "title": "cargo test · 12s" }
{ "seq": 44, "type": "activity.upserted", "session_id": 7, "node": { … } }   // 终态（raw_output 落库后）
{ "seq": 45, "type": "activity.removed", "session_id": 7, "activity_id": "…" }
{ "seq": 46, "type": "reply.status", "session_id": 7, "reply_id": 9, "status": "in_progress" }
{ "seq": 47, "type": "refresh", "reason": "ownerless-downgrade" }
```

- 事件按 `seq_global` 单调编号；客户端 `?since_seq=` 恢复；订阅过慢发 `lagged`。
- **TUI 与 Web 消费同一套 wire 事件**。

## 6. 视图生成与实时更新（v3.1 核心）

### 6.1 实时视图 = 渲染增量事件流

```text
工具执行中                   运行时                      展开的客户端（TUI/Web）
   │  Render(Log 增量) ──────► │  内存累积（零写库）        │
   │                           │  广播 detail_delta（200ms）► 同 id Log 卡片实时追加
   │  Render(新 Table) ──────► │  内存累积                 │► 新增 Table 卡片
   │  Title("…5s") ──────────► │  O(1) 列更新 + title_changed ► 标题行刷新
   │  …                        │                          │
   └─ 结束 ──────────────────► │  落库 raw_output + activity.upserted
                               │                          │► 展开区重拉 detail
                               │                          │   = render_human(raw) 权威视图（替换预览）
```

- **实时性**：客户端展开期间，每 ≤200ms（`DETAIL_BROADCAST_MS`）收到一次增量并就地更新——shell run 的输出实时可见。
- **零写库**：所有增量只进内存 + 广播；数据库仅在标题列更新（2s）与终态（1 次）写入。
- **只推给展开者**：detail_delta 只发给展开该 activity 的订阅者，未展开不产生流量。

### 6.2 进行中展开（live detail）

用户中途展开一个**正在执行**的 activity：
- `GET /detail?live=1` → 服务端返回**内存累积视图**（已推 RenderDelta 的合并结果）；
- 同时订阅 `operation.detail_delta` 持续追加；
- 终态后：detail 请求走 `render_human(raw)` 返回权威完整视图，替换本地预览。

### 6.3 终态权威视图（render_human）

- 展开已完成 activity：`GET /detail` → 服务端调用 `render_human(raw)`（无渲染函数则 `fallback_human_view(raw)`）→ ViewBlocks。
- 纯函数、惰性（仅展开时调用一次），结果不落盘（可在内存按 activity_id 短暂缓存）。

### 6.4 一致性保证

- 快照测试断言：`render_human(raw)` 的完整输出 ⊇ 流式 RenderDelta 的累积内容（对同一执行序列）；
- 客户端以终态权威视图为准替换预览，避免工具增量遗漏。

## 7. 实时通道与 API 设计

| 接口 | 方法/路径 | 说明 |
|---|---|---|
| 会话快照 | `GET /api/v1/sessions/{id}` | 全量（activity 树；detail 惰性） |
| 实时事件流 | `GET /api/v1/sessions/{id}/stream?since_seq=&kinds=` | SSE；重放+实时；Lagged 通知 |
| 双向控制 | `WS /api/v1/sessions/{id}` | 取消/输入/订阅（可选） |
| 展开详情（终态） | `GET /api/v1/sessions/{id}/operations/{activity_id}/detail` | 调 `render_human(raw)`（或 fallback） |
| 展开详情（进行中） | `GET …/detail?live=1` | 返回内存累积视图 + 订阅 detail_delta |
| 实时详情增量 | 事件 `operation.detail_delta`（mode/block_id/view） | 展开时订阅，实时推送（内存） |
| 发消息 | `POST /api/v1/sessions/{id}/replies` | 现有 |
| 工具权限回复 | `POST /api/v1/sessions/{id}/permission/reply` | 现有 |

**客户端订阅协议（简单版）**：
1. 打开会话 → `GET snapshot` 渲染初始；
2. 同时 `GET stream?since_seq=<快照seq>` 订阅；
3. 展开 activity：
   - 进行中 → `detail?live=1` 拉当前累积视图，之后按 `detail_delta` 的 mode/block_id 就地更新；
   - 已完成 → `detail` 拉 `render_human(raw)` 权威视图；
4. 收到 `activity.upserted`（终态）→ 节点替换为 raw_output；展开区重拉 detail；
5. 收到 `lagged` → 全量刷新。

## 8. 持久化策略：实时 ≠ 无限写库

### 8.1 写放大预算（硬约束）

| 阶段 | 写入 | 频率上限 |
|---|---|---|
| 流式期间（渲染增量） | **零写库**：只进内存 + 广播 detail_delta | — |
| 流式期间（标题） | `UPDATE agena_content_nodes SET title=?`（O(1) 单列） | 2s 一次 |
| 流式期间（摘要，可选） | `UPDATE … SET summary=?`（O(1) 单列） | 2s 一次 |
| 终态 | 一次性 `upsert_content_node`（raw_output 整行） | 每工具一次 |
| 崩溃恢复 | 无（流式内容不落盘；恢复后显示“中断”，可重跑） | — |

- 上限证明：10 分钟长跑 shell，数据库写入 ≈ 300 次标题列更新 + 1 次终态写；输出文本 0 写。

### 8.2 存储收敛

- 保留单表 `agena_content_nodes`；`data` 列 = `RawOutput` 序列化。
- 写路径收敛为两个内部函数：`upsert_content_node`（终态）+ `update_activity_label`（O(1) 列更新）。
- 旧数据宽松反序列化（缺字段默认空，无迁移）。

## 9. 落地路线（推翻重来，但分阶段切换）

| 阶段 | 内容 | 产出 | 验证 |
|---|---|---|---|
| P0 | 统一契约定稿（本文档 v3.1） | 契约文档 | 评审 |
| P1 | 领域收敛：删死变体、`ActivityView`、`RawOutput`、`ViewBlock` | 新 domain 类型 | `cargo test` |
| P2 | 渲染接口：`ToolHumanRenderer` + `RenderDelta`（SDK 兼容） | 协议类型 + IPC | SDK 测试 |
| P3 | 运行时处理器：事件路由、内存累积（含 live detail）、有界持久化、渲染调度（render_human/fallback）、for_model | `ActivityHandler` | 单测 + 写放大断言 + 增量累积测试 |
| P4 | 实时通道统一：SSE 事件形状（detail_delta 带 mode/block_id）+ Web 消费 | Web 实时展开 | 端到端 shell 实时测试（≤200ms） |
| P5 | TUI 切统一事件（删三套并行事件路径；live detail 接入） | TUI 统一消费 | 渲染快照 |
| P6 | 渲染迁移：`operation_blocks_from_tool_output` 块生成下放为各工具 `render_human` + 流式 RenderDelta；未迁移前 fallback 保持现状 | 内置工具自渲染 | 人类视图与现状逐字一致（Golden） |
| P7 | 清理：删除旧投影/双渲染/死代码，8 INSERT 收敛 | 干净架构 | 全量测试 + 黄金快照 |

> P6 是 v3 的关键迁移：**不是删掉美观卡片，而是把它们的“作者”从运行时换成工具自己**（含流式增量），迁移后人类视图与现状逐字一致。

## 10. 保留 vs 废弃清单

**保留**：单表 `content_nodes`；revision_seq 收敛；`seq_global` + Lagged；`normalize_tool_title/summary`；O(1) 列更新；lazy detail REST（扩展 live 模式）；owner 反查（streaming-refactor 分支）；**现状美观卡片逻辑（作为各工具 render_human + RenderDelta 的迁移起点，Golden 不变）**；**200ms detail 广播节流（现状 `DETAIL_BROADCAST_MS`，升级为统一 detail_delta）**。
**废弃/替换**：8 个 INSERT → 1 个 `upsert_content_node`；三套并行渲染标题函数 → `ActivityView`；三套并行事件（CommandOutputDelta/OperationDetailDelta/TranscriptPatch）→ 统一 wire 事件；`operation_blocks_from_tool_output` 运行时 if-match → 各工具 `render_human`；Web 无实时（补齐）；缺 Notice（补齐）；死变体（删除）；v1/v2 的 `human_blocks`/`model_output`/`blocks` 存储（废除）。

## 11. 简单工作流程（端到端示例：shell run，自带 render_human + RenderDelta）

```text
1. 用户 POST /replies ──────────────► 模型返回 tool_call(shell.run "cargo test")
2. 运行时创建 Operation Activity ────► activity.upserted(Pending)
3. 执行开始 ────────────────────────► activity.upserted(InProgress) + reply.status
4. 工具推实时渲染增量（零写库）：
     Render(New, Log{stdout, "编译中…"})      ► detail_delta（200ms 广播）
     Render(Append, Log{stdout, "✓ 完成"})   ► detail_delta
     Title("cargo test · 5s")              ► title_changed（O(1) 列更新）
   ── 用户中途展开 ────────────────────► detail?live=1 拉内存累积 + 订阅增量，实时看到 Log 增长
5. 工具结束 ────────────────────────► ToolActivityResult
     ├─ raw_output（payload + text）→ 一次性 upsert_content_node
     ├─ for_model(raw) → 模型收到 ToolResult
     ├─ 展开区重拉 detail → render_human(raw) → 完整 Command 卡片（权威视图，替换预览）
     └─ 广播 activity.upserted(Completed)
6. 数据库：仅标题列更新 + 1 次终态写；raw_output 一份；实时视图是增量事件，终态视图是 render_human。

若工具未提供 render_human：展开时 fallback_human_view(raw) = 直接渲染原始输出（与给模型同源），仍实时（流式 Render 增量若缺省则由运行时把 text 切成 detail_delta）。
```

## 12. 风险与对策

| 风险 | 对策 |
|---|---|
| 渲染函数有副作用/不确定 | 契约要求确定性纯函数；ctx 限定受控资源；渲染惰性（仅展开/detail 时调用） |
| 插件 render hook 的 IPC 开销 | 渲染惰性 + 内存缓存 ViewBlocks（不落盘）；流式增量走事件（不触发渲染调用） |
| 流式预览与终态权威视图不一致 | 工具负责一致（增量 = render_human 的实时切片）；快照测试断言“增量累积 ⊆ render_human 输出” |
| 增量事件风暴（高频输出） | 200ms 节流 + 只推给展开者 + 订阅队列有界 + lagged 兜底 |
| 迁移期人类视图退化 | P6 逐步下放；未迁移工具 fallback 保持现状输出 |
| 渲染函数执行崩溃 | 捕获错误 → fallback_human_view(raw) |
| 工具想给模型“脱敏/截断” | 通过 raw_output 维度配比（payload 精简 + text 详细） |
| 分阶段切换新旧事件并存 | P4 前旧路径为 fallback；P5 原子切换；统一 wire 事件是唯一前进契约 |

## 13. 验收标准

1. **单一事实源**：任何 activity 持久化内容只有一份 `raw_output`（data 列）；代码中不存在人类/AI 副本字段；
2. **渲染归属**：所有内置工具人类视图由各工具 `render_human` 生成；运行时无“按工具名猜块”分支；无渲染函数时 fallback 显示原始输出（与模型同源）；
3. **实时**：`shell run` 长任务，TUI 与 Web 展开 activity 均实时看到输出（≤200ms），中途展开也能看到已累积内容（live detail）；
4. **写库有界**：流式期间 DB 写入 = 标题/摘要列更新（≤0.5 次/s），输出文本 0 写；有测试断言；
5. **统一**：TUI/Web 同一套 wire 事件；渲染只有一个 `ViewBlock` 渲染器；状态映射一个函数；
6. **兼容**：P6 迁移后各工具人类视图与现状逐字一致（Golden Invariants）；旧插件 fallback 可读；旧数据无迁移；
7. **一致性**：流式 RenderDelta 累积 ⊆ `render_human(raw)` 输出，快照测试通过；
8. 全量测试绿 + 黄金快照通过。
