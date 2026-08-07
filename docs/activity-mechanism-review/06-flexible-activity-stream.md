> **v2 修订（重要）**：本文中 `human_blocks`/`model_output` 作为终态显式字段的设计，在总设计 v2 中已废除——
> 数据只存一份 canonical（`payload`/`blocks`/`text`），给 AI 看与给人看是同一份数据的两个纯函数投影（`for_model`/`for_human`），
> 视图永不落盘。详见 `07-comprehensive-redesign.md` v2 §2（核心原则）与 §6（投影器）。

# 06 设计：更灵活的 Activity 体系 —— 工具活动事件流与三层内容显式化

> 状态：设计提案（未实施）｜适用分支：`agent/activity-mechanism-review`（实施时另开分支）
> 目标：让**工具**能够实时输出良好的标题与展开内容，并能**显式声明**“给人看的内容”与“给 AI 的原文”；
> 同时保持 `05-refactor-plan.md` 的 Golden Invariants —— 不声明新能力的工具，行为与现状逐字一致。

## 1. 现状能力矩阵（已逐行验证）

| 能力 | 流式期间（工具可控制） | 结束时（工具可控制） | 位置 |
|---|---|---|---|
| 标题 | ❌ 只能运行时自动 `{base} · Ns`（`refresh_streaming_title`，`replies_execution.rs:2779`） | ✅ `ToolStreamEnd.title`（经 `compose_tool_title(name, title)` 组合，`replies_execution.rs:2992`） | |
| 展开内容（纯文本） | ✅ `text_delta` → in-memory `CommandOutputDelta` 广播（不持久化，`replies_execution.rs:2730`） | ✅ `output_text` + payload | |
| 展开内容（结构化块） | ❌ 不能 | ⚠️ 只能**隐式**由 payload 形状派生（shell/apply_patch/glob/read/cron/tool_search/lsp/web，`helpers.rs:153 operation_blocks_from_tool_output`） | |
| 摘要 | ❌ | ✅ `ToolStreamEnd.summary`（`normalize_tool_summary`） | |
| 给 AI 的内容 | ❌（模型执行期间不消费） | ⚠️ **隐式**投影：structured JSON（web/crawl）→ generic JSON → `output_text`（`wire_message.rs:630 project_operation_output`）；工具无法分别指定“人类 vs AI” | |
| sections（折叠区） | ❌ | ✅ `ToolStreamEnd.sections` | |
| 附件 | ❌ | ✅ `ToolStreamEnd.attachments` | |

**流式通道现状**：`ToolStreamChunk = { text_delta?, metadata }`（`agena-plugin-sdk/src/hooks/tool.rs:334`），只有纯文本增量。

## 2. 设计目标与原则

1. **工具拥有表达力**：工具在生命周期内可实时发布“标题 / 展开内容（文本或结构化块）/ 摘要”，可显式声明终态三层内容。
2. **显式优先、隐式兜底**：工具声明了什么就用什么；没声明的字段走现状推导（Golden Invariants）。
3. **单一数据源、单表不变**：继续 compact `data` 持久化，只做向后兼容扩展；模型/人类视图在投影时派生，不双写。
4. **三端复用同一渲染契约**：显式块复用现有 `OperationBlock` 渲染器（TUI/Web 已支持）。
5. **不改变模型输入除非工具显式要求**：给 AI 的内容默认保持现状投影；工具显式提供 `model_output` 才替换（属产品决策，单独评审）。

## 3. 核心设计 A：工具活动事件流（ToolActivityEvent）

把 `ToolStreamChunk` 从“只有文本增量”扩展为**有序事件流**。定义在 `agena-tool`（provider-independent，与 `ToolPresentationSection` 同级）：

```rust
pub enum ToolActivityEvent {
    /// 实时标题。工具接管后运行时不再自动追加 ` · Ns`。
    Title(String),
    /// 可选：追加状态后缀（工具自己控制，如 ` · scanning`、` · 3/5`）。
    TitleSuffix(String),
    /// 展开内容纯文本增量（= 现有 text_delta，向后兼容）。
    TextDelta(String),
    /// 展开内容结构化块增量（新能力）。
    Block(ActivityBlock),
    /// 实时摘要。
    Summary(String),
    /// 折叠区（结束时聚合为 sections）。
    Section(ToolPresentationSection),
    /// 给 AI 的原文增量（缺省时 = 累积 TextDelta，兼容现状）。
    ModelDelta(String),
    /// 附件。
    Attachment(AttachmentItem),
    /// 元数据。
    Metadata { key: String, value: String },
}
```

`ActivityBlock` 是对现有 `OperationBlock` 的**工具可声明契约**（映射到同一渲染器）：

```rust
pub enum ActivityBlock {
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
```

## 4. 核心设计 B：三层内容的终态显式化

工具结束时除现有 `ToolStreamEnd` 外，新增显式声明（全部可选）：

```rust
pub struct ToolActivityResult {
    /// 标签。缺省 → 现状：compose_tool_title(name, title) / 失败裸工具名。
    pub title: Option<String>,
    pub summary: Option<String>,
    /// 人类视图显式块。缺省 → 现状：operation_blocks_from_tool_output 隐式派生。
    pub human_blocks: Option<Vec<ActivityBlock>>,
    pub sections: Vec<ToolPresentationSection>,
    /// 给 AI 的原文。缺省 → 现状：structured JSON → generic JSON → output_text。
    pub model_output: Option<ModelOutput>,
    /// 结构化持久化 payload（现状 data 列内容）。
    pub payload: Option<serde_json::Value>,
    pub attachments: Vec<AttachmentItem>,
}

pub enum ModelOutput {
    Text(String),
    Structured(serde_json::Value),
}
```

三层内容模型（对齐 `activity-streaming-refactor.md` 的术语，并补足“显式”能力）：

| 层 | 语义 | 现状（隐式） | 新（显式） |
|---|---|---|---|
| AI 输入 | 模型发出的调用 | invocation（已有） | 不变 |
| AI 结果 | 模型读到的原文 | `project_operation_output` 投影 | `model_output` 显式声明 |
| 人类结果 | 用户看到的展开 | blocks 隐式派生 + markdown 渲染 | `human_blocks` 显式声明 |
| 标签 | 列表/标题行 | title/summary 现状 | `title/summary` 显式声明 + 流式实时更新 |

## 5. 运行时处理（事件路由）

```text
工具事件流 ──► 运行时（agena-runtime-session）
                ├─ Title          → operation.set_title + update_part_title（O(1) 列更新）+ header checkpoint + 广播
                ├─ TitleSuffix    → 同上（追加到当前标题）
                ├─ TextDelta      → 内存累积 human text（现有 CommandOutputDelta 广播，不持久化）
                ├─ Block          → 内存累积 live blocks + 广播结构化块事件（新增 EventKind::ActivityBlockDelta）
                ├─ Summary        → update_part_summary（O(1) 列更新，新增）+ checkpoint
                ├─ ModelDelta     → 内存累积 model preview 文本
                ├─ Section        → 内存累积 sections
                ├─ Attachment     → 内存累积附件
                └─ Metadata       → operation.metadata

终态组装（ToolStreamEnd / ToolActivityResult）──► OperationCompletion
                ├─ title/summary：显式优先；否则现状推导
                ├─ human blocks：显式优先；否则 operation_blocks_from_tool_output
                ├─ model_preview：显式 model_output 优先；否则 output_text（现状）
                └─ data（持久化）：payload + 新增可选字段（见 §6）
```

**标题实时更新的语义**：工具发过 `Title`/`TitleSuffix` 后，运行时对本次执行**停用自动 `· Ns` 刷新**（工具接管）；未发过则保留现状自动刷新（Golden）。

## 6. 持久化（单表不变，data 向后兼容扩展）

现状 `data` 列 = `ToolOutput` payload（compact）。扩展为同一 JSON 内的可选字段（serde `default` + `skip_serializing_if`，旧数据无需迁移）：

```json
{
  "payload": {},
  "managed_outputs": [],
  "truncated": false,
  "human_blocks": [],       // 新：显式人类块（可选）
  "model_output": null      // 新：显式 AI 原文（可选，{text|structured}）
}
```

- 渲染层（`derive_operation_markdown` / TUI / Web）：`human_blocks` 存在 → 渲染显式块（每块类型有对应 markdown/卡片渲染，复用现有 `OperationBlock` 渲染器）；否则现状 `render_tool_payload_markdown_with_name` 派生（Golden）。
- 模型侧（`wire_message.rs project_operation_output`）：`model_output` 存在 → 直接用；否则现状投影（Golden）。
- **不双写**：模型视图与人类视图都是投影时派生，`data` 仍是唯一持久化源。

## 7. 工具侧 API（作者体验）

### 插件工具（agena-plugin-sdk）
- `ToolStreamChunk` 保持 `text_delta` 兼容，新增可选字段或升级为 tagged `ToolActivityEvent`（wire 序列化需向后兼容：旧插件仍发 `{stream_id, text_delta, metadata}`）。
- 结束帧 `ToolStreamEnd` 增加 `model_output`/`human_blocks` 可选字段。

### 内置工具（Rust）
- 提供 builder：
```rust
let stream = ActivityStreamBuilder::new("shell.run")
    .title("cargo test")                       // 实时标题
    .log(LogStream::Stdout, "...")            // 结构化展开
    .table(columns, rows)
    .finish()
    .with_model_output(ModelOutput::Text(raw))
    .with_human_blocks(vec![...]);
```

## 8. 三端呈现

- **TUI**：流式 `Block` 事件 → 展开区实时渲染块卡片（复用 `operation_render.rs` / `message_render.rs` 现有 block 渲染）；实时 `Title` → header 行刷新（复用 `refresh_streaming_title` 的 checkpoint 广播路径）。
- **Web**：`chatRenderModel.ts` 现有 blocks/`model_output` 渲染模型可复用；新增流式事件订阅时按块类型渲染。
- **API**：`OperationPartResource`/`ToolResultEnvelopeResource` 已有 blocks 字段；新增 `human_blocks` 可选字段的序列化直通。

## 9. 与已有工作的关系（重要）

- `docs/activity-streaming-refactor.md`（worktree-activity-tool-streaming 分支，未合并）已实现：owner 传播、InProgress 立即 checkpoint、人类结果流式（流式 text → human Text block）、TUI 优先渲染 human blocks。**它是本设计的运行时基础**：先合并它，再叠加显式声明协议。
- 该分支的已知边界“流式过程中人类块 = Text block，结束时替换为结构化块”正是本设计的 `Block` 事件要解决的：流式期间即可推结构化块。
- 本设计不推翻 master 的 Golden Invariants：所有“显式”字段缺省时行为与 master 现状一致。

## 10. 落地阶段（每阶段测试全绿、可独立合并）

| 阶段 | 内容 | 风险/注意 |
|---|---|---|
| P1 基础设施 | 合并 worktree-activity-tool-streaming（owner 修复、InProgress、human result 流式） | 先固化 terminal golden；最终内容不变 |
| P2 协议 | `agena-tool` 定义 `ToolActivityEvent`/`ActivityBlock`/`ToolActivityResult`；SDK `ToolStreamChunk` 兼容扩展 | wire 向后兼容 |
| P3 运行时路由 | 事件 → 标题/摘要列更新、live blocks 累积与广播、终态组装（显式优先） | 标题接管语义（`Title` 后停自动秒数）；`agena_streaming_base_title` 保留 |
| P4 持久化 | data 扩展 + 渲染层 `human_blocks` 优先 + 模型侧 `model_output` 优先 | 旧数据无迁移；渲染/模型默认路径不变 |
| P5 三端 | TUI/Web 流式块渲染 + 实时标题 | 复用现有 block 渲染器 |
| P6 工具迁移 | shell/fs/web 等内置工具逐步声明标题/块/模型原文 | `model_output` 改变模型输入 → 单独评审 |

## 11. 兼容性与风险

| 风险 | 对策 |
|---|---|
| 工具声明 `model_output` 改变模型输入，进而改变模型行为 | P6 单独评审；默认保持现状投影；显式声明是 opt-in |
| `Title` 接管后丢秒数（用户依赖耗时信息） | 工具可用 `TitleSuffix` 自行追加；文档示例给出 `{title} · {elapsed}s` 模式 |
| 流式结构化块广播频率过高 | 复用现有批处理节奏（`DELTA_BATCH_MS`/`DETAIL_BROADCAST_MS`）聚合广播 |
| 显式块与隐式派生不一致（工具声明与 payload 不符） | 显式优先；渲染层加调试日志；文档明确“显式是权威” |
| wire 兼容：旧插件只发 text_delta | `ToolStreamChunk` 反序列化对旧形状宽松（缺省字段） |
| 事件类型无限膨胀 | 只保留原子事件（Title/Text/Block/Summary/Section/Attachment/Model/Metadata）；复合需求由组合表达 |

## 12. 验收标准

1. 未声明新能力的工具（含全部现有内置/插件工具）在重构后渲染与模型输入与 master 现状逐字一致（Golden Invariants I1–I4）；
2. 新能力有端到端示例：某工具声明实时标题 + 流式结构化块 + 显式 human/model 内容，三端正确呈现；
3. `cargo test` 全绿；SDK 旧插件兼容测试通过；
4. `data` 列旧数据（无新字段）读取、渲染正常（无迁移）。
