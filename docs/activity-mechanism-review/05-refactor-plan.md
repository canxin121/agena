# 05 重构规划：在“标题与内容不变”约束下的分层重构方案

> 状态：规划（未实施） ｜ 适用分支：`agent/activity-mechanism-review`（或后续实施分支）
> 本规划把 04 的路线图收紧为可执行步骤，并以一条硬约束贯穿：
> **工具调用 activity 的标题/摘要/内容，其最终呈现必须与现状字节级一致（Golden Invariants）**。

## 0. 为什么必须有这条约束（现状机制全景）

工具调用 activity 的标题与内容不是渲染层“推导”出来的，而是**运行时与工具在调用生命周期中逐段产出、经统一规范化后持久化、三端只消费不重算**的。

### 0.1 标题（title）生产链

| 阶段 | 生产点 | 产出 | 位置 |
|---|---|---|---|
| 占位（流式早期） | `tool_execution_title(name)` | 工具名 | `processor/tool_call_helpers.rs:5` |
| 流式名称到达 | `operation.set_title(tool_execution_title(name))` | 工具名（即时更新，避免空白标题） | `processor/tool_calls.rs:162` |
| 完整 invocation | `tool_execution_title_for_invocation` = `compose_tool_title(name, invocation_call_summary(input))` | `"fs.read · README.md"`、`"tools.search · Search tools · filesystem"` | `tool_call_helpers.rs:13` |
| provider native | `provider_native_tool_execution_title(title, name, input)` | provider 提供的标题或参数摘要 | `tool_call_helpers.rs:19` |
| 流式刷新 | `refresh_streaming_title` → `{base_title} · {elapsed}s` | 追加耗时后缀，base_title 存 `metadata["agena_streaming_base_title"]` | `manager/replies/replies_execution.rs:2779` |
| 完成 | `compose_tool_title(name, execution.summary().title)`（工具结果标题） | `"shell.run · cargo test"` 风格 | `replies_execution.rs:2992` |
| 失败/非执行/授权 | `terminal_operation_title` | 裸工具名（`shell.run`） | `manager/replies.rs:164` |

**统一规范化**：所有写入标题的构造入口（`OperationPart::pending/completed`、`tool.rs` 各 setter、`ToolExecutionResult`、插件 hooks）都经过 `agena_tool::normalize_tool_title` —— 空白折叠、强制单行、96 显示宽度（Unicode 宽度感知 + grapheme 截断 + 省略号）。摘要同理（`normalize_tool_summary`，120 宽）。工具负责摘要的“含义”，运行时绝不从 `output_text` 推导标题/摘要（`agena-tool/src/lib.rs` 注释明确）。

### 0.2 持久化

- 标题/摘要随 `OperationActivity` 原样入库：`session/history/store/mod.rs:1169 activity_payload` 的 Operation 分支逐字复制 `operation.title/summary`；`data` 只存 compact `ToolOutput` payload；`markdown` 字段恒为空（不持久化）。
- 流式标题刷新走 **O(1) 列更新**：`update_part_title`（`session/store/history.rs:305`）只 `UPDATE agena_content_nodes SET title=?` + `UPDATE agena_model_message_parts SET name=?`，**绝不重写 content/payload**。
- 内容块/详情：`OperationBlock`（Text/Markdown/Json/Table/Log/Command…）由工具输出经 `operation_blocks_from_tool_output` 产生（`replies_execution.rs:2989`）；human 详情 markdown 在快照加载 / lazy detail 时才由唯一入口 `render_tool_payload_markdown_with_name`（经 `derive_operation_markdown`，`manager/helpers.rs:22`）从 compact data 派生。

### 0.3 三端呈现（只消费，不推导）

| 端 | 函数 | 行为 |
|---|---|---|
| TUI snapshot | `operation_activity_title`（`snapshot.rs:607`） | 读 `operation.title`；空则 `invocation.name` |
| TUI API 资源 | `tool_display_label`（`renderer/transcript_text.rs:118`） | 读 `tool.title`；空则 `invocation.name` → 通用 invocation 标签 |
| Web | `activityTitle`（`packages/agena-web-ui/.../chatRenderModel.ts:33`） | 读 `payload.title`；空则 `invocation.name` |
| Web 摘要 | `activitySummary`（同文件 :54） | 读 `payload.summary` 或 `problem.user.fallback` |

三端对 operation 标题**都不重新推导**，这是“标题不变”能成立的现状基础。

## 1. Golden Invariants（重构必须保持的不变量）

### I1 标题字节级不变
同一会话、同一工具调用序列，重构前后：
- 调用开始标题（`name · summary` 组合）逐字一致；
- 流式刷新标题（`{base} · Ns`）逐字一致（`agena_streaming_base_title` 机制必须保留）；
- 完成标题（工具结果 title 组合 / fallback 裸工具名）逐字一致；
- 失败/非执行/授权阶段标题（裸工具名）逐字一致。

**禁止项**：任何新领域方法/渲染层方法不得从 `data` 推导标题；`title()` 的语义只能是“读已持久化的字段”。
**禁止改动**：`invocation_call_summary` 的 PREFERRED_KEYS 顺序与取值、`compose_tool_title` 的分隔符 `" · "` 与 fallback 规则、`normalize_tool_title` 的宽度/截断/省略号逻辑。

### I2 摘要字节级不变
`summary` 只能来自工具结果 / `failure.user.fallback` / `output_text` fallback，经 `normalize_tool_summary`；呈现层不推导。

### I3 内容字节级不变
- 快照/API 的 `blocks`、`sections`、`model_preview` 与重构前一致；
- detail markdown 只能经 `render_tool_payload_markdown_with_name` 单一入口派生，重构不得改变其输出（含 `RenderContext` 的默认值）；
- `update_part_title` 保持“只更新列、不重写 payload”的 O(1) 契约。

### I4 契约不变
- `ExecutionStatus ↔ ActivityState ↔ wire 状态字符串` 集中映射时枚举/字符串值不变；
- API 字段名、`activity_type` 取值不变（wire 契约）。

## 2. 每个重构动作的“变与不变”分析

| 动作（04 编号） | 对标题/内容的影响 | 约束落地方式 |
|---|---|---|
| A1 删除死变体（SkillExecution/Checklist/Search/FileChanges/NestedTask；RuntimeActivity::Error 未构造） | 无影响：现有会话没有任何 activity 使用这些变体 | 删除即安全；若保留 Custom，其 `presentation` 由 producer 提供、不推导 |
| **A2 收敛 title/summary 呈现推导（最大风险点）** | 三端现在各自实现“读 title，fallback invocation.name”；收敛为领域方法时若从 data 推导则标题变 | 定义 `OperationActivity::display_title()` = `title.trim()` 非空 ? `title` : `invocation.name`（与三端现状逐字一致，**只读字段**）；`display_summary()` 同理；三端改为调用，golden 断言不变 |
| A3 Operation 形态收敛（OperationPart/OperationActivity/OperationPartResource） | 映射函数重写可能丢字段/改值 | 新建 `OperationActivity::from_part(&OperationPart)` 逐字搬运（call_id/invocation/title/summary/data/authorization/error）；`activity_payload` 与 snapshot 投影改为调用它；字段级单元测试断言相等 |
| B1 human-blocks 流式（合并不合并分支均可选项） | **唯一允许的“内容时序”变化**：流式期间 detail 提前到达；最终内容必须不变 | 合并前先固化 terminal 内容 golden；只新增流式中间视图；`apply_tool_success` 最终 blocks/markdown 与现状一致；`agena_streaming_base_title` 不冲突 |
| B2 web `model_output_text` 修复 | 现在读不存在的字段（恒空）；修复是**补全缺失内容**，不是改写标题/摘要 | 只动 `content.model_output`；不碰 `title/summary`；单独提交并在 PR 明示为预期变化 |
| B3 `MessagePartDetailResource` 补 Notice | 新增变体，不影响现有 8 个变体 | 纯增量 |
| C1 文件拆分（activity.rs/history.rs/store 按职责） | 无影响（纯搬移） | 每步跑全量测试 + snapshot 渲染测试 |
| C2 SQL 收敛（8 个 INSERT → 1 个 upsert） | 列错位会改内容 | **保留 `update_part_title` 列更新路径**（O(1) 是性能契约，不能并入会重写 payload 的 upsert）；upsert 的 title/summary/data 列映射与原 INSERT 逐一对照 |
| C3 `ActivityNode::new/transition` 启用 | 替换 struct 字面量时字段错位会改标题/内容 | 字段映射与 history.rs 字面量一致；启用后跑全量测试 + snapshot diff |
| D1 插件 transcript 活动通道 | 新增能力（Custom 生产端） | 不影响现有 |
| D2 `ActivityOwner::Session` 写者 | 新增能力 | 不影响现有 |
| D3 每变体端到端测试 | 建立 golden 断言，防回归 | 见 §3 |
| D4 错误三表示统一 | 错误活动标题也是呈现内容 | 只统一内部处理；`ErrorActivity`/`OperationActivity.error` 的标题、`problem.user` 字段逐字保留 |
| E From/TryFrom、状态映射集中、Box 大 enum | 无影响（值不变） | 转换测试断言逐字相等；映射测试断言字符串不变 |

## 3. 验证策略：Golden Snapshot 与测试闸门

1. **字段级不变量测试（Rust，每步必跑）**
   - `OperationActivity::from_part` 输出 == 手写 `activity_payload` 输出（title/summary/data/invocation/authorization/error 逐字段相等）；
   - `display_title()/display_summary()` == 三端现状函数输出；
   - 状态映射测试断言 wire 字符串不变（`in_progress/completed/failed/cancelled`）。
2. **渲染快照测试（自动闸门）**
   - TUI snapshot 测试已断言大量标题（如 `operation_activity_titles_use_the_composed_operation_title`，`snapshot.rs:2188`）；重构每步必须全绿；
   - 扩展：为流式刷新标题（`base · Ns`）、失败裸工具名标题补专门断言。
3. **DB 级测试**
   - store 测试断言 `agena_content_nodes` 行（title/summary/data 列）与重构前一致；
   - `update_part_title` 测试断言不触碰 content/payload 列。
4. **Web 纯函数测试**
   - 为 `activityTitle/activitySummary`（chatRenderModel.ts）加输入输出快照测试，作为 B2 的守卫。
5. **端到端冒烟（每个里程碑）**
   - 跑一个真实会话（shell.run 流式→完成、fs.read、失败、重试），dump 三端渲染结果，与基线 diff。

## 4. 分步实施（每步可独立合并、测试全绿）

- **Step 0 基线**：写本规划；为“标题/内容不变量”补测试骨架（golden fixtures）——先红后绿。
- **Step 1（A1）**：删除死变体与死代码（`RuntimeActivity::Error`、未使用的 `ActivityNode::new/transition` 若确认无调用）；零行为变化。
- **Step 2（A2）**：新增 `OperationActivity::display_title()/display_summary()`（只读字段）；TUI/Web 三端改为消费；golden 断言与现状一致。
- **Step 3（A3）**：`OperationActivity::from_part`；store/history 映射函数改为调用它；字段级测试。
- **Step 4（C2+C3）**：SQL upsert 收敛（保留 `update_part_title`）；启用 `ActivityNode::new` 替换字面量。
- **Step 5（C1）**：文件拆分（activity.rs/history.rs/store）——纯搬移，逐文件提交。
- **Step 6（B2+B3）**：web `model_output_text` 修复（预期变化，单独提交）；`MessagePartDetailResource` 补 Notice。
- **Step 7（B1，可选）**：合并 human-blocks 流式分支；先固化 terminal 内容 golden。
- **Step 8（D+E）**：插件通道/`ActivityOwner::Session` 写者/每变体端到端测试/From 转换与映射集中。

## 5. 风险与对策

| 风险 | 对策 |
|---|---|
| 领域 `title()` 被误用为“从 data 推导标题” | 语义注释 + code review 检查项；golden 断言钉死 |
| `agena_streaming_base_title` 机制在重构中丢失（标题会变成 `name·Ns·Ns` 叠加） | Step 3/4 保留 metadata 处理；为流式刷新标题加专门断言 |
| SQL 收敛把 `update_part_title` 并入 upsert（重写 payload、破坏 O(1)） | 显式保留两个入口并注释性能契约 |
| human-blocks 流式合并改变最终内容 | 合并前固化 terminal golden，合并后 diff |
| web `model_output_text` 修复改变 web 快照 | 定性为“补全缺失”，单独提交并在 PR 说明 |
| 状态映射集中改坏 wire 值 | 枚举映射测试断言字符串不变 |
| 三端 fallback 行为不一致（title 空时的降级链） | A2 收敛到同一领域方法，行为逐字对齐后替换 |

## 6. 验收标准

1. 上述 Golden Invariants I1–I4 全部有自动化断言且通过；
2. 对同一组代表性会话，重构前后三端渲染 diff 为空（除 B2 的 model_output 补全与 B1 的流式时序）；
3. `cargo test` 全绿；web 相关测试（如新增）全绿；
4. 04 中 A–E 各项按 §2 表逐项完成，且每项都有对应的“不变”证据（测试或 diff）。
