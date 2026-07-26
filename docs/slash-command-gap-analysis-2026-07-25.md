# Agena Slash Command 缺口与质量审计

> 调研日期：2026-07-25（Asia/Shanghai）
> 范围：Agena 交互式 TUI 的 `/...` 命令；不把模型 Tool、Skill、MCP server 或普通 CLI flag 混入命令数量。
> 方法：以 Agena 当前源码为主证据，交叉阅读仓库内已有调研，以及 Codex、Gemini CLI、Grok Build、OpenCode 和 OpenClaw 的本地源码/文档快照。本文是产品和架构建议，**没有在本次工作中实现任何命令或改变运行时行为**。

## 0. 重新评估后的产品方向（本节覆盖后文的优先级建议）

前两轮列出了大量“可以有入口”的项目，其中不少只是 Settings/Studio/诊断页的深链接。这不符合 Agena 应主打的方向：**UI 负责展示复杂状态、编辑复杂对象和提供确认；slash command 应负责发起或控制一个有生命周期、有身份、可恢复的工作。**

因此，`/plugins`、`/mcp`、`/auth`、`/theme`、`/docs`、`/tools` 等不应成为近期 slash-command roadmap；它们应该由 UI 导航、状态栏和设置页解决。下面的命令才是“输入一句话比点 UI 更有价值”的候选，而且多数已有扎实的底层基础：

```text
Goal（为什么）  →  Plan（怎么做）  →  Task（谁在做）
       \                                  /
        \── Context（还剩多少、卡在哪里）

Schedule（何时再做）       Process（外部命令是否仍在跑）
                \              /
                 Stop（停止当前会话执行）
```

| 推荐命令 | 用户真正控制的对象 | 已有 Agena 基础 | 应以 UI 呈现什么 | 优先级 |
| --- | --- | --- | --- | --- |
| `/stop` | 当前 session 的单一 active execution | `Ctrl+C` 已调用 `request_cancel_run`；生命周期有明确 cancellation 语义。 | 运行状态、取消中/已取消/无运行的确定反馈。 | P0 |
| `/goal` | 当前会话的长期、可验证目标与预算 | `agena.plan` 已保存 `objective`、`phase`、steps、`autorun`；tasks 有 token/cost budget。仍需新增 Goal 的持久状态与验收规则。 | Goal 卡片：目标、状态、预算/已用、证据、阻塞原因、关联 plan/tasks。 | P0 |
| `/plan` | 当前目标的执行计划 | `agena.plan.get/set/update/clear` 已存在，输出含 phase、steps、checks 和 current step。 | 可编辑 Plan board；命令用来 show/draft/pause/resume/clear，而非把复杂步骤硬塞进一行。 | P0 |
| `/task` 与 `/tasks` | 后台子 Agent 的创建、监控、追问、取消和恢复 | `agena.tasks` 已有 create/list/get/output/cancel/message/followup/wait；实现有 task id、session-private recovery、最多 4 个并发子任务、token/cost/timeout budget。 | Task dashboard：状态、agent/profile、预算、输出、取消、追问、follow-up、重启后 interrupted 恢复提示。 | P0 |
| `/schedule`（或 `/cron`） | 定时/一次性恢复工作 | `agena.cron` 已有 list/create/delete/update/pause/resume/history/wakeup。 | Schedule board：下次运行、历史、暂停、删除和受确认的编辑。 | P1 |
| `/processes`（别名 `/ps`） | 当前会话关联的后台 shell 进程 | `agena.shell` 已有 list/logs/stop，且背景进程有稳定 process id/status。 | Process dashboard：命令摘要、日志尾部、运行时长、退出码、停止按钮。 | P1 |
| `/context` | 当前会话可用上下文和实际阻塞面 | `agena.context.status` 已存在；session lifecycle 已区分 workflow、execution、message 三种状态机。 | Context dashboard：budget、compact 建议、Goal/Plan/Task 注入和 active execution，而不是一个短 flash。 | P1 |
| `/research <question>` | 一个可见、可暂停的研究工作流 | 可由 Goal + Plan + 多个 `tasks.create` 组合，但当前没有产品级 orchestration contract。 | 一个工作流卡片及其任务树、来源、阶段性产物和最终综合；不能只是把 prompt 换个前缀。 | P2 |

真正应避免的做法是把 `/goal` 做成“给模型加一句提示词”，或把 `/tasks` 做成“打开一个列表”。它们的价值来自**可持久化、可查看、可暂停/恢复/取消、可审计**。相应地，复杂对象的编辑仍应落到 UI overlay：slash 是动作入口和简洁控制语法，UI 是状态面和确认面。

### 0.1 最小而完整的命令语法

以下是建议的首版，不需要几十个管理命令：

```text
/stop

/goal
/goal <objective> [--budget <tokens>]
/goal status | pause | resume | clear

/plan
/plan draft [instructions]
/plan pause | resume | clear

/task <agent-profile> <objective> [--timeout <duration>] [--budget <tokens>]
/tasks
/tasks show <task-id> | cancel <task-id> | message <task-id> <text>
/tasks followup <task-id> <text> | wait [<task-id>...]

/schedule
/schedule once <duration> <prompt>
/schedule pause | resume | delete <job-id> | history [<job-id>]

/processes
/processes logs <process-id> | stop <process-id>

/context
```

设计约束：

1. `/stop` **只**取消当前 session execution，绝不含糊地停止子任务或 shell process；子任务只能由 `/tasks cancel` 停止，进程只能由 `/processes stop` 停止。
2. `/goal` 置换已有目标时必须显示旧/新目标和关联 plan 的影响，并要求确认；它需要单独的 `active/paused/blocked/completed/cancelled/budget_limited` 状态，而不是滥用 plan 的 phase。
3. `/plan draft` 可以发起模型辅助的计划草案，但“接受草案、开始 autorun、改步骤、改检查项”必须由 Plan UI 完成并留下事件记录。
4. `/task` 创建后应立即返回 Task id/card，**不等待 terminal result**；用户以 `/tasks` 或 UI 持续查看。现有 `agena.tasks.create` 正适合此模式，旧的同步 `tasks.run` 不应成为主交互路径。
5. `/schedule` 的 cron expression、时区和写入 prompt 是复杂/高风险输入，命令应打开结构化表单并在保存前预览下次触发时间。
6. 每一项都必须在 busy 时有清晰语义；只读 dashboard/取消应可用，创建/变更操作则依据单写者规则明确拒绝、排队或转为独立后台 task。

## 结论

Agena 的基础已经不差：TUI 有 **40 个内置 slash command**，支持别名、命令面板、前缀补全和插件命令；会话分支、快照、权限回复、附件、提交和 PR 等日常动作都已可达。重新筛选后，真正的问题不是缺少各类设置入口，而是 **Goal → Plan → Task → Schedule/Process** 这一整条“长期工作控制链”没有成为用户可控、UI 可观察的 command surface。

最应先修复的不是继续堆新命令，而是三件事：

1. **P0：运行中的会话无法执行本应本地执行的 slash command。** `queue_or_submit()` 的注释明确说本地命令“不入队”，但它随后调用 `submit_composer()`；后者在 session busy 时直接恢复草稿并返回。因此 `/allow`、`/deny`、`/status`、`/copy`，以及未来的 `/stop`，在执行中都不能按注释承诺工作。这个结论来自控制流阅读，尚缺回归测试。
2. **P0：Agena 已有 Plan、异步 Task、Cron、后台 shell process 等运行时能力，但没有对应的用户控制命令与 dashboard。** 这是最有价值的新增面，远高于配置/插件页深链接。
3. **P1：高频命令的参数契约过弱。** `/model <name>`、`/agent <profile>`、`/compact [instructions]` 在 Agena 中分别忽略参数、忽略参数、忽略参数；`CommandSpec.arguments` 只是显示字符串，不是可验证/可补全的输入 schema。但它们排在 Goal/Plan/Task 之后。

推荐路线是：先修复本地命令 busy 路径和 `/stop`，接着把现有 Plan/Task/Cron/Process runtime 做成少量强语义命令和 dashboard；之后才完善 `/context`、`/compact`、`/model` 等。不要把所有 Tool 机械变成命令，也不要未经信任地自动执行项目 Markdown 命令。

## 1. 术语、证据等级与边界

| 术语 | 本文含义 |
| --- | --- |
| 内置命令 | [`commands.rs`](../crates/agena-tui-app/src/commands.rs) `COMMANDS` 注册表中的 TUI `/...` 命令。当前为 40 项。 |
| 插件命令 | 插件 manifest 中带 `slash` 元数据的 `PluginCommandDefinition`；经 `command/invoke` 或其 action 路径执行。 |
| 缺少 | **没有 TUI slash 入口**；不声称底层功能、设置页、CLI、Tool 或 SDK 不存在。 |
| 做得不好 | 已有入口但其执行时机、参数、反馈、可发现性或一致性不足。 |
| `P0/P1/P2` | 建议优先级，不是已登记的缺陷等级。P0 阻塞安全/控制闭环；P1 是高频体验或扩展能力；P2 是长期架构与生态。 |

证据优先级为：Agena 可执行源码 > 本仓库内其他产品的源码/官方文档 > 历史调研摘要。外部产品只用于识别成熟模式，不代表应照搬其命令清单。

本审计不建议：

- 把每一个模型 Tool 都变为 slash command；Tool 是模型的可调用能力，命令是用户发起的显式控制面。
- 为了“命令齐全”复制每个竞品的工作流、目标或社交聊天命令。
- 自动加载并执行项目自定义命令中的 shell 片段；必须保留信任、逐次确认、权限和审计语义。
- 用 slash command 绕过已有的审批、插件权限、工作区边界或 OS 安全机制。

## 2. Agena 当前命令表面

### 2.1 内置命令：40 项，而非“只有几个设置入口”

权威表在 [`commands.rs`](../crates/agena-tui-app/src/commands.rs)。解析器只接受独立、去除首尾空白后以 `/` 开头的文本；`//` 作为普通提示词的转义。它将命令名和剩余参数按**首次空白**拆分，随后按命令名/别名匹配。

| 分组 | 命令（别名略去时表示无） | 已有价值 |
| --- | --- | --- |
| 发现 | `/help` (`/?`)、`/commands` (`/palette`) | 上下文帮助与动作面板。 |
| 会话生命周期与导航 | `/new` (`/clear`)、`/sessions`、`/lineage`、`/rewind`、`/rename`、`/timeline`、`/fork`、`/children`、`/parent`、`/continue` | 有会话树、回退、分支和继续执行，覆盖面较好。 |
| 配置/选择/状态 | `/settings` (`/config`)、`/model`、`/agent`、`/status`、`/usage` | 已有选择器和使用统计入口。 |
| 代码交付 | `/review`、`/snapshot`、`/commit`、`/pr`、`/export` | 从审查到提交/PR 已形成一条可用路径。 |
| 记忆与阅读 | `/memory` (`/mem`)、`/pager` | 记忆和长 transcript 浏览有明确入口。 |
| 上下文/队列 | `/compact` (`/compress`、`/summarize`)、`/btw`、`/queue` (`/q`) | 有压缩、旁路会话和队列管理。 |
| 交互权限 | `/user-input` (`/reply`)、`/allow`、`/allow-always`、`/deny`、`/deny-always` | 权限请求可由命令显式处理，是重要基础。 |
| 附件与本地 UI | `/attach` (`/file`)、`/image`、`/download` (`/dl`)、`/editor` (`/edit`)、`/copy`、`/copy-message`、`/copy-visible`、`/diagnostics` (`/feedback`) | 文件、剪贴板、外部编辑和诊断都已覆盖。 |

这是一个值得保留的优点：Agena 并不需要从零再造 `/new`、`/fork`、`/review`、`/commit`、`/pr`、`/memory` 或 `/queue`。后续设计应优先提升语义一致性和可发现性。

### 2.2 从输入到执行的当前路径

```text
Composer 输入 /name args
        |
        +-- COMMANDS 静态注册表 ──> execute_command() ──> TUI 本地 action
        |
        +-- plugin_slash_commands() ──> command/invoke ──> PluginCommandEffect
        |
        +-- 未匹配 ──> 作为普通模型提示词提交
```

相关实现：

- [`commands.rs`](../crates/agena-tui-app/src/commands.rs)：注册、别名、解析和前缀建议。
- [`app_command_actions.rs`](../crates/agena-tui-app/src/app_command_actions.rs)：`execute_command()` 的 40 路本地分发及插件 effect 处理。
- [`app_composer.rs`](../crates/agena-tui-app/src/app_composer.rs)：内置和插件建议合并、补全、接受后提交。
- [`execution.rs`](../crates/agena-tui-app/src/app_session_interactive/execution.rs)：提交、steer、队列和运行中状态。

### 2.3 插件命令：能力已存在，产品采用不足

[`PluginCommandDefinition`](../crates/agena-plugin-sdk/src/manifest.rs) 已提供 `id`、`title`、`description`、`category`、`slash`、`aliases`、`usage`、`location`、可选 `input_schema`、可选 `handler` 和 action。TUI 会过滤与内置命令重名的插件 slash 名，且建议列表能将插件项与内置项一起展示。这说明方向是正确的：内置不必承担所有垂直场景。

不过当前存在三个落差：

1. 建议项没有来源 badge；用户无法一眼知道这是 built-in、哪个 plugin、未来的 Skill 还是 MCP prompt。详情文本可能出现 plugin id，但不是稳定的来源模型。
2. 插件项在 [`app_composer.rs`](../crates/agena-tui-app/src/app_composer.rs) 被固定为 `can_submit_without_arguments: false`。即使命令无参，用户从建议框接受后仍需再次提交，和无参内置命令不同。
3. 虽有 `input_schema`，当前 composer 仍只把一段 `&str` 参数传入插件调用；建议菜单没有利用 schema 做必填校验、枚举/模型/文件补全或预览。

仓库内可直接找到的 bundled slash 例子是 [`/schema-lab`](../crates/agena-bundled-plugins/src/plugins/provided/schema_lab.rs)。这表明插件机制目前更像“已铺好的跑道”，但第一方能力尚未充分借它扩展命令表面。

## 3. 已有命令中做得不好的地方

### 3.1 P0：busy session 与“本地命令永不排队”的实现相矛盾

[`queue_or_submit()`](../crates/agena-tui-app/src/app_session_interactive/execution.rs) 的注释说：`Slash-commands always run locally — never queue.` 它识别本地命令后恢复草稿并调用 `submit_composer()`。但 `submit_composer()` 一开始在 `current_session_activity().is_busy()` 时恢复草稿并立即返回，随后才会解析/分发 slash command。

因此，在 session 忙碌时，下列本应立即作用于本地 UI 或权限状态的命令无法执行：`/allow`、`/allow-always`、`/deny`、`/deny-always`、`/user-input`、`/status`、`/copy*`、`/diagnostics` 等；新增 `/stop` 若复用该路径也会失效。这既破坏了注释承诺，也削弱了高负载时最需要的控制入口。

建议的修复验收标准：

1. 在 busy 检查之前解析并执行被声明为 `local_while_busy` 的命令；普通 prompt 仍遵守 steer/queue 规则。
2. 将“可在 busy 状态执行”变成 `CommandSpec` 的明确 metadata，而非散落在提交函数中的特殊判断。
3. 覆盖至少四类回归测试：权限回复、只读状态命令、`/stop`/取消、会产生新模型 run 的命令（后者应清晰提示不可用或进入既定队列策略）。

### 3.2 参数被展示但未成为契约

| 命令 | 观察到的当前实现 | 问题 | 建议 |
| --- | --- | --- | --- |
| `/model` | 注册表声明无参数；分发时直接 `open_session_model_chooser()`。 | 不能输入/粘贴精确 `provider/model`，脚本化和远程键盘体验弱；也没有 `status`。 | 支持 `/model` picker、`/model list`、`/model <provider/model>`、`/model status`；仍可保留 picker。 |
| `/agent` | 直接打开 agent chooser。 | `/agent <profile>` 不会选择 profile，难以用命令精确复现会话配置。 | 支持 `list`、`status`、`<name>`，并在无匹配时给出候选。 |
| `/compact` | `execute_command()` 调 `compact_current_session()`，完全未消费 `args`；注册表也未声明参数。 | 用户不能指定压缩后必须保留的工作上下文。 | 支持 `/compact [instructions]`；把指令显式进入 compact 请求，失败时保留原 context。 |
| `/help` | 直接 `open_context_help()`，没有消费参数。 | `/help compact`、`/help snapshot` 不能提供精确用法、别名、状态限制、示例。 | 支持 `/help [command]` 与命令详情页；未知名称显示相近匹配。 |
| `/rewind` | 无参数，直接打开 picker。 | 有 UI，但没有可复制的目标、预览和确认式命令契约。 | 保留 picker，同时考虑 `/rewind <checkpoint/message>` 的预览+确认工作流。 |
| `/sessions` | 接受 `[query|all|roots|subtree]`。 | 可搜索但缺精确 `/resume <id>`/`/open <id>` 入口，自动化/文档引用不便。 | 增加可验证的会话 ID/别名恢复命令，不替代现有浏览器。 |

这里的根因是 `CommandSpec.arguments: &'static str` 仅服务显示，`parse_invocation()` 只保留未经结构化解析的尾部文本。少数命令（如 `/snapshot`、`/pr`）各自手写解析，是可工作的短期方案，但会带来不一致的引号、错误提示、补全与可测试性。

### 3.3 建议/帮助/可用性模型太静态

当前 `command_suggestions_for_prefix()` 只按名字和别名前缀过滤内置命令；composer 还会无条件把所有插件命令加入。它没有表达：当前是否有 session、是否 busy、是否有待回复的权限请求、是否有 git 工作区、是否启用插件、是否配置 MCP、命令是否会发起新 run、是否支持附件。

结果是用户可先选择一个当前不可执行的命令，才在 action 中得到 flash 警告。应把以下信息前移到建议和 `/help <command>`：

- 可用性：`requires_session`、`requires_idle`、`requires_pending_permission`、`requires_git`、feature/capability gate。
- 影响：只读、本地 UI、修改会话、发起模型 run、发起外部副作用、需要审批。
- 参数：必填/可选、类型、有效枚举、例子、动态候选。
- 来源：builtin / plugin id / project command / user command / Skill / MCP prompt。

### 3.4 一刀切拒绝附件、结果模型不统一

`submit_composer()` 对任一匹配到的内置或插件 slash command 都会在草稿带附件时提示“不支持附件”。这对 `/attach`、`/image` 这类天然处理本地媒资的命令尤其别扭：它们只能由参数或终端选择器进入，无法声明“此命令接受当前 composer 附件”。应在命令元数据中明确 `input_mode`（无输入、文本、附件、文本+附件），而非以全局拒绝实现安全边界。

同样，内置命令大多以 UI action 或 flash message 结束；插件命令有 `None`、`Message`、`SubmitPrompt`、`OpenRoute`、`OpenUrl` effect。长期应统一结果类型/审计事件，至少记录命令 id、来源、参数摘要、可逆性、审批结果和最终 effect。这样 `/status`、插件命令、未来项目命令才可被同一诊断面观察。

## 4. 对照其他 Agent：值得学习的是模式，不是命令数量

### 4.1 Codex：命令有明确的能力/状态边界

Codex 的 [`slash_command.rs`](../../codex/codex-rs/tui/src/slash_command.rs) 不仅列 enum 和描述，还定义 `supports_inline_args()`、`available_in_side_conversation()` 等状态契约；其 popup 层会按 feature flags 过滤命令。它提供 `/init`、`/diff`、`/skills`、`/mcp`、`/plugins`、`/permissions`、`/status`、`/stop` 等入口。

特别可借鉴两点：

- [`get_git_diff.rs`](../../codex/codex-rs/tui/src/get_git_diff.rs) 将 `/diff` 做成受工作区边界和 Git 安全选项约束的只读本地操作，而不是让模型临时猜一个 shell 命令。
- 协议注释显式区分某些操作（例如手动 compact、review）与“本轮 steer”不能同时发生的情况。这种显式状态契约正是 Agena busy 路径目前缺少的。

不应照搬的部分：Codex 有其自身的 sandbox、远程工作区和服务端 feature flag 体系；Agena 应复用自己的权限/Plugin Host 语义。

### 4.2 Gemini CLI：项目化自定义命令，但执行仍需确认

Gemini 的 [`commands.md`](../../gemini-cli/docs/reference/commands.md) 中 `/init` 生成 `GEMINI.md`，`/mcp` 管理 auth/list/reload，`/commands list|reload` 让自定义命令可见和可刷新。其 [`custom-commands.md`](../../gemini-cli/docs/cli/custom-commands.md) 明确了用户级 `~/.gemini/commands/` 与项目级 `.gemini/commands/`、项目优先级、路径到 namespaced slash 名的映射、`{{args}}` 参数占位及 shell 注入的确认流程。

对 Agena 的启示：

- `/init` 不是“生成一堆文档”，而是一个可审阅的项目指令 bootstrap，生成/更新前应展示 diff 并请求确认。
- 自定义命令要有来源、优先级、reload/诊断和参数替换语义；绝不能把 Markdown 中的 shell 段落默默执行。
- 这类机制适合作为 P2 的“受信项目命令”设计，不应先于 busy 修复和 typed built-ins。

### 4.3 Grok Build：把扩展来源和长期任务讲清楚

Grok Build 的 [slash-command guide](../../grok-build/crates/codegen/xai-grok-pager/docs/user-guide/04-slash-commands.md) 将 `/hooks`、`/plugins`、`/marketplace`、`/skills` 显式映射到扩展 modal，并让 `user-invocable: true` 的 Skill 成为命令。补全行会显示名称、说明、参数提示和来源（builtin、Skill scope 或 plugin）。同一文档还把 `/goal` 与 `/workflow` 的 pause/resume/stop、预算和“非 exactly-once”限制写得很清楚。

对 Agena 的启示：

- `/skills`、`/plugins` 不一定必须做成复杂子命令；先作为现有工作台的深链接也能显著降低发现成本。
- 能把 Skill 暴露为命令，但应是 opt-in：有 `user_invocable`、命名空间与重名优先级、允许工具集、是否直接 deterministic dispatch 等明示 metadata。
- 如果未来加目标/工作流，命令必须准确说明暂停、恢复、预算、跨重启和外部副作用的边界；不能用“已完成”的 UI 文案掩盖不确定性。

### 4.4 OpenCode：在建议列表中标出来源

OpenCode 的 [`en.ts`](../../opencode/packages/app/src/i18n/en.ts) 直接定义了 `custom`、`skill`、`mcp` 三种 slash badge 文案。这是一个小但高价值的 UX：同名或相似命令很多时，用户知道它来自哪里、是否项目特定、是否可审查。Agena 当前将内置和插件混合在建议中，缺少这种稳定的 provenance 展示。

### 4.5 OpenClaw：命令的 fast path、动态参数和安全分类

OpenClaw 的 [slash command 文档](../../cline/openclaw/docs/tools/slash-commands.md) 区分 standalone command、directive、inline shortcut，并为 `/context [list|detail|json]`、`/model <name>`、`/queue`、`/compact [instructions]`、`/stop` 给出明确语义；在支持的聊天平台上还会为动态参数提供自动补全或按钮选择。

其“命令仅在授权 sender 上 fast-path”的部分是多聊天平台的访问控制需求，不能原样搬到单机 TUI。但两项原则适用于 Agena：

1. 本地控制命令必须有不经过模型/队列的明确定义路径，且该路径应在 busy 时真实可用。
2. `model`、`context`、`queue` 等状态命令应返回结构化、可解释的当前状态，而非只打开一个未标注的选择器或短 flash。

### 4.6 既有跨产品调研的边界提醒

本仓库根目录的 [`agent_tools_skills_mcp_2026-07-24.md`](../../agent_tools_skills_mcp_2026-07-24.md) 已明确区分 Tool、Skill、MCP 与 slash command，并记录了 Gemini 的 MCP prompts、Grok 的项目/用户命令目录、Claude 的 `/init`/review 等。本文沿用这条边界：不以“某产品 Tool 多”为理由要求 Agena 增加等量 slash command。

## 5. 宽口径缺口库存（非近期 roadmap）

本节保留完整审计证据，便于以后查找某个入口是否已有产品基础；**它不是优先级列表**。近期实现顺序以 [第 0 节](#0-重新评估后的产品方向本节覆盖后文的优先级建议) 为准。特别是 Settings/Studio 深链接类项目，不应因为“可以做”而挤占 Goal、Plan、Task 的控制面工作。

| 候选 | Agena 当前能力/表面 | 建议 | 优先级 | 理由与验收重点 |
| --- | --- | --- | --- | --- |
| `/stop` 或 `/cancel` | registry 没有 `CommandId::Stop`；已有 Ctrl+C 取消路径不等于可发现的文本命令。 | 新增本地、busy-safe 取消命令。 | P0 | 必须能中止当前 session run，幂等、可审计，不能排队。 |
| busy-safe 本地命令分发 | 注释与 `submit_composer()` busy early return 矛盾。 | 修复执行顺序并加 `local_while_busy` 元数据/测试。 | P0 | 先确保 `/allow`、`/deny`、`/status` 和 `/stop` 在运行中可靠。 |
| `/model [list|status|<id>]` | 有 `/model` chooser；参数被忽略。 | 升级现有命令。 | P1 | 既支持图形选择也支持精确、可复制、可补全的模型选择。 |
| `/agent [list|status|<profile>]` | 有 `/agent` chooser；参数被忽略。 | 升级现有命令。 | P1 | 让 agent profile 可查询、可复现。 |
| `/compact [instructions]` | 有 `/compact`，但只触发默认 compact。 | 升级现有命令。 | P1 | 用户能声明“保留什么”；busy 行为明确。 |
| `/help [command]` | 只有泛帮助。 | 升级现有命令。 | P1 | 展示别名、参数、例子、状态前置条件、副作用和来源。 |
| `/context [detail]` | 有 compact、usage、status；缺用户可读的 context 预算/组成视图。 | 新增只读命令。 | P1 | 显示 token/budget、压缩状态、重要注入来源；不泄露不该展示的内部内容。 |
| `/diff [--staged]` | 有 review/commit/PR，未见直接 worktree diff 命令。 | 新增受边界约束的只读命令。 | P1 | Git 检查、未跟踪文件、输出上限、安全 Git config；可从 `/review` 前快速查看变更。 |
| `/init` | 未见项目指令 bootstrap slash 入口。 | 新增带 diff/确认的引导命令。 | P1 | 生成或改进 Agena 项目指令文件；不可静默覆盖。文件名/优先级先经过产品决策。 |
| `/skills` | Skills 是既有运行时能力，slash 表面没有入口。 | 新增工作台深链接/列表。 | P1 | 先显示发现结果、来源、启用状态、适用条件和错误；再考虑调用。 |
| `/mcp` | MCP 生命周期/状态存在于产品能力，未见 TUI slash 入口。 | 新增状态/管理深链接。 | P1 | 至少 list/detail/reconnect/auth 的可发现入口；认证/重连仍走既有批准流。 |
| `/plugins` | 插件命令机制和 Plugin Host 已存在，未见内置 slash 管理入口。 | 新增工作台深链接/列表。 | P1 | 显示 id、版本、权限、命令、错误和来源，而非只“安装成功”。 |
| `/permissions` | 有 Permission Studio；注册表测试还断言它不是独立命令。 | 作为有意的产品取舍复审。 | P1 | 可保留 `/settings permissions`，但 `/permissions` alias/深链会降低紧急管理时的发现成本。 |
| `/doctor` / `/health` | `/diagnostics` 打开终端诊断；`/status` 只 flash runtime summary。 | 升级或增加聚合健康报告。 | P1 | 汇总 provider/auth/config/MCP/plugin/浏览器/目录边界与可行动修复；不要把原始日志当诊断结论。 |
| `/resume <id>` / `/open <id>` | `/sessions` 有浏览入口。 | 补精确恢复，保留浏览 UI。 | P2 | 与共享的会话 ID、深链接、脚本/文档引用配合。 |
| 自定义项目/用户命令 | 有 plugin command SDK；未见 `.agena/commands` 一类简单项目命令发现。 | P2 设计受信 command manifest。 | P2 | 采用明确来源/优先级/重新加载/审计/确认；不要自动执行任意 Markdown 或 shell。 |
| Skill-to-command bridge | 有 Skill 体系，但 Skill 不会自动出现在 slash catalog。 | P2 opt-in bridge。 | P2 | `user_invocable`、命名空间、冲突规则、工具许可、来源 badge；默认不暴露全部 Skill。 |
| MCP prompt-to-command bridge | MCP 可携带 prompts 的生态模式，但不宜默认自动注册。 | P2、仅明确授权。 | P2 | server/source、参数 schema、信任状态和撤销入口必须可见。 |
| 把每个 Tool 变成命令 | Tool 本质不同，且命令会膨胀。 | 明确不做。 | 非目标 | 只为频繁、可预测、用户主动控制且安全边界清楚的动作加入口。 |

## 6. 非主线命令族的补充审计（保留证据，不建议近期实现）

前一节刻意只列了最先应该做的入口，不能代表完整差距。继续对照 Agena CLI、TUI 的 Studio/快捷键以及其他 Agent 的完整命令目录后，下面这些缺口同样成立。它们按“已有底层能力但缺 TUI slash 表面”“TUI 已有隐式 UI 但命令语义缺失”“应明确不做”细分，避免将待讨论的愿望清单伪装为现成功能。

### 6.1 Agena CLI 已有成熟管理能力，但 TUI 没有相应命令面

[`AgenaCommand`](../crates/agena-cli/src/cli/mod.rs) 已有 `auth`、`config`、`debug`、`cost`、`memory`、`mcp`、`permissions`、`provider`、`plugin`、`sessions` 等强类型 CLI 命令。这里不是要求 TUI 重写全部 CLI，而是说明有一批用户可理解、后端已具备语义的控制操作没有被 TUI slash catalog 吸收。

| 候选命令族 | 已有的非 slash 能力（证据） | TUI 当前缺口 | 建议的最小 TUI 契约 | 优先级 |
| --- | --- | --- | --- | --- |
| `/mcp [list\|status\|get <server>\|reconnect <server>\|login <server>]` | CLI 有 `status`、`list`、`get`、`add`、`remove`、enable/disable、`reconnect`、`login`、`logout`；详见 [`McpSubcommand`](../crates/agena-cli/src/cli/mod.rs) 和 [MCP CLI 管理文档](configuration.md#mcp-cli-管理)。 | 原报告只写了笼统的 `/mcp`，遗漏了“逐 server 健康、重连、OAuth/bearer 登录、工具数量、脱敏错误”的细粒度需求。 | P1 先只读 `list/status/get` 与 `reconnect`；`add/remove/login/logout` 必须通过表单、确认和 keyring 流程，不把 secret 放进 composer 文本。 | P1 |
| `/plugins [status\|inspect <id>\|logs <id>\|search <q>\|sync\|upgrade]` | CLI 的 `plugin` 有 status、inspect、logs、validate、install、uninstall、installed list、sync、search、upgrade 等子命令。 | 仅深链 `/plugins` 还不够：故障插件、manifest 校验和 marketplace 生命周期难发现。 | P1 提供 `status/inspect/logs` 只读面；安装、卸载、升级进入可预览的 Plugin Workbench，保留审批/网络提示。 | P1 |
| `/permissions [list\|search <q>\|edit]` | CLI 有 list/create/replace/revoke/reply；TUI 有完整 Permission Studio。 | 当前有针对**待处理请求**的 allow/deny，却没有针对**持久规则**的发现、检索和编辑捷径。 | `/permissions` 进入规则 Studio；`list/search` 只读列出 scope/主体/规则来源；写操作继续走确认 UI。 | P1 |
| `/providers [list\|<id>]`、`/models [<provider>]`、`/capabilities <model>` | CLI `provider` 支持 list、models、capabilities；TUI 已有 Provider Studio、Model Catalog 和 model chooser。 | `/model` 只处理本会话选择，不能回答“为何模型不可选、该模型是否支持图片/工具/思考模式、provider 是否已认证”。 | `/providers`、`/models`、`/capabilities` 作为只读 catalog/Studio 深链；`/model` 保持“选择当前会话”的单一职责。 | P1 |
| `/auth [list\|login <provider>\|logout <provider>]` | CLI 有 `auth list`，另有 login/logout；Provider Studio 已有交互认证字段和回调。 | 认证入口被埋在 provider/settings 中，和 `/model` 错误/未配置状态不连贯。 | `list` 脱敏展示认证状态；login/logout 打开 Provider Studio 的对应安全流程，不接受 token 作为 slash 参数。 | P1 |
| `/config [show\|validate\|sources]` | CLI 有 `config resolve`、`config validate`；Settings Studio 已聚合多个配置部分。 | `/settings` 是编辑工作台，不是“当前有效配置、来自哪个 layer、是否有效”的诊断命令。 | `show` 投影有效配置与来源，`validate` 只读，`edit` 才跳转 Settings Studio；敏感值始终脱敏。 | P1 |
| `/sessions [list\|tree <id>\|export <id>\|import <path>]` | CLI 有 list、export JSONL、import JSONL、tree；TUI 仅有浏览/筛选 `/sessions` 与当前 transcript `/export`。 | 会话恢复、树和当前 transcript 导出已有，但会话 bundle 的可移植性/精确树查看没有 TUI 命令契约。 | P2 先做 `tree`、`export` 的确认式入口；`import` 明确创建新会话、展示来源和冲突策略。 | P2 |
| `/cost [session]` | CLI 有 `cost`，而 TUI 有 `/usage` dashboard。 | 使用量周期与一段会话的模型成本不是同一问题；用户无法从 TUI 快速归因。 | 仅在成本数据可用时显示可解释、币种/估算口径明确的会话成本；否则不要伪造精度。 | P2 |
| `/debug session` / `/inspect` | CLI 有 debug session；`agena inspect --json` 输出能力 manifest。 | `/diagnostics` 面向终端问题，不等同于可复制的 session/config/capability 快照。 | 面向支持人员的受脱敏控制的只读报告；debug-only 或显式确认，避免正常菜单噪声。 | P2 |

### 6.2 已经藏在 TUI 或快捷键里的操作，缺少可发现的 slash 表达

| 候选 | 现有路径 | 为什么仍是缺口 | 建议 | 优先级 |
| --- | --- | --- | --- |
| `/clear-screen`（或 `/cls`） | 目前 `/clear` 是 `/new` 的别名；真正退出/中断依赖 `Ctrl+C`，快捷键文档没有“只清终端 scrollback”的 slash 语义。 | 这是跨 CLI 的常见预期陷阱：用户输入 `/clear` 可能以为只清屏，却创建了新会话。 | 保持 `/new` 语义；将 `/clear` 改为弃用别名或在帮助中强警告，并提供明确的 `clear-screen`。这是兼容性决策，应先做 telemetry/迁移评估。 | P1 |
| `/history` / `/prompt-history` | Composer 在光标首行按 `↑` 可打开历史搜索。 | 键盘隐含功能不可链接、不可通过 `/help` 搜索，也无法在远程/替代输入设备上发现。 | 打开同一历史搜索 UI；支持安全的 query 过滤，不自动提交历史文本。 | P2 |
| `/edit-prompt <message>` | 有 composer 编辑、提示历史和 `/rewind` picker。 | “重新编辑并重跑哪个历史用户输入”没有清晰命令入口，回退与编辑重试的区别不明显。 | 先设计消息 ID/预览/确认与对下游分支的影响，再实现。 | P2 |
| `/theme`、`/keymap`、`/vim`、`/interface` | Settings Studio 有 Interface section；TUI 有集中式 keymap。 | TUI 是高键盘密度产品，主题、Vim/编辑行为、快捷键是高频个人偏好；仅 `/settings` 搜索发现成本高。 | 作为深链接/查询入口，不必每项手写配置 DSL；`/keymap <action>` 应显示实际绑定和冲突。 | P2 |
| `/about`、`/version`、`/docs`、`/release-notes` | CLI/仓库有版本、诊断和文档，`/diagnostics` 可看环境。 | 支持、issue 报告和新用户引导缺少短、稳定、可复制的产品身份和文档入口。 | `/about` 输出版本、workspace、已选模型、文档地址；`/docs <topic>` 打开本地/网页文档时需明确外部动作。 | P2 |
| `/tools [list\|search\|detail]` | runtime static plugin 包含 filesystem、shell、web、plan、skills、LSP、cron、memory、MCP、settings；Plugin Host/Tool Registry 已存在。 | 用户无法从 TUI 理解“当前模型看见哪些工具、来自哪里、为何某工具不可用/需要审批”。 | 只读工具目录：名称、来源、tags、参数摘要、权限状态、availability；不直接把工具执行按钮塞进命令。 | P1 |
| `/lsp [status\|restart]` | 已有 LSP static plugin，按首次匹配文件 lazy-spawn；配置见 [LSP](configuration.md#lsp)。 | 对语言服务器不可用、未启动或项目根路由错误的排障没有交互入口。 | 先实现 status（server、root、启动/错误、语言匹配）；restart 仅在生命周期 API 完整时加入。 | P2 |
| `/web [status]` / `/browser [status]` | `agena.web` plugin 支持本地 crawl、fetch 与受管理 Chrome/Chromium；配置说明见 [Web](configuration.md#web)。 | 用户看不见浏览器是否可用、当前渲染策略是什么、失败应去哪里修。 | 合并到 `/doctor` 的 provider/plugin/browser 分栏即可，不一定另建常驻命令。 | P2 |

### 6.3 Agent、计划、任务和持久记忆仍有控制面缺口

| 候选 | 当前情况 | 建议 | 优先级 |
| --- | --- | --- | --- |
| `/agents [list\|show <name>\|source <name>]` | `/agent` 是当前 session chooser；配置支持内置、用户、workspace、配置与运行时注册的 profile 优先级，CLI 也有 `agents list`。 | 用复数 `/agents` 管理/浏览 catalog，用单数 `/agent` 选择当前会话。显示来源、模型、权限上限和可用工具，不把 profile 内容静默注入。 | P1 |
| `/plan` / `/task` / `/tasks` | runtime 已有 plan static plugin 和异步 task plugin，但当前 TUI registry 没有用户查看/控制入口。 | 这是第 0 节的 P0 主线：Plan board 和 Task dashboard 必须能变更、暂停、恢复、取消和审计，不能只做只读列表。 | P0 |
| `/memory search <q>`、`/memory status`、`/memory write` | 当前 `/memory` 只做 list/open/forget；memory plugin 本身提供 search/get/list/write/delete execution tools。 | 记忆入口偏文件编辑器，缺检索、来源、自动回忆状态和显式写入的用户控制。 | P1 先 `search/status`；`write` 必须显示目标文件/范围/覆写预览，避免把模型记忆与用户笔记混淆。 | P1 |
| `/cron` / `/schedule` | runtime 已有 cron plugin 的 list/create/delete/update/pause/resume/history/wakeup，但 slash registry 没有相应面。 | 作为 P1 的 Schedule board；以表单处理 cron/timezone/preview，不能要求用户记住底层 schema。 | P1 |
| `/goal`、`/workflow`、`/deep-research` | Agena 有 Plan、Task、Cron 的底座，但没有 Codex 式 Goal 状态。 | `/goal` 是 P0；通用 `/workflow` 和 `/deep-research` 应在 Goal/Plan/Task dashboard 稳定后作为 P2 编排模板，而不是先做字符串命令。 | P0/P2 |

### 6.4 需主动拒绝或谨慎处理的“看似缺失”

| 候选 | 结论 | 原因 |
| --- | --- | --- |
| `/hooks` | 不建议作为顶层兼容命令直接复活。 | [`configuration.md`](configuration.md#removed-agenahooks) 说明旧的 `agena.hooks` shell/HTTP bridge 已移除，相关行为应由常规 plugin manifest hook 实现。可在 `/plugins` 详情中展示 hooks，而不是制造过时的全局模型。 |
| `/directory` / 多工作区管理 | 不是当前确定缺口。 | 现有运行以一个 workspace root 为明确边界；在没有多根工作区生命周期、权限合并和文件提及规则之前，不能仅模仿 Gemini 的 `/directory`。 |
| `/shell`、`/bash` | 默认不应增加。 | Agena 已有受权限控制的 shell Tool。把自由 shell 命令变成 slash fast path 会扩大误触发、注入、审计和 approval 绕过风险。 |
| `/always-approve`、`/auto` | 不建议作为一个模糊的全局开关。 | 应复用 Permission Studio 的按工具、路径、网络、scope 规则；“总是批准”容易隐藏影响范围。 |
| `/think`、`/reasoning` | 仅在 Provider/Model capability 有准确映射后考虑。 | 当前应通过 `/model`/`/capabilities` 展示并选择可支持的 mode；不要在不支持的 provider 上伪造统一参数。 |

本节不改变第 0 节结论：近期只做 `/stop`、`/goal`、`/plan`、`/task`、`/tasks`，然后是 `/schedule`、`/processes`、`/context`。会话导入导出、界面偏好、LSP/browser status、项目命令和各类设置深链接由正常 UI 导航解决，除非后续证据显示它们阻塞核心工作流。

## 7. 推荐的统一命令模型

现有 `CommandSpec` 可以渐进升级为统一的 `CommandDescriptor`。不要在第一步重写全部动作；先让旧字段兼容，再逐个迁移。建议的最小元数据如下：

```rust
CommandDescriptor {
    id,                         // 稳定机器 id，例如 "session.stop"
    slash, aliases,             // 用户输入
    title, description, category,
    provenance,                 // builtin | plugin(id) | project | user | skill | mcp(server)
    input_schema,               // JSON Schema 或等价 typed contract
    input_mode,                 // none | text | attachments | text_and_attachments
    availability,               // session/idle/busy/permission/capability predicates
    execution,                  // local_ui | local_effect | submit_prompt | plugin_invoke
    side_effect,                // read_only | session_mutation | external_mutation
    approval_requirement,
    result_kind,                // message | overlay | prompt | route | url | structured_event
    audit_policy,
}
```

它解决的不是“抽象优雅”，而是可验证行为：

- composer 可以根据 `availability` 隐藏、置灰或解释不可用命令；busy-safe 命令在错误的早退点之前执行。
- `/help <command>`、palette、键盘补全和 API/远程 TUI 都读取同一描述，而不再各自拼 usage 字符串。
- 参数 schema 驱动 enum/model/会话/file path 建议、必填校验和安全地显示解析后的效果；复杂语法仍可以由命令专属解析器实现。
- `provenance` 让用户知道命令来自何处，并为插件、项目命令、Skill、MCP prompt 的冲突和信任策略提供基础。
- 统一 effect/audit 使诊断能回答“刚才的 `/foo` 是谁提供的、它是否提交了模型 prompt、是否请求了权限、结果是什么”。

### 命名、冲突与安全建议

1. 内置命令保留最高优先级；插件保持现有“不能遮蔽内置”的规则。
2. 对非内置来源提供可限定名字，例如 `/plugin:<plugin-id>:<command>`、`/project:<name>`、`/skill:<name>`；短名只有无冲突且来源可信时才显示。
3. 项目命令只从显式目录/manifest 发现；展示文件来源、版本控制状态、作者/签名（若可用）和它将要做的事。
4. 项目命令、Skill、MCP prompt 若会执行 Tool、shell、写文件或网络请求，仍经过现有 permission policy；“从 slash 触发”不是信任提升。
5. 对 command-only fast path 写出严格定义：哪些命令不会调用模型、哪些可在 busy 时执行、哪些会开始新 run、哪些只读。不要仅靠注释约定。

## 8. 分阶段落地计划

### P0：一条可持续工作的控制链

1. 修复 busy 时本地命令被 `submit_composer()` 拦截的问题。
2. 增加 `/stop`（可别名 `/cancel`），并确保它只取消当前 active execution；busy session、没有 active run、重复调用均有确定反馈。
3. 给现有 `agena.plan` 建立 TUI command adapter 和 Plan board：`/plan`、`/plan draft`、`/plan pause|resume|clear`。复杂步骤/检查项只在 UI 内编辑。
4. 新增持久 `Goal` 状态与 `/goal`：设置/替换、status、pause、resume、clear、token budget、用量、阻塞原因、证据和与 plan/task 的关联；替换旧 goal 必须确认。
5. 给现有异步 `agena.tasks` 建立 `/task` 和 `/tasks`：创建立即返回 task card，list/show/cancel/message/followup/wait 都对应已有 task lifecycle，而非重新实现子 Agent runtime。
6. 为 `/stop`、`/goal`、`/plan`、`/tasks cancel` 和权限回复写 busy/restart/cancellation 回归测试，尤其验证 task restart 后只能显式 follow-up，绝不自动重放。

### P1：把后台工作变成可看、可控的 UI

1. 建立 Task dashboard、Plan board 和 Goal card 的统一关联视图；`/goal`、`/plan`、`/tasks` 只负责进入正确状态/视图。
2. 将 `agena.cron` 接到 `/schedule`，展示 next fire/history，所有新增或修改 schedule 都用结构化表单和确认预览。
3. 将 `agena.shell.list/logs/stop` 接到 `/processes`（`/ps`），把 shell background process 和 agent task 明确区分。
4. 增加 `/context`，汇总 context budget、compact 状态、active execution、Goal/Plan/Task；它应是工作流 dashboard，不是技术诊断 dump。
5. 将 `CommandSpec.arguments` 的显示用途和真实输入 schema 分离，优先覆盖 task id、agent profile、budget、timeout、schedule id 与 plan/goal action。
6. 将 `/model`、`/agent`、`/compact`、`/help` 作为后续体验改善，不与工作控制面抢占 P0。

### P2：建立可复用编排，而非堆命令

1. 在 Goal/Plan/Task contract 稳定后，增加 `/research <question>` 等**有固定可视化状态和可验证输出**的编排模板。
2. 引入统一 `CommandDescriptor` 和结构化 command result/audit event，首批迁移 Goal/Plan/Task/Schedule/Process。
3. 做参数级自动补全：agent profile、task id、budget、timeout、schedule id、plan action；不要先做插件/设置命令补全。
4. 设计受信的项目/用户 command manifest、Skill/MCP prompt bridge 仅在核心控制面稳定后评估。

## 9. 验证清单

实现每项时，至少应验证以下维度，而不只检查“能在 idle 时跑一次”：

| 维度 | 样例 |
| --- | --- |
| 解析 | 别名、大小写、`//` 逃逸、空参数、引号/空白、未知命令不吞普通 prompt。 |
| busy/queue | busy 时 `/stop`、权限回复、只读命令立即可用；会新建 run 的命令按设计拒绝、steer 或入队。 |
| 权限 | 命令不绕过 file/network/shell/plugin/MCP 的既有批准机制。 |
| 建议 | 无参命令可一次接受即执行；插件无参命令也应遵守自己的 schema，而不是固定多按一次 Enter。 |
| 可发现性 | `/help <command>`、palette 与补全中的描述、参数、来源、可用性一致。 |
| 兼容性 | 旧别名、现有 `/snapshot`、`/pr` 等手写参数格式和 plugin manifest 不被破坏。 |
| 审计 | 可追踪命令来源、解析参数摘要、effect、审批和错误；敏感参数不明文泄露。 |

## 10. 主要证据索引

### Agena（主证据）

- [内置注册、解析、别名和现有测试](../crates/agena-tui-app/src/commands.rs)
- [内置与插件命令执行分发](../crates/agena-tui-app/src/app_command_actions.rs)
- [composer 建议、无参提交和插件固定需参数行为](../crates/agena-tui-app/src/app_composer.rs)
- [busy/queue/submit 控制流](../crates/agena-tui-app/src/app_session_interactive/execution.rs)
- [插件 slash 名、别名和详情辅助函数](../crates/agena-tui-app/src/app_command_helpers.rs)
- [插件命令的 manifest contract](../crates/agena-plugin-sdk/src/manifest.rs)
- [bundled `/schema-lab` 示例](../crates/agena-bundled-plugins/src/plugins/provided/schema_lab.rs)
- [当前 23 个插件 / 102 个 Tool 的权威索引；含 plan、tasks、cron、shell、context](plugins-and-tools-reference.md#当前权威插件索引)
- [`agena.plan` 的 objective/phase/steps/autorun 契约](plugins-and-tools-reference.md#agenaplan)
- [异步 `agena.tasks` 的 task id、预算、cancel/message/followup/wait 与重启恢复实现](../crates/agena-bundled-plugins/src/plugins/provided/tasks.rs)
- [session 单写者、取消和 UI 状态机契约](session-execution-lifecycle.md)

### 对照材料

- [Codex slash command registry](../../codex/codex-rs/tui/src/slash_command.rs) 与 [受限的 `/diff` 实现](../../codex/codex-rs/tui/src/get_git_diff.rs)
- [Gemini CLI 内置命令](../../gemini-cli/docs/reference/commands.md) 与 [自定义命令、安全参数替换](../../gemini-cli/docs/cli/custom-commands.md)
- [Grok Build slash command、Skill 和自动补全说明](../../grok-build/crates/codegen/xai-grok-pager/docs/user-guide/04-slash-commands.md)
- [OpenCode 的 custom/skill/MCP badge 文案](../../opencode/packages/app/src/i18n/en.ts)
- [OpenClaw 的 command/directive/fast-path 模型](../../cline/openclaw/docs/tools/slash-commands.md)
- [既有 Tool/Skill/MCP 边界调研](../../agent_tools_skills_mcp_2026-07-24.md)
