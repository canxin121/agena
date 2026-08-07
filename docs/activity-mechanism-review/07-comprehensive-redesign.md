# 07 总设计：Activity 体系彻底重构（推翻重来）方案 · v3（工具自渲染 + 单一事实源）

> 状态：总设计提案 v3（未实施）｜适用分支：`agent/activity-mechanism-review`（实施时开 `agent/activity-v2`）
> v3 相对 v2 的核心变化：
> **“如何把输出渲染给人看”在函数上下沉给工具本身（渲染函数 render hook）；工具没有渲染函数时，运行时才直接渲染原始输出（也就是给 AI 的内容）作为 fallback。**
> 保留 v2 单一事实源：存储只有一份原始事实；人类视图与 AI 视图都是即时投影/渲染，永不落盘。
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

### P3 渲染职责下沉给工具（v3 核心）
“如何把输出渲染给人类”是**工具的职责**：工具在注册时声明渲染函数；运行时在需要人类视图时调用它。**工具没有渲染函数时，运行时才直接渲染原始输出（即给 AI 的内容）作为 fallback**——保证任何工具都至少可读。

### P4 流式内容不进库
实时 detail 只存在于内存事件 + 客户端本地累积；终态一次性写 raw output。

### P5 显式 = 控制事实与渲染，不是提供副本
工具控制“两方不同”的两条途径：
1. 控制**事实维度**（payload 给模型什么、text 给人类/模型什么）；
2. 控制**渲染函数**（如何把事实变成人类视图）。

## 3. 目标架构总览

```text
┌─────────────┐ ToolActivityEvent 流（渲染增量） ┌──────────────────────────┐
│ 工具执行器   │ ──────────────────────────────► │ 运行时 Activity 处理器      │
│ shell/fs/…  │ ToolActivityResult（终态原始事实） │ agena-runtime-session    │
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
                        │   detail = render_human(raw) 或 fallback(raw)
                        │   流式 = detail_delta 增量事件
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

### 4.2 单一事实源（raw output，v3 比 v2 更小）

```rust
pub struct RawOutput {
    pub payload: Option<serde_json::Value>,   // 机器可读事实（模型投影主源；也是渲染函数输入）
    pub text: String,                         // 文本事实（模型 fallback；也是渲染函数输入）
    pub attachments: Vec<AttachmentItem>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub truncated: bool,
}
```

> v3 变更：**不再存储 blocks**。结构化人类块（Table/Log/Command/…）不再是存储维度，而是渲染函数的**输出**（ViewBlocks，即时计算、不落盘）。

### 4.3 工具渲染函数（v3 核心新增）

```rust
/// 视图描述块：可序列化到 wire，TUI/Web 复用同一渲染器。
pub enum ViewBlock {
    Text { text: String },
    Markdown { text: String },
    Json { value: serde_json::Value },
    Table { columns: Vec<String>, rows: Vec<Vec<serde_json::Value>> },
    Log { stream: LogStream, text: String },
    Command { command: String, cwd: Option<String>, exit_code: Option<i32>, stdout: String, stderr: String },
    FileChanges { changes: Vec<FileChange> },
    Diff { diff: String, language: Option<String> },
    SearchResults { /* … */ },
    Media { mime_type: String, artifact: ArtifactRef },
    Custom { kind: String, schema: serde_json::Value, presentation: BTreeMap<String, String> },
}

/// 工具声明的人类渲染函数（注册时提供）。
pub trait ToolHumanRenderer {
    /// 从原始输出生成人类视图。要求：确定性、无副作用（除读取 ctx 提供的受控资源）。
    fn render_human(&self, ctx: &RenderContext, raw: &RawOutput) -> Result<Vec<ViewBlock>, RenderError>;
}

/// 渲染上下文：workspace 访问、command 提示、live_tail 等（延续现状 RenderContext，`helpers.rs:22`）。
pub struct RenderContext { /* workspace_root、command、read_managed 等 */ }
```

- **内置工具**（Rust）：在工具注册表直接提供 `impl ToolHumanRenderer`（fn）。
- **插件工具**：注册时声明 `render` hook；运行时经 IPC 调用 `hooks/tool.render(raw) -> ViewBlocks`。无 hook = 无渲染函数。
- **fallback**：`render_human` 缺失时，运行时调用 `fallback_human_view(raw)` —— **直接渲染原始输出**（payload JSON 美化 / text 原样），与 `for_model` 看到的内容同源（用户要求的“找不到就渲染给 AI 的原始输出”）。

## 5. 统一事件协议（工具 → 运行时 → 客户端）

### 5.1 工具侧事件流（`agena-tool` 定义，SDK 兼容扩展）

```rust
pub enum ToolActivityEvent {
    Title(String),                      // 实时标题
    TitleSuffix(String),
    TextDelta(String),                  // 渲染增量：展开文本（兼容 text_delta）
    ViewBlock(ViewBlock),               // 渲染增量：结构化块（v3：渲染函数的流式预览）
    Summary(String),
    Section(ToolPresentationSection),
    Attachment(AttachmentItem),
    Metadata { key: String, value: String },
    // 注意：不再有 ModelDelta——模型侧内容只由终态 raw_output 投影。
}

pub struct ToolActivityResult {          // 终态（全部可选，缺省走现状推导）
    pub title: Option<String>,
    pub summary: Option<String>,
    pub raw_output: RawOutput,           // 唯一存储（payload/text/attachments/metadata）
    pub sections: Vec<ToolPresentationSection>,
}
```

> v3 变更：流式事件全部是**渲染增量**（`ViewBlock`/`TextDelta`，直接给客户端实时视图）；终态只落库 `raw_output`；权威人类视图由 `render_human(raw_output)` 即时生成。

### 5.2 运行时 → 客户端 wire 事件（统一形状，SSE/WS 同构）

```json
{ "seq": 42, "type": "operation.detail_delta", "session_id": 7, "activity_id": "…", "view": { "kind": "log", "stream": "stdout", "text": "…" } }
{ "seq": 43, "type": "operation.title_changed", "session_id": 7, "activity_id": "…", "title": "cargo test · 12s" }
{ "seq": 44, "type": "activity.upserted", "session_id": 7, "node": { … } }   // 终态（raw_output 落库后）
{ "seq": 45, "type": "activity.removed", "session_id": 7, "activity_id": "…" }
{ "seq": 46, "type": "reply.status", "session_id": 7, "reply_id": 9, "status": "in_progress" }
{ "seq": 47, "type": "refresh", "reason": "ownerless-downgrade" }
```

- 事件按 `seq_global` 单调编号；客户端 `?since_seq=` 恢复；订阅过慢发 `lagged`。
- **TUI 与 Web 消费同一套 wire 事件**。

## 6. 视图生成：渲染函数优先，原始输出 fallback（v3 核心）

```text
                         ┌───────────── raw_output（一份存储）─────────────┐
                         │  payload（机器事实）     text（文本事实）          │
                         └───────┬───────────────────────────┬───────────┘
                                 │                           │
        for_model(raw)           │      render_human(raw)     │  for_label(raw)
        ────────────┼────────────┴───────────┼────────────────┴──────
                    ▼                        ▼                      ▼
         模型 ToolResult 原文          人类展开视图（ViewBlocks）      标题/摘要列
         payload 优先                 （工具渲染函数输出）           （已持久化）
         → text fallback            无渲染函数时：fallback_human_view(raw)
                                     = 直接渲染原始输出（与 for_model 同源）

规则：
- 人类视图 = 工具.render_human(raw)（有渲染函数时）；否则 fallback = 渲染原始输出（给 AI 的内容）。
- 流式期间：客户端本地累积 detail_delta（渲染增量）；终态后展开时调用 render_human(raw) 生成权威视图，替换本地累积（一致性由工具负责 + 快照测试保证）。
- 模型视图 = for_model(raw)（与人类渲染无关）。
- 标签 = title/summary 列（现状）。
```

## 7. 实时通道与 API 设计

| 接口 | 方法/路径 | 说明 |
|---|---|---|
| 会话快照 | `GET /api/v1/sessions/{id}` | 全量（activity 树；detail 惰性） |
| 实时事件流 | `GET /api/v1/sessions/{id}/stream?since_seq=&kinds=` | SSE；重放+实时；Lagged 通知 |
| 双向控制 | `WS /api/v1/sessions/{id}` | 取消/输入/订阅（可选） |
| 展开详情（lazy） | `GET /api/v1/sessions/{id}/operations/{activity_id}/detail` | 展开时调用 `render_human(raw)`（或 fallback）生成 ViewBlocks |
| 实时详情增量 | 事件 `operation.detail_delta` | 展开时订阅，实时推送（内存） |
| 发消息 | `POST /api/v1/sessions/{id}/replies` | 现有 |
| 工具权限回复 | `POST /api/v1/sessions/{id}/permission/reply` | 现有 |

**客户端订阅协议（简单版）**：
1. 打开会话 → `GET snapshot` 渲染初始；
2. 同时 `GET stream?since_seq=<快照seq>` 订阅；
3. 收到 `operation.detail_delta` 且 activity 展开 → 就地追加（Text 追加 / ViewBlock 追加卡片）；
4. 收到 `activity.upserted`（终态）→ 节点替换为 raw_output；展开区重拉 `detail`（render_human 权威视图）；
5. 收到 `lagged` → 全量刷新。

## 8. 持久化策略：实时 ≠ 无限写库

### 8.1 写放大预算（硬约束）

| 阶段 | 写入 | 频率上限 |
|---|---|---|
| 流式期间（渲染增量累积） | **零写库**：只进内存 + 广播 detail_delta | — |
| 流式期间（标题） | `UPDATE agena_content_nodes SET title=?`（O(1) 单列） | 2s 一次 |
| 流式期间（摘要，可选） | `UPDATE … SET summary=?`（O(1) 单列） | 2s 一次 |
| 终态 | 一次性 `upsert_content_node`（raw_output 整行） | 每工具一次 |
| 崩溃恢复 | 无（流式内容不落盘；恢复后显示“中断”，可重跑） | — |

- 上限证明：10 分钟长跑 shell，数据库写入 ≈ 300 次标题列更新 + 1 次终态写；输出文本 0 写。
- v3 比 v2 更省：raw_output 不含 blocks，data 列更小。

### 8.2 存储收敛

- 保留单表 `agena_content_nodes`；`data` 列 = `RawOutput` 序列化（v2 的 blocks 字段移除）。
- 写路径收敛为两个内部函数：`upsert_content_node`（终态）+ `update_activity_label`（O(1) 列更新）。
- 旧数据宽松反序列化（缺字段默认空，无迁移）。

## 9. 落地路线（推翻重来，但分阶段切换）

| 阶段 | 内容 | 产出 | 验证 |
|---|---|---|---|
| P0 | 统一契约定稿（本文档 v3） | 契约文档 | 评审 |
| P1 | 领域收敛：删死变体、`ActivityView`、`RawOutput`、`ViewBlock` | 新 domain 类型 | `cargo test` |
| P2 | 渲染函数接口：`ToolHumanRenderer` + 插件 render hook（SDK 兼容） | 协议类型 + IPC | SDK 测试 |
| P3 | 运行时处理器：事件路由、内存累积、有界持久化、渲染调度（render_human/fallback）、for_model | `ActivityHandler` | 单测 + 写放大断言 |
| P4 | 实时通道统一：SSE 事件形状 + Web 消费 detail_delta | Web 实时展开 | 端到端 shell 实时测试 |
| P5 | TUI 切统一事件（删三套并行事件路径） | TUI 统一消费 | 渲染快照 |
| P6 | **渲染迁移**：把现状 `operation_blocks_from_tool_output` 的块生成（shell/apply_patch/glob/read/cron/tool_search/lsp/web）**下放为各工具自己的 `render_human`**；未迁移前 fallback 保持现状 | 内置工具自渲染 | 人类视图与现状逐字一致（Golden） |
| P7 | 清理：删除旧投影/双渲染/死代码，8 INSERT 收敛 | 干净架构 | 全量测试 + 黄金快照 |

> P6 是 v3 的关键迁移：**不是删掉美观卡片，而是把它们的“作者”从运行时换成工具自己**。迁移完成后运行时不再维护任何工具渲染约定。

## 10. 保留 vs 废弃清单

**保留**：单表 `content_nodes`；revision_seq 收敛；`seq_global` + Lagged；`normalize_tool_title/summary`；O(1) 列更新；lazy detail REST；owner 反查（streaming-refactor 分支）；**现状美观卡片逻辑（作为各工具的 render_human 迁移起点，Golden 不变）**。
**废弃/替换**：8 个 INSERT → 1 个 `upsert_content_node`；三套并行渲染标题函数 → `ActivityView`；三套并行事件 → 统一 wire 事件；`operation_blocks_from_tool_output` 的运行时 if-match → 各工具 `render_human`；Web 无实时（补齐）；缺 Notice（补齐）；死变体（删除）；v1/v2 的 `human_blocks`/`model_output`/`blocks` 存储（废除）。

## 11. 简单工作流程（端到端示例：shell run，工具自带 render_human）

```text
1. 用户 POST /replies ──────────────► 模型返回 tool_call(shell.run "cargo test")
2. 运行时创建 Operation Activity ────► 广播 activity.upserted(Pending)
3. 执行开始 ────────────────────────► activity.upserted(InProgress) + reply.status
4. 工具推渲染增量（内存，零写库）：
     Title("cargo test")            ► operation.title_changed（O(1) 列更新，2s 节流）
     ViewBlock(Log{stdout,…})       ► operation.detail_delta（200ms 内存广播）
   ── 用户展开 ────────────────────► TUI/Web 实时看到 Log 卡片逐条出现
5. 工具结束 ────────────────────────► 组装 ToolActivityResult
     ├─ raw_output（payload + text）→ 一次性 upsert_content_node（终态）
     ├─ for_model(raw) → 模型收到 ToolResult
     ├─ 展开时 render_human(raw) → shell 的 Command 卡片（权威视图，替换流式预览）
     └─ 广播 activity.upserted(Completed)
6. 数据库：仅标题列更新 + 1 次终态写；raw_output 只有一份；人类/AI 视图都是即时生成。

若工具未提供 render_human（如旧插件）：展开时 fallback_human_view(raw) = 直接渲染原始输出
（payload JSON / text），与给模型的内容同源——用户永远看得到可读输出。
```

## 12. 风险与对策

| 风险 | 对策 |
|---|---|
| 渲染函数有副作用/不确定 | 契约要求确定性纯函数；ctx 限定受控资源（workspace 读取）；渲染在展开/detail 时惰性调用 |
| 插件 render hook 的 IPC 开销 | 渲染惰性（仅展开/detail 时调用一次）；ViewBlocks 可缓存于内存（不落盘） |
| 流式预览与终态权威视图不一致 | 一致性由工具负责；快照测试断言“渲染增量累积 == render_human(raw)” |
| 迁移期人类视图退化 | P6 逐步把现状块生成下放为各工具 render_human；未迁移工具 fallback 保持现状输出 |
| 渲染函数执行崩溃 | 捕获错误 → fallback_human_view(raw)（原始输出渲染） |
| 工具想给模型“脱敏/截断” | 通过 raw_output 维度配比（payload 精简 + text 详细） |
| 分阶段切换新旧事件并存 | P4 前旧路径为 fallback；P5 原子切换；统一 wire 事件是唯一前进契约 |
| 内存广播背压 | 队列有界 + lagged + detail 只发给展开该 activity 的订阅者 |

## 13. 验收标准

1. **单一事实源**：任何 activity 持久化内容只有一份 `raw_output`（data 列）；代码中不存在人类/AI 副本字段；
2. **渲染归属**：所有内置工具的人类视图由各工具自己的 `render_human` 生成；运行时没有“按工具名猜块”的分支；无渲染函数时 fallback 显示原始输出（与模型同源）；
3. **实时**：`shell run` 长任务，TUI 与 Web 展开 activity 均实时看到输出（≤200ms）；
4. **写库有界**：流式期间 DB 写入 = 标题/摘要列更新（≤0.5 次/s），输出文本 0 写；有测试断言；
5. **统一**：TUI/Web 同一套 wire 事件；渲染只有一个 `ViewBlock` 渲染器；状态映射一个函数；
6. **兼容**：P6 迁移后各工具人类视图与现状逐字一致（Golden Invariants）；旧插件 fallback 可读；旧数据无迁移；
7. 全量测试绿 + 黄金快照通过。
