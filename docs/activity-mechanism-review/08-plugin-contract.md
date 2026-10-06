# 08 插件统一接入契约：声明即接入，运行时自动接线

> **历史研究归档：2026-08-07，基线 `acaeaf76`。** 本文保留当时的调查和设计推演；文中的路径、类型、接口与结论属于该基线，不能直接作为当前实现的契约。2026-10-07 整合时保留原文及分支历史，未将这些旧设计直接应用到运行代码。
>
> 当前代码入口：[Activity runtime](../../crates/agena-runtime/src/activity/mod.rs)、[Session store](../../crates/agena-storage/src/store/mod.rs)、[Plugin tool contracts](../../crates/agena-tool/src/lib.rs)。整合记录见 [本地分支整合记录](../research/local-branch-integration-2026-10-07.md)。

> 状态：设计提案（未实施）｜适用分支：`agent/activity-mechanism-review`（实施时开 `agent/activity-v2`）
> 本卷是 `07-comprehensive-redesign.md`（v3.1）的插件接入专卷。
> **目标：插件作者只需要实现一个 `Tool` trait（执行 + 可选渲染），activity 的创建、生命周期、持久化、广播、投影全部由运行时自动完成——插件零 activity 知识，不需要做任何“控制 activity”的独特事情。**

## 1. 原则

- **P-A 插件零 activity 知识**：插件 API 面不出现 Activity/ActivityId/content_nodes/patch/投影/事件总线。插件只与“工具”概念打交道；运行时是唯一 activity 编排者。
- **P-B 声明式接入**：插件实现 `Tool` trait（必要项：name/schema/execute 或 stream；可选项：render_human/title）。SDK 宏自动生成全部 hook 接线。
- **P-C 缺省即完整**：插件不提供任何可选能力，运行时全部兜底（标题自动、渲染 fallback 原始输出、流式 text 自动切成增量）。
- **P-D 单一 wire 协议**：插件流帧（`ToolStreamItem`）与运行时事件、客户端事件**同构**，SDK 只做翻译，不做语义发明。

## 2. 插件契约（SDK trait）

```rust
// agena-plugin-sdk

/// 工具契约：插件唯一必须实现的面。
pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> String;
    fn input_schema(&self) -> JsonSchema;

    /// 同步执行（默认：不支持 → SDK 报错）。
    fn execute(&self, args: StructuredObject) -> Result<ToolOutput, ToolError> {
        Err(ToolError::unsupported())
    }

    /// 流式执行（默认：把 execute 包装为单元素流——插件实现 stream 或 execute 之一即可）。
    fn stream(&self, args: StructuredObject) -> Result<ToolStream, ToolError> {
        let output = self.execute(args)?;
        Ok(ToolStream::once(output))
    }

    /// 人类渲染（可选）。缺省 = 运行时 fallback 渲染原始输出（即给 AI 的内容）。
    fn render_human(&self, ctx: &RenderContext, raw: &RawOutput) -> Result<Vec<ViewBlock>, RenderError> {
        Err(RenderError::fallback())
    }

    /// 标题（可选）。缺省 = 运行时 compose_tool_title(name, 参数摘要)。
    fn title(&self, args: &StructuredObject) -> Option<String> { None }
}

/// 流式事件：插件流式输出的最小单元。
pub enum ToolStreamItem {
    Render(RenderDelta),                 // 实时渲染增量（New/Append/Replace + ViewBlock）
    Title(String),                       // 实时标题（可选）
    TitleSuffix(String),
    Summary(String),
    Section(ToolPresentationSection),
    Attachment(AttachmentItem),
    Metadata { key: String, value: String },
    End(ToolActivityResult),             // 终态（raw_output）
}

pub struct ToolStream(/* Stream<Item = ToolStreamItem> 或迭代器 */);

/// 渲染增量（与 07 §5.1 同一类型，插件直接使用）。
pub struct RenderDelta { pub block_id: Option<String>, pub mode: DeltaMode, pub view: ViewBlock }

/// 终态（与 07 §5.1 同一类型）。
pub struct ToolActivityResult {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub raw_output: RawOutput,
    pub sections: Vec<ToolPresentationSection>,
}
```

**插件作者的全部代码**（示例：最小同步插件）：

```rust
#[agena_tool]
struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str { "echo" }
    fn description(&self) -> String { "回显文本".into() }
    fn input_schema(&self) -> JsonSchema { json_schema!({ "text": { "type": "string" } }) }

    fn execute(&self, args: StructuredObject) -> Result<ToolOutput, ToolError> {
        let text = args.get_string("text")?;
        Ok(ToolOutput::text(format!("echo: {text}")))
    }
    // 不实现 render_human / title → 运行时自动 fallback / 自动标题
}
```

**示例：流式插件（shell run 实时输出 + 自渲染）**：

```rust
#[agena_tool]
struct ShellRun;

impl Tool for ShellRun {
    fn name(&self) -> &str { "shell.run" }
    fn input_schema(&self) -> JsonSchema { json_schema!({ "command": { "type": "string" } }) }

    fn stream(&self, args: StructuredObject) -> Result<ToolStream, ToolError> {
        let cmd = args.get_string("command")?;
        let (tx, rx) = ToolStream::channel();
        std::thread::spawn(move || {
            let mut child = Command::new("sh").arg("-c").arg(&cmd).stdout(piped()).stderr(piped()).spawn()?;
            for line in read_lines(child.stdout) {
                tx.send(ToolStreamItem::Render(RenderDelta::append("out", ViewBlock::Log { stream: Stdout, text: line })));
            }
            tx.send(ToolStreamItem::End(ToolActivityResult {
                title: Some(format!("{cmd}")),
                summary: Some(format!("exit {}", child.status)),
                raw_output: RawOutput { text: collected, payload: Some(json!({ "exit_code": code })), ..Default::default() },
                sections: vec![],
            }));
        });
        Ok(rx)
    }

    fn render_human(&self, _ctx: &RenderContext, raw: &RawOutput) -> Result<Vec<ViewBlock>, RenderError> {
        // 终态权威视图：完整 Command 卡片
        Ok(vec![ViewBlock::Command { command, exit_code, stdout, stderr }])
    }
}
```

> 注意：插件代码里**没有任何** Activity、事件、持久化、投影概念。`ToolStreamItem::Render` 就是“给人看的实时增量”，`End` 就是“我的输出事实”，其余全部由运行时接线。

## 3. 运行时自动接线表（插件无感）

| 插件声明/行为 | 运行时自动行为 |
|---|---|
| 注册 `Tool`（宏生成 tool.definitions） | 注册进工具注册表 → 模型可见可调用 |
| 模型发起 tool_call | 自动创建 Operation Activity（Pending）→ 广播 activity.upserted |
| 执行开始 | 自动 InProgress 转场 + checkpoint + 广播 reply.status |
| `stream()` 产出 `ToolStreamItem::Render` | 自动内存累积 + 广播 detail_delta（200ms 节流，只给展开者） |
| `ToolStreamItem::Title/Suffix` | 自动 O(1) 列更新 + 广播 title_changed |
| `ToolStreamItem::End(ToolActivityResult)` | 自动落库 raw_output + 广播 activity.upserted(Completed) + `for_model` 投影给模型 |
| `render_human()` | detail 请求时自动调用；无 → fallback_human_view(raw)（渲染原始输出） |
| `title()` | 调用开始/完成自动使用；无 → compose_tool_title |
| 执行失败/被拒 | 自动 ErrorActivity/非执行状态 + 模型收到失败结果 |
| 取消 | 自动 Cancelled 状态 + 广播 |

## 4. 缺省值表（插件什么都不做也完整可用）

| 能力 | 插件提供 | 缺省（运行时自动） |
|---|---|---|
| 执行 | execute 或 stream（SDK 校验至少一个） | — |
| 标题 | `title()` 或 `Title` 事件 | `compose_tool_title(name, 参数摘要)` + 流式 `· Ns` |
| 人类视图 | `render_human()` | `fallback_human_view(raw)` = 直接渲染原始输出（与模型同源） |
| 实时增量 | `Render` 事件 | 运行时把 output_text 按 chunk 切成纯文本 detail_delta |
| 模型内容 | `raw_output.payload/text` | 从输出自动生成（现状投影语义） |
| 摘要 | `Summary` 事件 / result.summary | `normalize_tool_summary(output_text)` |

## 5. SDK 宏自动生成（`#[agena_tool]`）

对 `impl Tool` 自动生成（插件不可见）：
- 工具定义注册帧（tool.definitions）；
- `tool.invoke` / `tool.invoke.stream` hook 的序列化/反序列化（`ToolStreamItem ↔ wire 帧`）；
- `tool.render` hook（若实现了 `render_human`）；
- 生命周期空实现（无 title/render → 直接不注册对应 hook，运行时走缺省）。

**wire 协议（hook 形状，与运行时事件同构）**：

```jsonc
// 运行时 → 插件：调用流式工具
{ "method": "tool.invoke.stream", "params": { "tool": "shell.run", "args": { "command": "cargo test" } } }
// 插件 → 运行时：返回 handle 后推帧
{ "stream_id": "s1", "type": "render", "delta": { "mode": "append", "block_id": "out", "view": { "kind": "log", "stream": "stdout", "text": "…" } } }
{ "stream_id": "s1", "type": "title", "title": "cargo test · 5s" }
{ "stream_id": "s1", "type": "end", "result": { "raw_output": { "text": "…", "payload": {…} }, "title": "…", "summary": "…" } }
// 运行时内部把这些帧翻译为客户端统一 wire 事件：detail_delta / title_changed / activity.upserted
```

## 6. 统一 API 全景（三层同构）

```text
插件层（Tool trait）       运行时层（自动接线）          客户端层（REST + SSE）
─────────────────        ─────────────────────        ─────────────────────
ToolStreamItem::Render ──► 内存累积 + 广播 ──────────► operation.detail_delta
ToolStreamItem::Title ───► O(1) 列更新 + 广播 ────────► operation.title_changed
ToolStreamItem::End ─────► 落库 + 广播 + for_model ───► activity.upserted + 模型 ToolResult
render_human() ─────────► detail 时调用（惰性） ──────► GET …/detail（ViewBlocks）
（无 render_human）──────► fallback_human_view(raw) ──► 同上（原始输出）
```

- **一个概念体系**：`ToolStreamItem`（插件）≡ 运行时事件源 ≡ 客户端 wire 事件（同构，仅翻译）。
- **一个渲染契约**：`ViewBlock` 从插件到 TUI/Web 同一类型，一个渲染器。
- **一个存储**：`RawOutput` 唯一落库。

## 7. 与现状 SDK 的兼容与迁移

| 现状（master） | 新契约 | 迁移 |
|---|---|---|
| `ToolInvokeOutput`（title/summary/output_text/sections/payload/attachments） | `ToolActivityResult` | SDK 兼容层自动转换（output_text→raw_output.text，payload→raw_output.payload） |
| `ToolStreamChunk`（text_delta/metadata） | `ToolStreamItem::Render(TextDelta)` | 兼容层自动包装；**旧插件零改动即获得实时广播** |
| `ToolStreamEnd` | `ToolActivityResult` | 兼容层自动转换 |
| 无渲染 hook | `render_human`（新增可选） | 旧插件无 → fallback 原始输出（行为不退化） |
| `IntoToolInvokeOutput` / `IntoToolStreamEnd` | `Tool` trait + `#[agena_tool]` 宏 | 宏提供默认 impl 保持旧 trait 可用 |

- 旧插件**无需任何改动**即可在新运行时工作：SDK 兼容层把旧返回类型自动翻译为新事件。
- 新插件可用旧 trait 或新 `Tool` trait；新能力（实时增量、自渲染）只能通过新 trait 获得。

## 8. 插件开发者工作流（一句话）

1. 写 `struct` + `impl Tool`（execute 或 stream + 可选 render_human/title）；
2. 加 `#[agena_tool]` 宏；
3. 编译加载 → 运行时自动完成：注册、activity 生命周期、实时广播、终态落库、模型结果、人类视图（自渲染或 fallback）。

## 9. 验收标准

1. **声明即接入**：一个新插件只实现 `Tool` trait（≤50 行）即可获得完整 activity 体验（生命周期、实时展开若 stream、终态视图、模型结果、自动标题），无需任何 activity 概念；
2. **缺省完整**：不实现 render_human/title/stream 的插件：标题自动、人类视图 = 原始输出 fallback、同步执行可用；
3. **旧插件零改动**：现有插件在新运行时行为不退化（实时广播自动获得）；
4. **wire 同构**：`ToolStreamItem` ↔ 运行时事件 ↔ 客户端 wire 事件为同一概念体系，SDK 只翻译不发明；
5. **写放大不变**：流式期间零写库（标题列 2s、终态 1 次）；
6. 全量测试绿 + 黄金快照通过（含一个最小同步插件、一个流式自渲染插件的端到端测试）。
