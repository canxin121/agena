# 07 总设计：Activity 体系彻底重构（推翻重来）方案 · v2（单一事实源）

> 状态：总设计提案 v2（未实施）｜适用分支：`agent/activity-mechanism-review`（实施时开 `agent/activity-v2`）
> v2 修正：按“**单一事实源（Single Source of Truth）**”原则重写存储与投影。
> **数据只存一份；给 AI 看与给人看是同一份数据的两个纯函数投影；视图永不存储。**
> 硬目标不变：1) 实时性（shell run 展开实时看输出，TUI+Web）；2) 实时 ≠ 无限写库（写放大有预算）。

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

## 2. 核心原则（v2 新增，最高优先级）

### P1 单一事实源
每个 activity 的内容**只存一份 canonical**。不存在“人类副本”和“AI 副本”。

### P2 视图是纯函数投影
“给 AI 看”“给人看”“给标题/摘要”都是**同一份 canonical 的投影**（`for_model` / `for_human` / `for_label`），投影结果**永不落盘**、只在读取/发送时即时计算。

### P3 显式 = 控制事实，不是提供副本
工具想“给人看 A、给模型看 B”，不是提交两份内容，而是**用不同维度的事实表达输出**：
- `payload`（机器可读事实，JSON）→ 模型投影主源；
- `blocks`（结构化事实，ActivityBlock 中立表达）→ 人类投影主源；模型投影也可消费；
- `text`（文本事实）→ 人类展开 + 模型 fallback。
三份都是“事实”，不是“视图”；视图由投影器按消费方生成。

### P4 流式内容不进库
实时 detail 只存在于内存事件 + 客户端本地累积；终态一次性写 canonical。

## 3. 目标架构总览

```text
┌─────────────┐  ToolActivityEvent 流（实时）   ┌──────────────────────────┐
│ 工具执行器   │ ─────────────────────────────► │ 运行时 Activity 处理器      │
│ shell/fs/…  │  ToolActivityResult（终态事实）  │ agena-runtime-session    │
└─────────────┘                                └────────────┬─────────────┘
                                                             │
        ┌────────────────────────────────────────────────────┼─────────────────────────┐
        │ 内存累积 canonical（text/blocks/payload，零写库）     │ 终态一次写 canonical      │ 投影（纯函数，不存储）
        ▼                                                    ▼                          ▼
 ┌──────────────┐ 统一 wire 事件（detail_delta/title_changed/…）  ┌──────────────┐   ┌──────────────┐
 │ 事件流服务    │ ───────────────────────────────────────────► │ 存储（单表）   │   │ for_model     │
 │ seq + Lagged │      ┌──────────────────────┐                 │ content_nodes│   │ for_human     │
 └──────┬───────┘      │ TUI / Web（同一套消费） │                │ canonical    │   │ for_label     │
        └──────────────┤ 实时渲染展开 detail    │                 └──────────────┘   └──────┬───────┘
                       └──────────────────────┘                                             │
                                                                                             ▼
                                                                                  模型读到 ToolResult（投影原文）
```

## 4. 统一活动模型（领域层）

### 4.1 收敛后的 Activity 类型

- `ActivityPayload` 收敛为 **9 个活变体**（删除死变体）：`Operation`、`Resource`、`SkillReference`、`Reasoning`、`TextArtifact`、`TextSegment`、`Interaction`、`Error`、`Notice`。
- 每个变体实现统一视图 trait（**只读已存字段/投影 canonical，绝不另存视图**）：
```rust
pub trait ActivityView {
    fn title(&self) -> &str;              // 标签列（已持久化）
    fn summary(&self) -> &str;            // 标签列（已持久化）
    fn canonical(&self) -> &CanonicalContent; // 单一事实源
}
```

### 4.2 单一事实源（canonical content）

```rust
pub struct CanonicalContent {
    pub payload: Option<serde_json::Value>,   // 机器可读事实（模型投影主源）
    pub blocks: Vec<ActivityBlock>,           // 结构化事实（中立表达；人类投影主源，模型 fallback）
    pub text: String,                         // 文本事实（人类展开 + 模型 fallback）
    pub attachments: Vec<AttachmentItem>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub truncated: bool,
}
```

- **没有** `human_blocks`、`model_output` 字段——它们不是事实，是投影（见 §6）。
- `ActivityBlock` 是唯一结构化块契约：Text/Markdown/Json/Table/Log/Command/FileChanges/Diff/SearchResults/Media/Custom；工具可声明、可持久化、可渲染。

## 5. 统一事件协议（工具 → 运行时 → 客户端）

### 5.1 工具侧事件流（`agena-tool` 定义，SDK 兼容扩展）

```rust
pub enum ToolActivityEvent {
    Title(String),                      // 实时标题（工具接管后停自动秒数）
    TitleSuffix(String),                // 状态后缀（` · scanning`）
    TextDelta(String),                  // 展开文本增量（兼容现有 text_delta）→ 追加 canonical.text
    Block(ActivityBlock),               // 展开结构化块增量（新）→ 追加 canonical.blocks
    Summary(String),                    // 实时摘要
    Section(ToolPresentationSection),
    ModelDelta(String),                 // 给 AI 的原文增量（缺省=累积 TextDelta）→ 影响模型投影的 text
    Attachment(AttachmentItem),
    Metadata { key: String, value: String },
}

pub struct ToolActivityResult {          // 终态事实（全部可选，缺省走现状推导）
    pub title: Option<String>,
    pub summary: Option<String>,
    pub canonical: CanonicalContent,     // 唯一存储：payload/blocks/text/attachments/metadata
    pub sections: Vec<ToolPresentationSection>,
}
```

> v2 变更：`ToolActivityResult` 不再有 `human_blocks`/`model_output` 字段；工具表达输出只通过 `canonical` 的三个维度（payload/blocks/text）。

### 5.2 运行时 → 客户端 wire 事件（统一形状，SSE/WS 同构）

```json
{ "seq": 42, "type": "operation.detail_delta", "session_id": 7, "activity_id": "…", "block": { "kind": "log", "stream": "stdout", "text": "…" } }
{ "seq": 43, "type": "operation.title_changed", "session_id": 7, "activity_id": "…", "title": "cargo test · 12s" }
{ "seq": 44, "type": "activity.upserted", "session_id": 7, "node": { … } }   // 快照增量（终态落库后）
{ "seq": 45, "type": "activity.removed", "session_id": 7, "activity_id": "…" }
{ "seq": 46, "type": "reply.status", "session_id": 7, "reply_id": 9, "status": "in_progress" }
{ "seq": 47, "type": "refresh", "reason": "ownerless-downgrade" }            // 兜底
```

- 事件按 `seq_global` 单调编号；客户端 `?since_seq=` 恢复（先重放持久化事件，再挂实时）；订阅过慢发 `lagged`。
- **TUI 与 Web 消费同一套 wire 事件**，消除三套并行。

## 6. 投影器：同一份数据，两个（三个）视图（v2 核心）

```text
                    ┌───────────────────── canonical（一份存储）─────────────────────┐
                    │  payload（机器事实）   blocks（结构化事实）   text（文本事实）       │
                    └──────────┬──────────────────┬──────────────────┬──────────────┘
                               │                  │                  │
        for_model(canonical)   │   for_human(canonical)  │  for_label(canonical)
        ──────────────┼────────┴──────────┼────────┴──────────┼──
                      ▼                   ▼                   ▼
        模型 ToolResult 原文         人类展开视图（卡片/markdown）   标题行（title/summary）
        payload 优先                blocks 渲染卡片             （已持久化列，非派生）
        → blocks 紧凑序列化          → 无 blocks 时从 payload      （现状 compose_tool_title
        → text fallback            /text 派生 markdown           /normalize 逻辑）

投影规则（纯函数，读取时即时计算，永不落盘）：
- for_model：payload 非空 → 结构化 JSON（现有 project_operation_output 语义）；否则 blocks 紧凑序列化；否则 text。
- for_human：blocks 非空 → 渲染卡片序列；否则现状 render_tool_payload_markdown（从 payload/text 派生）。
- for_label：title/summary 列（现状），不推导。
```

- **不双写、不缓存投影**：同一 canonical，模型侧与人类侧“看起来不一样”，只是两个投影函数的输出差异。
- **工具控制“两方不同”** = 控制 canonical 的维度配比：
  - 想给模型完整 JSON、给人美观卡片 → `payload`（完整 JSON）+ `blocks`（Table/Log 卡片）；
  - 想给模型精简文本、给人详细 → `payload`（精简 JSON）+ `text`（详细）+ `blocks`；
  - 不提供任何显式事实 → 运行时按现状从工具输出推导 canonical（Golden Invariants）。

## 7. 实时通道与 API 设计

| 接口 | 方法/路径 | 说明 |
|---|---|---|
| 会话快照 | `GET /api/v1/sessions/{id}` | 全量（含 activity 树，canonical 投影为视图） |
| 实时事件流 | `GET /api/v1/sessions/{id}/stream?since_seq=&kinds=` | SSE；重放+实时；Lagged 通知 |
| 双向控制 | `WS /api/v1/sessions/{id}` | 取消/输入/订阅（可选，SSE+POST 可替代） |
| 展开详情（lazy） | `GET /api/v1/sessions/{id}/operations/{activity_id}/detail` | 展开时拉全量 detail（for_human 即时计算） |
| 实时详情增量 | 事件 `operation.detail_delta` | 展开时订阅，实时推送（内存） |
| 发消息 | `POST /api/v1/sessions/{id}/replies` | 现有 |
| 工具权限回复 | `POST /api/v1/sessions/{id}/permission/reply` | 现有 |

**客户端订阅协议（简单版）**：
1. 打开会话 → `GET snapshot` 渲染初始；
2. 同时 `GET stream?since_seq=<快照seq>` 订阅；
3. 收到 `operation.detail_delta` 且 activity 展开 → 就地追加（Text 追加 / Block 追加卡片）；
4. 收到 `activity.upserted`（终态）→ 替换该节点（canonical → for_human/for_model 重算）；
5. 收到 `lagged` → 全量刷新。

## 8. 持久化策略：实时 ≠ 无限写库（v2 保持）

### 8.1 写放大预算（硬约束）

| 阶段 | 写入 | 频率上限 |
|---|---|---|
| 流式期间（text/blocks 累积） | **零写库**：只进内存 canonical + 广播 detail_delta | — |
| 流式期间（标题） | `UPDATE agena_content_nodes SET title=?`（O(1) 单列） | 2s 一次（`TITLE_REFRESH_MS`） |
| 流式期间（摘要，可选） | `UPDATE … SET summary=?`（O(1) 单列） | 2s 一次 |
| 终态 | 一次性 `upsert_content_node`（canonical 整行） | 每工具一次 |
| 崩溃恢复 | 无（流式内容不落盘；恢复后显示“中断”，可重跑） | — |

- 上限证明：10 分钟长跑 shell，数据库写入 ≈ 300 次标题列更新（2s×300）+ 1 次终态写；输出文本本身 0 次写。
- 可选进阶：detail checkpoint（每 10s 或每 1MB 阈值）用于超长任务崩溃续显，默认关闭。

### 8.2 存储收敛

- 保留单表 `agena_content_nodes`；`data` 列 = `CanonicalContent` 序列化（v1 的 `human_blocks`/`model_output` 字段移除，绝不存投影）。
- 写路径收敛为两个内部函数：
  - `upsert_content_node(activity, owner, canonical, …)` —— 终态/节点创建（唯一 INSERT/UPDATE 入口）；
  - `update_activity_label(session, activity_id, title?, summary?, state?)` —— 流式期间 O(1) 列更新（唯一列更新入口）。
- 旧数据（v11 之前 compact payload）可无迁移读取：`CanonicalContent` 反序列化对旧形状宽松（缺字段默认空）。

## 9. 落地路线（推翻重来，但分阶段切换）

| 阶段 | 内容 | 产出 | 验证 |
|---|---|---|---|
| P0 | 统一契约定稿（本文档 v2） | 契约文档 | 评审 |
| P1 | 领域收敛：删死变体、`ActivityView`、`CanonicalContent`、`ActivityBlock` | 新 domain 类型 | `cargo test` |
| P2 | 事件协议：`ToolActivityEvent`/`ToolActivityResult`（agena-tool + SDK 兼容） | 协议类型 + wire 序列化 | SDK 旧插件测试 |
| P3 | 运行时处理器：事件路由、内存 canonical 累积、有界持久化、三投影器 | `ActivityHandler` | 单测 + 写放大断言 |
| P4 | 实时通道统一：SSE 事件形状（detail_delta/title_changed）+ Web 消费 | Web 实时展开 | 端到端 shell 实时测试 |
| P5 | TUI 切到统一事件（删 CommandOutputDelta/OperationDetailDelta 特殊路径） | TUI 统一消费 | 渲染快照 |
| P6 | 工具迁移：shell/fs/web 用 canonical 声明事实 | 内置工具新能力 | 端到端 |
| P7 | 清理：删除旧投影/双渲染/死代码，8 INSERT 收敛 | 干净架构 | 全量测试 + 黄金快照 |

每阶段可独立合并，但 P1–P5 接口在 P0 定死，避免中途打补丁。

## 10. 保留 vs 废弃清单

**保留（已验证的好地基）**：单表 `content_nodes`；revision_seq 收敛；`seq_global` 事件流 + Lagged；`normalize_tool_title/summary`；O(1) 列更新思想；lazy detail REST；owner 反查（streaming-refactor 分支）；compact payload 优先原则（canonical.payload 即其延续）。
**废弃/替换**：8 个 INSERT → 1 个 `upsert_content_node`；三套并行渲染标题函数 → `ActivityView`；三套并行事件 → 统一 wire 事件；隐式块派生（降为 for_human 的 fallback）；Web 无实时（补齐）；`MessagePartDetailResource` 缺 Notice（补齐）；死变体（删除）；**v1 草案中的 `human_blocks`/`model_output` 双副本存储（废除，改为投影）**。

## 11. 简单工作流程（端到端示例：shell run）

```text
1. 用户 POST /replies ──────────────► 模型返回 tool_call(shell.run "cargo test")
2. 运行时创建 Operation Activity ────► 广播 activity.upserted(Pending)
3. 执行开始 ────────────────────────► 广播 activity.upserted(InProgress) + reply.status
4. 工具推事件流（全部进内存 canonical，零写库）：
     Title("cargo test")            ► operation.title_changed（O(1) 列更新，2s 节流）
     Block(Log{stdout, …编译…})      ► operation.detail_delta（200ms 内存广播）
     TextDelta(…运行输出…)           ► operation.detail_delta（追加 canonical.text）
   ── 用户展开该 activity ──────────► TUI/Web 实时看到 Log 卡片/文本逐条出现
5. 工具结束 ────────────────────────► 组装 ToolActivityResult
     ├─ canonical（payload + blocks + text）→ 一次性 upsert_content_node（终态）
     ├─ for_model(canonical) → 模型收到 ToolResult（payload 优先）
     ├─ for_human(canonical) → 展开视图（blocks 卡片）
     └─ 广播 activity.upserted(Completed)
6. 数据库：期间仅标题列更新 + 1 次终态写；canonical 只有一份，两方视图都是即时投影。
```

## 12. 风险与对策

| 风险 | 对策 |
|---|---|
| 分阶段切换期间新旧事件并存 | P4 前保留旧路径为 fallback；P5 原子切换 TUI；统一 wire 事件是唯一前进契约 |
| 投影开销（同一 canonical 多次投影） | 投影是纯函数且轻量（JSON/块序列化）；懒 detail 与客户端本地累积避免重复投影 |
| 工具想给模型“脱敏/截断”内容 | 通过 canonical 维度配比（payload 精简 + text 详细）；如需规则化脱敏，加投影参数（非存储字段） |
| 内存广播背压 | 订阅者队列有界 + `lagged` 通知 + detail 只发给展开该 activity 的订阅者 |
| 事件乱序 | `seq_global` 单调 + 客户端按 seq 丢弃旧事件 |
| 长任务崩溃丢流式内容 | 默认接受（显示中断）；可选 detail checkpoint（默认关） |
| 标题自动秒数 vs 工具接管 | 发过 `Title` 即接管；未发保持 `{base}·Ns`（Golden） |

## 13. 验收标准

1. **单一事实源**：任何 activity 的持久化内容只有一个 canonical（data 列）；代码中不存在“人类副本/模型副本”字段；投影函数无副作用、不落盘；
2. **实时**：`shell run` 长任务，TUI 与 Web 展开 activity 均能实时看到输出（≤200ms 延迟）；
3. **写库有界**：流式期间 DB 写入 = 标题/摘要列更新（≤0.5 次/s），输出文本 0 写；有测试断言写放大；
4. **统一**：TUI 与 Web 消费同一套 wire 事件；渲染只有一个 `render_activity_block`；状态映射只有一个函数；
5. **兼容**：未声明新能力的工具渲染与模型输入与现状逐字一致（Golden Invariants I1–I4）；旧插件兼容；旧数据无迁移；
6. 全量测试绿 + 黄金快照通过。
