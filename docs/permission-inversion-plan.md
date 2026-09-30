# 权限决策反转到插件侧：删掉契约/预检/能力清单，宿主只给查询 API

> 本文件是 `/Users/canxin/.claude/plans/elegant-juggling-lemon.md` 的实现期副本，
> 防止 plan 文件被其它会话覆盖。以本副本为准。

## Context

用户原话：

> 我们现在全都让插件去自己定义自己的声明和抽出路径，为什么我们不直接让插件可以拿到我们的权限设置
> 本身，然后自己去判断权限是否满足啥的呢？我们只需要提供一些方便的判断用的帮助函数啥的。这样的话
> 更加方便灵活，并且写起来简单多了。……并且我们改就是直接彻底的修改而不是残留一堆历史兼容和依赖。

用户已通过 AskUserQuestion 选定：

- **决策点**：`插件自决，宿主只给帮助函数`
- **破坏性**：`可以，彻底拆干净`

## 关键决定

1. 删除 `ToolPermissionContract` 及四个 Spec 类型。`PathKind` 保留（权限查询还用）。
2. 删除 `permission_paths` / `permission_networks` hook 及转换 trait。
3. 删除 `#[tool(path(...), network(...))]`、`permission(...)`、`#[input(path=,network=)]` 抽取。
4. ~~`mutating` / `read_only` / `shell` / `interactive` / `task` 五个 bare flag 保留。~~
   **后续修订**：五个 bare flag 已并入 tag（见下）。`#[tool(...)]` 现在只有自述面：
   `tags(...)`。bare 拼写（`shell`、`read_only`、`mutating`、`interactive`、`task`）
   仍可写，但只是 `tags(...)` 的缩写，与 tag 落在同一个列表里。
5. 新增 `check_path_permission` / `check_network_permission` + `require_*` 便捷函数。
6. 不加任何兼容层。
7. 权威自述走 tag（`ToolTag::Shell` / `ToolTag::Task`）。

## 实施清单（见 plan 文件第 1–10 节）

## 分阶段

1. 加查询 API（不改现有结构）。
2. 删插件侧声明面与预检管线。
3. 删下游消费者（ceiling、`ExecutionAccess::ReadOnly`、只读 profile）与文档。

## 阶段 3 落地记录（已完成）

- `ExecutionAccess` 整个枚举删除（不只 `ReadOnly`）：`Inherit` 是它唯一剩下的变体，
  删除后 `SessionExecutionContext.access`、`PersistedExecutionConfig.access`、
  `SubtaskStatusChangedEvent.access`、`SessionSummary.subtask_access`、
  `SessionExecutionContextResource.{execution_access,subtask_access}` 全部消失。
- `TaskAccess` / `RunSubtaskAccess` 与 `TaskToolInput.access` / `RunSubtaskRequest.access`
  一并删除——这条链路的唯一用途就是给子会话设 `ReadOnly`。
- `agena-domain::CapabilitySourceKind::ExecutionAccess` 删除；工具能力集（
  `capability_denied_tool_names`）仍通过 `AgentProfile` 拒绝。
- 保留 `permission_ceiling`（用户明确要求「只保留 ceiling」）：它仍是子会话的非升级边界。
- 插件侧「只读」现在只由 `read_only` tag 与其它 tag 共同自述表达（模型 profile 过滤、
  能力清单 effects、MCP 匿名只读都在读这些 tag）。

## 反转后的效果边界（文档结论）

- 宿主的**自有工具**（executor-backed builtins：`fs.read`/`glob`/`grep`/`apply_patch`/
  `lsp.*`/`shell.*`/`monitor.*`/`cron.*`）仍在会话管线里做路径与网络预检，
  仍可 `Ask`、仍能落持久规则。这是反转后唯一的「正式审批」入口。
- **插件自实现 I/O** 不再被宿主兜底：插件在自己动手之前自愿调用
  `HostClient::check_path_permission` / `check_network_permission`
  （或 `require_*`）拿到只读策略答案。非合作插件（stdio/http/wasm 传输）不受约束——
  这是用户明确选择的代价。
- `Ask` 在这次查询里的语义是「策略没有批准」：查询不能阻塞等人类回答，所以
  `Ask` 与 `Auto` 都映射为「未批准」（见 `mappers.rs::permission_query`）。
- 插件调用宿主工具（`host.invoke_tool`）仍然走会话管线，仍会弹审批。
