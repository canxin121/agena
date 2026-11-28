# 在 Agena 中使用 Codex、Claude Code 和 DeepSeek Harness 插件

## 安装

Agena 独立完成安装和运行，无需安装 Codex、Claude Code 或 DeepSeek Harness 的 CLI。它直接读取 marketplace 或 npm 包，把插件放到实例的 marketplace 缓存中，再写入 `agena.json`。从本地目录安装时使用目录里的文件。Agena 加载和调度已适配的组件。

```bash
agena plugin install codex:superpowers
agena plugin install claude:feature-dev --enable-hooks
agena plugin install deepseek:@hawk2048/oh-my-dsh
agena plugin install codex:review@team --marketplace owner/codex-marketplace
agena plugin install codex:review --package-source owner/review@v1.2.0
agena plugin install ./local-plugin --enable-hooks
```

- Web 的插件市场可以选择 Codex、Claude Code 或 DeepSeek Harness，搜索后直接安装。DeepSeek 默认按 npm 包名查找，也可以填写 DeepSeek 插件索引的 JSON URL。CLI 可以用 `agena plugin search superpowers --ecosystem codex`，通过 `--marketplace` 指定自定义市场或索引。
- `codex:插件名` 和 `claude:插件名` 默认从各自的公共 marketplace 安装。搜索只读取默认分支当前提交的市场索引，安装同仓库插件时只下载声明的插件目录，并按 Git 对象 ID 校验文件。声明完整提交 SHA 的 GitHub 子目录来源也使用这条下载路径。GitHub 接口不可用时会尝试 Git，已经解析到提交的下载仍固定在该提交。
- 自定义 marketplace 支持本地目录、HTTPS Git 仓库、GitHub `owner/repo@ref` 或 JSON URL。Git 下载和 Claude 依赖标签查询读取 Agena 进程的 Git 代理配置及 `HTTPS_PROXY`、`NO_PROXY` 环境变量。首次用 `--marketplace` 指定来源后，可用 `插件名@marketplace` 再次安装。JSON URL 的条目需要能独立获取插件文件。
- DeepSeek Harness 支持 npm 包名、版本、常用版本范围和 dist-tag，并校验 registry tarball 的完整性摘要。`--registry` 可以指定另一个 registry。
- 三种生态都可以用 `--package-source` 直接从目录、归档或 Git 仓库安装。使用 manifest 中的插件名，Web 的「安装 Agent 插件」也可填写包来源。Agena 保存来源供 `plugin upgrade` 使用。声明 marketplace 依赖的 Claude 包通过其市场安装，以便解析依赖版本和来源。
- `--installed` 读取磁盘上已经存在的安装。`--native-home` 可以选择要读取的目录。DeepSeek Harness 的 `--profile` 在这个模式下选择要导入的 profile。
- Claude marketplace 使用 `command` 安装源时，Agena 会显示命令和审批哈希。审阅后用 `--accept-command <SHA-256>` 重试。安装命令返回包目录，Agena 从该目录复制插件文件。
- 从 Claude marketplace 安装时，Agena 递归解析 `dependencies`，检查版本区间、循环依赖以及根 marketplace 的跨市场允许列表。Git 依赖可使用 `<插件名>--v<版本>` 标签。同次安装的多个依赖方共用一个满足各自版本要求的版本。已有且启用的依赖保留当前版本，版本冲突会使本次登记失败。使用 `command` 或 marketplace 下载认证 helper 的依赖需先单独安装。
- `--enable-hooks` 允许导入的 hooks 在 Agena 项目会话中运行。`--env NAME` 声明允许读取的服务环境变量。`--option-env key=NAME` 将 Claude 插件选项映射到服务环境变量。
- 复制到缓存的 Claude 包可以按 `bun.lock`、`npm-shrinkwrap.json` 或 `package-lock.json` 安装锁定的 Node.js 依赖。Agena 在独立目录调用 Bun 或 npm，跳过生命周期脚本。当依赖安装失败时，安装结果会显示原因。Codex 市场里的 Claude 格式包也使用这套依赖安装流程，并读取原有的 Claude manifest。

```bash
agena plugin list-installed
agena plugin upgrade claude.review-team
agena plugin upgrade --all
agena plugin validate ./local-plugin
agena plugin inspect claude.review-team
agena plugin uninstall claude.review-team
```

安装会打印 Agena 插件 ID。有记录来源的包可以用 CLI 或 Web 升级，并保留已有的启用状态和设置。首次安装 Claude 包时采用 `defaultEnabled`。CLI 的 `--dry-run` 和 Web 市场的「预览」会解析来源，显示版本、可识别组件、Claude 插件依赖和兼容诊断。远端包暂存到预览结束。预览仅读取包和锁文件，安装命令、Node.js 依赖安装和配置登记在正式安装时执行。Claude marketplace 插件及其依赖在一次配置写入中登记。已导入的组件通过当前 Agena 会话的权限和沙盒规则执行。组件自带的脚本、MCP 服务和语言服务器按各自的声明使用相应的解释器或可执行文件。

`agena plugin validate ./local-plugin` 的 JSON 输出包含组件数量和 Cordis 模块声明的服务依赖。缺少服务或服务拦截配置暂未支持时，对应模块会给出诊断，同包的独立组件仍可导入。使用 `--strict` 可以让相同诊断导致校验失败。

## 在 Agena 会话中运行

| 来源 | 已适配的组件 |
| --- | --- |
| Codex | Agent Plugins v1 和 `.codex-plugin/plugin.json` 中的 Skills、MCP、Markdown 命令、命令型和 MCP 工具型 hooks。支持 `extensions.com.openai.hooks`。显式声明覆盖默认 `hooks/hooks.json`。 |
| Claude Code | Skills、Markdown 命令、带工具限制和 worktree 隔离的 agents、output styles、终端主题、`bin/`、LSP、MCP 和 MCPB/DTX。支持 fork 命令与技能、命令型、HTTP、MCP 工具型、prompt 和 agent hooks，以及后台 hooks、局部 hooks、monitors、workflows 和部分 JavaScript mods 接口。 |
| DeepSeek Harness | `dsh.bundle.patch` 中的 `dsh-mcp-client`、`dsh-mcp-manager`、`dsh-lsp-stdio`、`dsh-skill-filesystem`、`dsh-hooks-codex` 和 `dsh-hooks-claude-code`。支持 Cordis 斜杠命令、JSON 域存储、工作流、本地子代理与树内协作、MCP 服务注册、查询、探测、重启和状态监听、MemVault 的记忆注入和回合提取、MCP credential templates、嵌套组和有序 patch。 |

每个包注册 `<插件 ID>.run`。在项目会话中调用它会启动一个 Agena 子 agent，并把子 agent 的最终文本带回当前调用。输入为 `{"prompt":"审阅本次改动","agent":"reviewer"}`，`agent` 是可选的导入角色名。子 agent 使用 Agena 的模型、权限、工具和任务记录。Claude 角色的模型 ID 和 `sonnet`、`opus`、`haiku` 可映射到 Agena 已启用的模型路由。精确 ID 优先，同等匹配优先选用默认提供商及其默认路由。没有唯一匹配时继承父会话的选路，并记录插件诊断。

手动配置可用 `plugins.list.` 下的 `package.kind: "agent_plugin"`，再将 `path` 指向本地目录。相对路径以声明条目的配置文件所在目录为基准。`agena plugin logs <插件 ID>` 可查看未映射组件和配置错误。

### 生命周期 hooks

插件工作台的 Hooks 页会显示已注册的处理器。命令处理器可以使用 shell，或通过 `args` 传递独立参数。Agena 将事件 JSON 写入 stdin，在会话工作区和沙盒内运行命令。HTTP hook 使用沙盒内的 `curl`。Header 环境变量需同时出现在 handler 的 `allowedEnvVars` 和 `package.environment` 中。

Claude `prompt` hook 使用 Agena 的辅助模型配置，或会话的模型配置。`$ARGUMENTS` 会替换成事件 JSON。模型给出的 `{"ok":false,"reason":"..."}` 可阻止提交或工具调用，向工具结果补充上下文，或让 `Stop` 继续处理该原因。

Claude `agent` hook 使用独立的模型上下文。它可以在返回判定前通过 Agena 会话的权限调用 Read、Grep 和 Glob。读取结果保留在验证上下文中，内部调用跳过 hooks 以避免重复触发。

Claude 与 Codex 的 `mcp_tool` hook 通过 Agena 的 MCP 权限和连接管理调用工具。可引用 Agena 配置中的服务名，或用 `plugin:<包名>:<服务名>` 引用同一包的 MCP 服务。Codex 也可直接填写本包的服务名。输入支持用 `${tool_input.file_path}` 等路径引用事件字段。处理器会解释 MCP 返回的文本和 hook JSON，内部 MCP 调用沿用正常授权并避免重新触发同一个 hook。

| Agena 行为 | 对应事件 |
| --- | --- |
| 拦截工具、改写参数、补充上下文 | `PreToolUse`。改写后的输入重新校验及授权。 |
| 替换模型看到的输出、追加上下文或观察工具失败 | `PostToolUse`、`PostToolUseFailure` |
| 一轮的本地工具调用全部完成后补充上下文或结束当前轮次 | `PostToolBatch` |
| 检查或阻止提示词提交 | `UserPromptSubmit` |
| 初始化和清理会话 | `SessionStart`、`SessionEnd` |
| 服务重启后继续会话、创建会话分支、提交压缩摘要 | `SessionStart` 的 `resume`、`fork` 和 `compact` matcher |
| 显式执行初始化或维护命令 | Claude `Setup`，在 `SessionStart` 之前运行 |
| 在导入的斜杠命令及技能展开前补充上下文或拒绝调用 | `UserPromptExpansion` |
| 在 Claude 技能的 shell 片段执行前跳过单个片段或撤销整次调用 | `PreSkillShellCommand` |
| 延续或观察运行 | `Stop`、`Interrupt` |
| 在压缩前补充摘要要求或拒绝，读取压缩结果 | `PreCompact`、`PostCompact` |
| 观察经过重试流程后仍失败的模型请求 | `StopFailure` |
| 在权限询问前拒绝 | `PermissionRequest` |
| 观察采样和压缩中的每次模型请求尝试 | Codex `PreModelRequest`、`PostModelRequest` |
| 在提交会话模型变更前审批，在变更后为新模型补充上下文 | Claude `PreModelSwitch`、`PostModelSwitch`。配置重载改变继承路由、重启后恢复已保存的会话路由时也能触发后置 hook。 |
| 观察自动审批模型的明确拒绝 | Claude `PermissionDenied` |
| 给子 agent 注入启动上下文或要求它继续执行 | `SubagentStart`、`SubagentStop` |
| 团队成员子 agent 进入空闲等待时通知或继续派发反馈 | Claude `TeammateIdle` |
| 为隔离的子 agent 创建或显式移除工作副本 | Claude `WorktreeCreate`、`WorktreeRemove` |
| 拦截当前会话的任务创建和完成 | `TaskCreated`、`TaskCompleted` |
| 观察工作目录的实际切换 | `CwdChanged` |
| 观察磁盘上受监视文件的新增、改写和删除 | `FileChanged` |
| 观察 Claude 项目指令、规则和导入文件进入会话上下文 | Claude `InstructionsLoaded` |
| 在助手文本流式显示前改写显示内容 | Claude `MessageDisplay` |
| 观察磁盘配置变化，在自动重载前审批项目设置 | Claude `ConfigChange` |
| 观察额外工作目录注册 | Claude `DirectoryAdded` |
| 工具审批等待约 6 秒，或子 agent 结束后通知父会话 | `Notification` |
| 在 MCP 表单或链接交互前应答，以及在用户提交后改写或拒绝应答 | Claude `Elicitation`、`ElicitationResult` |

工具 matcher 支持 `Bash`、`Read`、`Write`、`Edit`、`mcp__server__tool` 和 Agena 工具全名。Claude 工具事件支持 `if`，例如 `Bash(git *)` 和 `Read(src/**)`。匹配使用当前有效参数，包括同一阶段前面的 hook 对参数的改写。处理器共用对应 Agena 事件的执行时间限制。

Codex 的 `PreToolUse`、`PermissionRequest` 和 `PostToolUse` 收到本地 shell 调用的实际工作目录 `cwd`。项目会话的当前工作目录是 hook 脚本本身的运行位置。`Bash` 处理器的 `tool_input` 包含 `command`，`PostToolUse` 的 `tool_response` 提供命令输出文本。`updatedInput.command` 只改写命令文本，工作目录、超时和资源声明随原调用保留。有效命令继续经过 Agena 的权限检查。[Codex shell hook 工作目录](https://github.com/openai/codex/pull/57203)

`shell.open`、`shell.spawn` 和 `shell.watch` 启动 Bash 时触发 Codex `PreToolUse`。启动调用或后续的 `shell.read`、`shell.logs`、`shell.write` 可收取完成结果。第一次收完结果时触发一次 `PostToolUse`，携带原启动调用的 `tool_use_id`、实际命令、工作目录和本次收取的输出。可用 `shell.read` 的 `since_seq` 参数回读输出。[Codex unified exec hooks](https://github.com/openai/codex/blob/06023b038f642a2290a9ab53392342c18c5b4f39/codex-rs/core/src/tools/handlers/unified_exec/write_stdin.rs)

同步 Codex 命令和 MCP 工具 hooks 可声明 `onFailure: "block"`。处理器失败时，`PreToolUse` 阻止工具执行，`PermissionRequest` 拒绝审批，`PostToolUse` 将失败原因发给模型并保留已执行工具的原始回执。`SessionStart`、`UserPromptSubmit`、`SubagentStart`、`Stop`、`SubagentStop`、`PreCompact` 和 `PostCompact` 接入对应的取消、拒绝或继续执行逻辑。`PostCompact` 的阻止决定保留已提交的摘要。显式选择 PowerShell 的 Codex 命令 hook 会把原生程序退出码传回 Agena。[Codex required hooks](https://github.com/openai/codex/pull/62831) · [Codex PowerShell hooks](https://github.com/openai/codex/pull/64013)

Codex 命令 hook 可用 `additionalContextLimit` 调整上下文预览。超出限额的正文保存在该会话的 hook 输出目录，模型收到预览和原文路径。Agena 的事件上下文总预算和路径权限继续适用。[Codex 长输出规范](https://learn.chatgpt.com/docs/hooks#large-hook-output)

Codex 的 `PreModelRequest` 和 `PostModelRequest` 随采样和前台压缩中的每次模型请求尝试派发。提供商重试保留该次生成的 `request_id` 和 `turn_id`，`attempt_number` 从 1 递增。`PostModelRequest` 报告 `completed`、`failed` 或 `cancelled`，并传入可取得的 `response_id`、`token_usage` 与错误。OpenAI Responses 的不完整响应带上该次响应 ID、用量和结束原因。模型请求 hooks 的文本输出用于观察。[Codex 模型请求 Hook 实现](https://github.com/openai/codex/blob/06023b038f642a2290a9ab53392342c18c5b4f39/codex-rs/hooks/src/events/model_request.rs)

同步 MCP `PostModelRequest` 可在 `structuredContent.extra_codex_rollout_budget_units` 返回有限的非负数。Agena 对完成的响应和报告了用量的不完整响应登记该值，并在会话用量中以 Codex rollout 单位统计。格式错误的回传记入 Hook 诊断，有效的回传继续计入用量。 Responses 提供商可在 `usage.codex_rollout_budget_units` 返回原生权重用量。Agena 在正常响应和带用量的不完整响应中读取该数值，和 hook 回传的额外用量分别统计。

Claude 的项目指令支持 `CLAUDE.md`、`.claude/CLAUDE.md`、附加的 `CLAUDE.local.md` 和递归发现的 `.claude/rules/**/*.md`。主指令沿用 Agena 的目录优先级，依次查找 `AGENTS.override.md`、`AGENTS.md`、`AGENA.md`、`CLAUDE.md` 和 `.claude/CLAUDE.md`。根目录的通用规则在组装模型上下文时加载。子目录规则随文件访问发现。`paths` 规则以包含 `.claude` 的目录为基准，支持 YAML 列表、逗号分隔和大括号通配，适用于文件读取、写入、编辑和可归属到单个文件的 Bash 预览。

Claude 项目指令中的 `@path` 可以按引用文件所在位置解析相对路径，最多递归四层。带空格的文件名使用反斜杠转义，Markdown 代码与引号中的路径按原文处理。项目文件、规则和导入均受 Agena 的工作区读取权限及字节限额约束。同一个文件的符号链接别名共用加载记录。[Claude Code 项目指令](https://code.claude.com/docs/en/memory)

Claude 的 `InstructionsLoaded` 按 `load_reason` 匹配。根目录指令首次进入模型上下文时发送 `session_start`，访问子目录时发送 `nested_traversal`，命中路径规则时发送 `path_glob_match` 并附带 `globs`，导入时发送 `include` 并附带 `parent_file_path`。按需加载携带 `trigger_file_path`，本地指令的 `memory_type` 为 `Local`。成功压缩后发送 `compact` 并重置去重记录。同一会话中，Agena 为每个指令文件记录最近通知的内容，文件内容变化后再次通知。该事件异步观察模型上下文中的指令加载。[Claude Code InstructionsLoaded](https://code.claude.com/docs/en/hooks#instructionsloaded)

Claude `MessageDisplay` 接收助手文本的逐行批次，带稳定的 `turn_id` 和 `message_id`、递增的 `index`，以及 `final`。超过 64 KiB 的单行会拆开。处理器返回 `hookSpecificOutput.displayContent` 后，Web、TUI 和流式内容 API 读取另外保存的显示流。工具内容、用户文本、模型上下文和插件转录文件沿用原始数据。API 文本项在 `original_text_resource` 提供原始文本，TUI 导出时使用该字段。处理器失败或超时时显示原文。当前 Agena 的交互式和 API 运行都使用逐行批次。[Claude Code MessageDisplay](https://code.claude.com/docs/en/hooks#messagedisplay)

Claude `ConfigChange` 在自动重载前使用已有运行时和已启用插件执行。默认工作区的 `.claude/settings.json` 和 `.agena/agena.json` 对应 `project_settings`，`.claude/settings.local.json` 对应 `local_settings`。工作区内已经纳入重载监听的 `SKILL.md` 对应 `skills`。退出码 2 或 `decision: "block"` 可以阻止候选配置发布，后续自动重载会重新检查曾被阻止的路径。实例配置变化以 `user_settings` 通知现有项目会话。手动重载由 Agena 的设置入口处理。[Claude Code ConfigChange](https://code.claude.com/docs/en/hooks#configchange)

Agena 合并默认工作区 `.claude/settings.json` 与 `.claude/settings.local.json` 中的 `claudeMdExcludes`。绝对路径 glob 作用于 `CLAUDE.md`、`CLAUDE.local.md`、`.claude/rules/` 和由这些文件导入的文档，可按符号链接路径或解析后的实际路径匹配。设定随运行时快照生效。`ConfigChange` 拒绝自动重载时，原有排除规则继续生效。[Claude Code 指令排除规则](https://code.claude.com/docs/en/memory#exclude-specific-claudemd-files)

Claude hook 中的插件 MCP 工具使用 `mcp__plugin_<包名>_<服务名>__<工具名>`。事件的 `mcp_server.name` 为 `plugin:<包名>:<服务名>`，`source` 为 `plugin`。该来源由 Agena 已注册的 MCP 服务确定。Agena 自行配置的服务在 Claude 事件中使用 `source: "agena"`。

Claude `PostToolUseFailure` 提供已执行工具的错误和可取得的执行时间。MCP 执行错误的 hook 上下文随之后的模型请求发送。前台 shell 命令的非零退出码以 `Exit code N` 开头传入该事件，hook 上下文跟随工具结果。

Claude `PermissionDenied` 收到自动审批模型拒绝的工具名、有效参数、调用 ID 和理由。返回 `hookSpecificOutput.retry: true` 会为下一次模型请求添加重试提示。每次新的工具调用都再次经过 Agena 的权限检查。

Claude `Elicitation` 和 `ElicitationResult` 按 `mcp_server_name` 匹配。插件内服务使用 `plugin:<包名>:<服务名>`。`Elicitation` 可以直接返回 `accept`、`decline` 或 `cancel`，省去用户输入。由用户操作的请求在应答后触发 `ElicitationResult`，可用新的 `action` 和 `content` 替换整份应答。多个处理器给出不同决定时，`decline` 优先。退出码 2 或 `decision: "block"` 也会拒绝该请求。DeepSeek Harness 的 Claude hooks 使用同一规则。

启用的 Claude `UserPromptExpansion` hooks 可以检查当前会话中导入的 Markdown 命令和技能，包括 Codex 命令。Matcher 使用上游的 `包名:命令名`，例如 `^review:explain$`。Hook 添加的上下文会随展开后的提示词提交。对于 fork 命令，上下文进入子 agent 的任务，拒绝决定会阻止子 agent 启动。

`PostToolBatch` 每批触发一次，按模型请求中的顺序提供已完成的 Agena 工具调用。载荷包含有效参数、调用 ID 和模型看到的工具结果文本。过大的载荷会缩减预览。Hook 可以用 `additionalContext` 为下一次模型调用补充上下文，或用拒绝决定结束当前执行并在会话中记录原因。

`Notification` 支持 `permission_prompt` 和 `agent_completed` matcher，提供 `notification_type`、`title` 和 `message`。通知处理与用户审批并行，审批由 Agena 的权限交互处理。

Claude 和 Codex 的 `SubagentStart` 可在子 agent 第一次调用模型前加入 `additionalContext`。`SubagentStop` 在子 agent 结束前提供 `last_assistant_message`。如果 hook 要求继续，Agena 让子 agent 在同一次执行里处理反馈，并在下次 hook 调用时设置 `stop_hook_active`。角色 matcher 可使用插件中的 `包名:角色名`。Claude `TeammateIdle` 在具名团队成员子 agent 结束回合进入空闲前触发，按 `teammate_name` 匹配并携带 `teammate_name` 与可选 `team_name`；返回退出码 2、`decision: "block"` 或 `continue: false` 时，Agena 将反馈原因交回该子 agent 继续执行。

模型完成当前响应时，即使会话仍有后台任务，Agena 也会触发 Claude `Stop`。`background_tasks` 包含仍在运行的后台 shell、monitor 和子 agent，`session_crons` 包含仍会唤醒该会话的已登记任务。Claude `SubagentStop` 查询直接父会话的这两组数据，父子会话均需启用该插件。DeepSeek Harness 的 Claude 方言 hooks 使用同一机制。[Claude Code Stop 输入](https://code.claude.com/docs/en/hooks#stop-input)

命令 hooks 收到 `transcript_path`，指向 Agena 从当前会话历史生成的 JSONL 文件。Claude hooks 使用会话消息格式，Codex hooks 使用 rollout 格式。DeepSeek Harness 通过对应的 hook 桥使用相同格式。`SubagentStop` 提供父会话的 `transcript_path` 和子会话的 `agent_transcript_path`。Claude `SubagentStart` 提供父会话的记录，Codex `SubagentStart` 提供子会话的记录。父会话访问要求记录中存在直接的子 agent 关系、两个会话属于同一工作区和根会话，且都启用了该插件。

会话记录包含已提交的用户文本、助手文本、工具调用与结果、已提交的压缩摘要。每个文件保留最多 64 MiB 的近期记录，单段文本最多约 256 KiB。附件内容、推理内容和供应商内部状态不写入这些文件。文件在会话生命周期内使用稳定路径，并以只读方式挂载到 hook 和后续的沙盒 shell 进程。HTTP、MCP 和模型处理器保留空路径。已启动的后台 shell 使用启动时挂载的记录版本。

Claude 的 `FileChanged` 使用操作系统的文件监听。插件的 `matcher` 按 `|` 分割成工作目录中的字面文件名，再按文件名过滤事件。`SessionStart`、`CwdChanged` 和 `FileChanged` 可返回绝对路径组成的 `watchPaths`，替换当前会话的动态列表。空数组会清除动态路径。一个插件登记的路径可触发同一会话中其他插件的匹配 hook。Agena 在注册和派发时检查会话的读取权限。其他进程写入文件也能触发 hook。运行时重载会更新会话的静态匹配规则，并保留动态列表。关闭或清空会话会停止对应的监听。

前台 Bash 命令实际切换到会话获准读取的目录时，Agena 记录新目录并发送 `CwdChanged`，其中包含 `old_cwd` 和 `new_cwd`。后续 Bash 命令默认使用该目录，静态文件监听随之移动。Claude 的 `CLAUDE_PROJECT_DIR` 始终指向项目根目录。适用的 `Setup`、`SessionStart`、`CwdChanged` 和 `FileChanged` 命令 hook 可通过 `CLAUDE_ENV_FILE` 为同一项目会话的新 Bash 进程设置环境，包括前台命令、后台命令和交互终端。工作目录变化时，Agena 为该目录创建新的环境文件。

#### 额外工作目录

```bash
agena sessions working-directories 42 --add ../shared
agena sessions working-directories 42
```

相对路径从当前会话的项目根目录解析。Agena 验证目录存在并将规范路径保存在这个可写项目会话中，服务重启后可继续使用。文件工具和后续启动的沙盒进程按该会话的工作区默认权限访问这些目录，同时执行显式路径规则、宿主保护和权限上限。MCP 的 `roots/list` 公布会话当前可读的附加目录。

REST 客户端可以向 `/api/v1/sessions/{id}/working-directories` 提交 `POST` 请求，正文形如 `{"path":"../shared"}`。返回值包含 `added`、规范路径 `directory` 和注册列表 `directories`。已经由项目根目录或已注册目录覆盖的路径返回 `added: false`。同一路由的 `GET` 读取注册列表。

Agena 提交新增目录并刷新文件监听后，向已启用的 Claude 插件派发 `DirectoryAdded`。事件包含当前 shell 工作目录 `cwd`、新增的 `directory` 和 `source: register_repo_root`。hook 可以按提交后的会话权限工作。返回的诊断消息和命令错误写入插件日志。[Claude Code DirectoryAdded](https://code.claude.com/docs/en/hooks#directoryadded)

`agena sessions create --setup init` 或 `agena exec --setup maintenance "检查项目"` 会先执行匹配 `init` 或 `maintenance` 的 Claude `Setup` 命令 hooks，再执行正常的 `SessionStart`。该事件沿用会话的插件配置和沙盒，hook 的返回值不影响会话启动。

Claude 的 `PreCompact` 收到 `trigger` 和 `custom_instructions`。会话压缩 API 从请求的 `system` 字段读取手动压缩要求。Claude hook 成功返回的纯文本会作为摘要要求。成功提交压缩后，Claude `PostCompact` 收到 `compact_summary`，较大的摘要会缩减预览。Codex hooks 收到对应的 `trigger`。

服务重启后，已有项目会话首次执行时触发 `SessionStart` 的 `resume`。创建普通会话分支或用分支方式回退时触发 `fork`。压缩摘要提交后触发 `compact`，普通手动压缩的上下文在下一轮对话使用，自动压缩则直接交给本轮的下一次模型请求。Codex 和 Claude 的 `SessionStart` 都可用成功返回的纯文本补充上下文。Claude hook 在 `startup`、`resume` 和 `fork` 时可读取当前自定义标题并用 `sessionTitle` 命名当前会话。

Codex `SessionStart` 返回 `continue: false` 时，Agena 会在模型请求前取消启动、恢复、分支、清空或手动压缩后的首轮对话。被取消的输入只保留在会话记录中，`stopReason` 显示在对应的 hook 活动里。Hook 追加的 `additionalContext` 随下一次实际运行的对话发送。自动压缩后的同一决定结束当前轮次，并保留已提交的压缩摘要。DeepSeek Harness 的 Codex hooks 使用相同规则。

Claude `SessionStart` 返回 `hookSpecificOutput.reloadSkills: true` 时，Agena 会在 hooks 执行完毕后刷新当前会话的文件技能和命令目录。Hook 在项目的 `.claude/skills` 和 `.claude/commands` 里生成的 Markdown 文件可以在同一会话中读取。发现和读取遵循当前会话的权限。

Claude 的 `resume` 和 `fork` hooks 可从当前可见历史中取得最近一次完成响应的时间、上下文 token 数和缓存过期估计。有可用价格时，`estimated_cache_write_usd` 给出目标模型重新写入缓存的估算费用。分支以自己的历史边界计算，已提交的压缩摘要也纳入判断。[Claude Code SessionStart](https://code.claude.com/docs/en/hooks#sessionstart)

启用导入插件后，项目会话可使用 `agena.task.create`、`agena.task.get`、`agena.task.list` 和 `agena.task.update`。Claude 角色工具列表可使用对应的 `TaskCreate`、`TaskGet`、`TaskList` 和 `TaskUpdate`。每个会话最多保存 256 个任务。任务、依赖关系、负责人与元数据会在服务重启后保留。Claude 插件和 DeepSeek 的 Claude hooks 可以阻止任务创建或标记完成。退出码 2 或 `decision: "block"` 会将原因返回给当前工具调用，并保留原任务状态。

Claude 角色可用 `TodoWrite` 调用 `agena.task.todo_write`，一次提交整份待办清单。输入使用 `content`、`activeForm` 和 `status` 字段，返回更新前后的清单。清单按项目会话保存，服务重启后可以继续更新。

Claude skill 的 hooks 在成功读取该 skill 后为当前会话生效。Markdown 命令的 hooks 在调用该命令后生效。通过 `context: fork` 启动的命令和 skill 的局部 hooks 在对应子会话运行期间生效。Agent 自带的 hooks 只匹配选中该角色的会话。Skill 和命令中的 `once: true` 在处理器首次成功后停止运行。

Claude 和 Codex 命令 hook 的 `async: true` 将结果中的上下文放入会话的待收消息。Claude 的 `asyncRewake: true` 另外在退出码为 2 时请求唤醒会话。Codex 的 `SessionEnd` 命令 hook 同步执行。Monitors 可在会话启动时运行，或在读到指定 skill 后启动。它们在会话沙盒内运行并把 stdout 逐行投递到该会话。会话结束或插件卸载时会停止相应的后台任务。

服务正常关闭时，Agena 在活动执行收尾后派发这些会话的 `SessionEnd`，清理命令返回后释放对应的会话记录文件。

### 模型切换

`PreModelSwitch` 在会话写入新的模型选择前运行，覆盖显式切换、恢复默认模型，以及提交消息时指定不同模型。插件可以返回 `allow`、`ask` 或 `deny`，多个判定按 `deny`、`ask`、`allow` 的顺序生效。超时会阻止切换。TUI 通过 `/model` 请求确认时会显示 hook 给出的原因。确认凭据只适用于同一会话版本和同一组选项，五分钟内可使用一次。普通 API 请求和模型选择器把 `ask` 当作拒绝处理。

`PostModelSwitch` 在显式选择写入后，或配置重载与恢复会话的模型变化首次生效时，异步运行。它的纯文本 stdout 或 `additionalContext` 只送给此次切换的目标模型，并在待投递期间持久化保存到会话存储，Agena 服务重启后仍可送达下一次模型请求。Agena 在模型请求准备时最多等待五秒。超时后仍在运行的 hook 可以把结果交给下一次请求。连续切换时，下一次请求只接收最近一次切换的上下文。

两个事件的 matcher 使用归一化的 Claude 模型名称。自定义网关模型无法归一化时会检查所有 matcher。成本字段使用最近一轮模型调用报告的 token 用量和可用的缓存写入价格估算。

### 命令、角色及 LSP

- 选择命令目录里的命令，或输入 `/<插件 ID>.<命令名>`。支持 Markdown 和 Claude manifest 内联 `content`，以及 `$ARGUMENTS`、`$ARGUMENTS[N]` 和 `$N`。
- Codex manifest 声明的 `onboardingSkill` 会出现在插件命令目录，可用 `/<插件 ID>.onboarding` 在当前会话运行。若包里已有同名命令，启动入口显示为 `plugin-onboarding`。
- Claude 命令和允许用户调用的 skill 可以声明 `context: fork`。调用时展开参数，用 `agent` 指定的角色启动 Agena 子 agent，并在命令结果中返回最终文本。子 agent 沿用父会话权限与工具限制。
- 普通 Claude skill 也可以从斜杠命令调用，展开参数和 `${CLAUDE_SKILL_DIR}`。设置 `user-invocable: false` 的技能按其元数据进入模型技能目录。
- 启用的 Claude 插件的 `bin/` 会加入项目会话的沙盒 shell PATH。原有 PATH 中的命令优先，插件目录使用会话已有的路径权限。
- Claude 主题从默认的 `themes/` 或 manifest 声明的 `experimental.themes` 和旧版 `themes` 读取。终端主题 ID 为 `<插件 ID>.<文件名去除 .json>`，可在 `ui.tui.theme` 选择。Agena 映射共有的颜色项，其余颜色沿用 Agena 的终端配色。
- Claude agents 的角色名是 `<插件 ID>.<agent 名>`。`tools` 和 `disallowedTools` 的整工具规则映射到 Agena 的工具目录，例如 `Read`、`Grep` 和 `Bash`。未映射的允许项会记入诊断。未映射的拒绝项会阻止导入该角色。Output styles 对应 `<插件 ID>.style.<样式名>`。
- Claude 角色的 `skills` 按名称或 `插件名:技能名` 读取有权限的技能正文，并加入该角色的模型请求。`memory`（`user`、`project`、`local`）按会话路径读取权限与非符号链接校验预加载对应作用域的 `MEMORY.md`，`background: true` 使该角色在委派启动时默认进入后台运行并在完成后通知父会话。`effort` 选择目标模型支持的同名推理档位，`omitClaudeMd` 跳过初始 Claude 项目指令，`autoCompactWindow` 可以降低自动压缩阈值。按需读取目录中的指令继续沿用会话权限。包内子角色的 `permissionMode` 和 `mcpServers` 与 Claude 一样作为忽略字段处理。 [Claude 子角色字段](https://code.claude.com/docs/en/sub-agents#supported-frontmatter-fields)
- MCP、LSP 和 exec hooks 的 `${user_config.KEY}` 可以读取声明为 `multiple` 的字符串数组默认值，模板中使用 JSON 数组。角色、技能和命令正文可展开公开选项，敏感选项显示为 `[redacted]`。Shell 片段保持自身环境与命令语义。
- Claude 的 `NotebookRead` 和 `NotebookEdit` 分别映射到 `agena.notebook.read` 和 `agena.notebook.edit`。读取支持单元格分页、已保存的输出和 PNG 或 JPEG 附件。编辑支持单元格 ID，旧版笔记本可用单元格序号。传入读取结果中的 `sha256` 可要求按指定版本编辑。
- LSP 从 `.lsp.json` 和 manifest 的 `lspServers` 合并读取，支持扩展名到语言 ID 的映射、stdio 启动、工作区配置、初始化参数和超时。Agena 授权路径并按需启动语言服务器。

### Claude Code 函数 hooks

用 `agena plugin install ./my-mod --enable-hooks` 安装带函数 hooks 的包。`hooks/hooks.json` 中的 `modules` 以声明它的 JSON 文件为基准解析。Agena 直接加载导出 `register(on, options)` 的 JavaScript 或 TypeScript ES 模块，并解析包内的静态相对导入。模块源文件变化会进入插件重载监听。`options` 读取 `userConfig` 默认值和 `--option-env key=NAME` 指定的值，布尔值和数字按声明类型传给模块。

每个项目会话有独立的模块状态。模块可以处理 `session.start`、`session.end`、`tool.offer`、`tool.describe`、`tool.call`、`tool.check`、`tool.run`、`prompt.submit` 和上文映射到 Agena 的 `classic.*` 事件，使用事件匹配器、`next(e)` 和异常恢复函数。同包的经典 hooks 构成 `classic.*` 的续体。`tool.offer` 和 `tool.describe` 可按会话过滤可见工具或改写工具描述。模型的 `tool.call` 续体经过 Agena 的参数校验、权限检查和实际执行。函数可以在执行前拦截调用或修改参数，也可以读取执行结果、补充模型上下文或替换模型可见输出。`tool.run` 在审批完成后收到最终参数，支持返回经校验的多媒体 `content` 块以及 `isEnd: true`（在工具完成后直接结束当前回合）。真实工具回执保留在会话记录中。

模块可在 `session.start` 中调用 `$.tool.register({ name, description, inputSchema })`。该工具使用 Claude 的 `mcp__包名__工具名`，可在当前项目会话通过 Agena 的工具目录查找和调用。名称重复时，使用完整的 Agena 插件 ID 加工具名。拥有该工具的模块可用 `tool.call` 或 `tool.run` 提供结果，Agena 在调用它前完成参数校验和授权。工具可以返回结构化对象和随结果发送给模型的 `context`。关闭或清空会话会释放工具，重载后当前会话会从新模块重新注册。

`$.tool.list()` 读取当前会话可用的 Agena 工具，包括模块注册的工具。`$.tool.check({ tool, input })` 查询适用的工具、路径和网络权限，以及 `tool.check` hook 的决定。查询不会打开权限对话框。`$.tool.call({ tool, ...input })` 接受已映射的 Claude 工具名和 Agena 完整工具名，沿用会话权限和沙盒。来自斜杠命令或会话启动的调用可以使用已有授权。REST 客户端也可通过 `POST /api/v1/plugins/tools/invoke`，传入 `plugin_id`、`tool`、`input` 和 `session_id`，调用该会话中的动态工具。

`$.http.fetch(url, options)` 支持 HTTP(S) 方法、请求头、文本 body、超时和重定向。返回 `status`、`ok`、`headers` 和 `text`。Agena 为每次跳转重新检查网络权限，连接获准的 IP 地址，并使用会话的沙盒设置。

模型调用工具时，`tool.check` 在 `PreToolUse` 改写和参数校验之后读取工具名、有效参数及适用的只读标记。`next(e)` 返回当前的权限判定。`deny` 拒绝该调用，`ask` 将该调用交给 Agena 的用户审批，`allow` 沿用 Agena 权限。命令型或函数型 `PreToolUse` 请求 `ask` 时也进入同一审批流程。Agena 的拒绝规则及自动审批拒绝优先于插件的批准请求。

函数中可以用 `$.process.run(argv, options)` 检查 Git 状态或运行包内程序。Agena 以参数数组启动程序，按会话策略隔离文件和网络访问。支持工作目录、额外环境变量、stdin 和超时。stdout 和 stderr 各保留 64 KiB，超过时返回截断标记。

项目用户提交消息时，`prompt.submit` 可返回 `{ drop: "原因" }` 阻止提交，或用 `next({ ...e, text, context })` 改写正文并补充模型上下文。Agena 提供提交来源、队列状态和附件元数据。附件内容沿用原消息。函数链之后运行经典 `UserPromptSubmit` hooks，`next(e)` 在消息被接收或排队后返回。改写后的正文进入对话记录，额外上下文随消息持久化并只在组装模型请求时使用。完整会话归档包含这部分上下文。变更已排队消息的内容会清除此前生成的上下文。

函数可在提交期间用 `$.prompt.media({ id })` 读取 `e.attachments` 中的图片和文档。响应附带 MIME 类型、总字节数及最多 384 KiB 的原始字节所对应的 Base64。可把响应的 `next` 作为下一次读取的 `offset`。Agena 按会话路径权限准备不可变的附件快照，模型提交时复用这份快照。附件句柄只在当次提交生效，提交结束后返回 `deny`，准备失败时返回 `unreadable`。

函数可用 `await $.prompt.submit({ text, wait, asUser })` 在当前项目会话提交纯文本。默认在会话空闲时启动新回合，运行期间先入队。传入 `wait: false` 可加入正在运行的回合，在当前工具批次完成后的模型请求中送达。如果当前回合先结束，已接收的提示词会在新回合执行。提交只跳过发起调用的函数，同包其他函数和经典处理器仍可处理或拒绝。调用在录入或入队后返回提交回执，拒绝时返回 `{ drop }`。默认给模型呈现插件署名，`asUser: true` 省略模型输入中的署名前缀，会话记录和 `prompt.submit` 事件继续携带插件来源。

在 TUI 或 Web 聊天中提交导入的 Markdown 斜杠命令时，`prompt.submit` 会在展开提示词和执行 shell 片段前运行，事件包含宿主解析的 `e.command.name` 和 `e.command.args`。例如 `name` 可为 `review:explain`。函数可以拦截提交或追加模型上下文。对斜杠文本的改写会被忽略，插件日志会记录该操作。经典 `UserPromptSubmit` 和 `UserPromptExpansion` 随后依次运行。展开发送成功后，`next(e)` 返回已接收的命令行和模型上下文。Fork 命令直接启动子 agent，父会话不触发 `prompt.submit`。

REST 客户端可读取插件命令目录中标记 `submits_prompt: true` 的条目，并在 `POST /api/v1/sessions/{id}/messages` 的消息正文旁传递 `"prompt_command": {"plugin_id": "claude.review", "command_id": "explain", "args": "src"}`。服务器按当前会话的插件配置重新解析命令。Web 插件工作台仍可单独展开命令，把结果放入可编辑的输入框。

模块可在 `session.start` 中用 `$.command.register()` 注册斜杠命令，并用 `command.run` 对该命令应答。命令显示在当前会话的 TUI 和 Web 命令目录。可以输入其原名，也可以用 `/<插件 ID>.<命令名>`。数字和 JSON 片段以原始参数文本传给 `e.args`。REST 客户端可用 `GET /api/v1/plugins/surface?session_id=<id>` 查询这些命令。插件重载后，Agena 在会话下一次读取命令目录或调用命令时运行新模块的 `session.start`，重新初始化模块变量。关闭或清空会话时释放注册项。函数另可使用日志、基础会话元数据、允许读取的环境变量以及支持 `version`/`ifVersion` 乐观并发校验的插件私有存储。私有存储在重载后仍可读取。

函数 hooks 的剩余接口包括 `ui.render`、`tool.judge`、工具目录增删 hooks、流式结果、进程流和文件服务。动态工具使用 Agena 的工具发现机制。经典事件的宿主字段和结果按 Agena 已接入的生命周期能力映射；启用进程沙盒时，`classic.*` 函数 hooks（`classic.PreToolUse` 除外）会收到当前会话的 `transcript_path`（`classic.SubagentStop` 另附 `agent_transcript_path`），并在 `$.process.run()` 的沙盒策略中以只读方式挂载这些转录文件。

### Claude 角色的 worktree 隔离

Claude agent 文件声明 `isolation: worktree` 后，通过插件的 `.run`、fork 命令或选择该角色的 workflow 启动时，Agena 为该子会话配置隔离工作副本。Git 默认副本位于 Agena 数据目录的 `worktrees/` 下。Agena 优先用本地的 `origin/HEAD` 建立副本，缺少该引用时使用当前 `HEAD`。项目位于 Git 子目录时，Agena 检查原会话对 Git 根目录的读取权限，并在副本中使用同一个相对目录作为子 agent 的工作区。显式删除时检查整个副本的 Git 状态。

默认 Git 创建流程读取源仓库根目录的 `.worktreeinclude`，按 `.gitignore` 语法复制匹配且已被 Git 忽略的普通文件。它支持目录模式、排除规则，以及 Claude 对 `**/` 匹配整个忽略目录的规则。Agena 检查清单及每个源文件的读取权限，保留新副本中已有的跟踪路径。符号链接不参与复制。会话记录保留已复制文件的内容和权限指纹，显式删除时可以清理仍保持原样的副本，修改过的文件会留下工作副本和会话。[Claude Code 的 `.worktreeinclude`](https://code.claude.com/docs/en/worktrees#copy-gitignored-files-into-worktrees)

已启用的 `WorktreeCreate` hook 可以取代 Git 创建流程。命令将路径写在 stdout 的最后一行非空文本内，HTTP hook 返回 `hookSpecificOutput.worktreePath`。相对路径以 hook 的工作目录解析。Agena 校验目录、符号链接和该会话对目录的读写权限。第一个匹配的创建处理器负责提供副本，处理器失败会终止这次启动。

子 agent 完成后保留副本，继续任务或重启 Agena 后沿用该目录。派生的对话分支返回原项目目录。显式删除子会话时，Agena 会调用对应的 `WorktreeRemove` 处理器，或按适用的 Git 规则清理。Agena 创建的 Git 副本如果含有本地文件、未提交修改或新的提交，会保留会话和副本。[Claude Code WorktreeCreate 和 WorktreeRemove](https://code.claude.com/docs/en/hooks#worktreecreate)

### Claude 消息 channels

可以把已安装的 Claude 插件的 stdio MCP 服务绑定到一个项目会话。插件 ID 来自安装结果，末尾使用插件 manifest 声明的服务原名。实例中直接配置的 stdio 服务可使用其服务名。服务需要在 MCP 握手时声明 `claude/channel`。

```bash
agena sessions channels 42 --catalog
agena sessions channels 42 --enable 'plugin:claude.example:alerts'
agena sessions channels 42
agena sessions channels 42 --disable 'plugin:claude.example:alerts'
```

`--catalog` 显示候选 stdio 服务及插件声明的 channel 配置项。`declared: false` 表示插件未提供 channel 元数据，Agena 在启用时通过 MCP 握手确认能力。`notifications/claude/channel` 以该服务署名的 `<channel>` 事件进入已选会话，空闲会话可由事件启动，活跃会话可在后续模型请求处理新事件。服务提供的回复工具通过通常的 MCP 工具目录和会话授权调用。该连接遵循 Agena 的 stdio 沙盒，服务需要的目录和网络端点应按其配置及会话权限显式授予。传输意外断开后，Agena 在会话权限仍然有效的情况下尝试重连。

Claude 插件的 stdio MCP 和技能 shell 片段默认使用同一个私有插件数据目录作为 `CLAUDE_CONFIG_DIR`。Telegram 和 Discord 等插件的配置技能可在此和服务共享 `channels/` 状态。安装时可通过 `--option-env` 指定插件选项使用的服务环境变量。插件按 manifest 声明使用相应的解释器和依赖，网络连接沿用当前会话的权限。

可信服务也可在启用时加上 `--permission-relay`，前提是服务声明 `claude/channel/permission`。Agena 用 Claude 的消息格式向同一会话的服务发送一次性工具审批请求，远程的允许或拒绝与本地对话框按先到先处理。批准只适用于对应的待决操作。热重载后，Agena 会在插件安装、服务定义和会话权限仍匹配的情况下重连，旧连接的待决审批已关闭。清空或关闭会话、重启服务或撤销权限会结束绑定。 [Claude Code Channels reference](https://code.claude.com/docs/en/channels-reference)

### Claude workflows

插件的 `workflows/` 或 manifest 指定的本地 `.js` 文件可以声明 `export const meta = { name, description }`。工具 `<插件 ID>.workflow` 接收 `{"name":"review","args":{"path":"src"}}`。也可以输入 `/<插件 ID>.workflow.review {"path":"src"}` 直接执行并查看结果。脚本在隔离的 JavaScript 引擎中运行，支持 `args`、`agent()`、`pipeline()`、`parallel()`、`phase()` 和 `log()`。`agent()` 可选择同一插件的角色，或用 `schema` 校验 JSON 结果。Fork、workflow 和插件的 `.run` 都占用父会话的 Agena 子 agent 名额，每个父会话最多创建 8 个子 agent。

Workflow 的完成结果按启动顺序保存在当前会话的插件私有存储。用 `/<插件 ID>.workflow.review --resume` 续跑最近一次，也可用 `--resume=<run ID>` 或工具参数 `resumeFromRunId` 指定当次运行。直接续跑使用保存的参数，追加 JSON 可覆盖参数。已完成且输入一致的前缀从记录重放，首个失败或输入变化的任务及其后续任务会重新执行。上一轮的子 agent 仍活跃时，等它结束后再续跑。Agena 服务重启后，可以在同一会话继续使用这些记录。

### Skills 和 MCP

导入的 Skills 可从技能库、`agena.skills` 和命令目录读取。若会话策略禁止读取插件目录，可先用 `agena plugin inspect <插件 ID>` 查找包路径，再给该目录加只读权限。MCP 服务加上插件前缀，并在会话首次使用时通过授权建立连接。支持 stdio、Streamable HTTP、旧 SSE 和 Claude WebSocket。SSE 的 POST 地址与事件流使用同一来源并独立授权。WebSocket 使用 `mcp` 子协议，在授权的连接上握手。

调用 Claude 插件中的 Markdown 命令或技能时，Agena 展开行内 `` !`命令` `` 和 ` ```! ` 围栏中的 shell 片段。支持 `$ARGUMENTS`、位置参数和 frontmatter 声明的命名参数。Bash 片段使用 Bash 解释器。Windows 上可用 `shell: powershell` 选择 PowerShell。普通代码围栏保留原样，片段的输出以原文插入一次。

带 shell 片段的斜杠命令先以原始调用文本触发 `UserPromptSubmit`，再触发 `UserPromptExpansion`。Agena 按当前项目会话的工具、命令、路径权限和沙盒配置检查所有片段，然后收集 `PreSkillShellCommand` 的判定。`deny`、`ask`、`defer` 或 `updatedInput` 跳过该片段，`continue: false` 撤销整次展开。需要自动审批的片段交给后续的普通工具调用处理。运行失败时终止展开。

模型可通过 `agena.skills.get` 加载确切的技能副本并传入 `arguments`。`context: fork` 使用展开后的正文启动 Agena 子 agent，父会话收到子 agent 的结果。`disable-model-invocation` 保留文档查看入口并禁止模型触发该技能的展开或局部 hooks。模型工具的调用会映射到 Claude hooks 的 `Skill`，`tool_input.skill` 携带 Agena 技能 ID。

Agena 的会话工具可以处理旧版 `elicitation/create`，也可以通过 MCP 2026-07-28 的 `input_required` 继续工具调用、提示词读取和资源读取。收到需要人工参与的表单或链接时，Agena 在发起调用的会话中显示对话框。单字段的布尔值和小型选项列表可直接选择，复杂表单支持 JSON 输入。链接可以在浏览器中手动处理。用户应答和 hook 覆盖值都经过 schema 校验。服务提前结束调用时，Agena 会关闭对应的输入请求。

Codex 的 `openai/form` 和 `openai/elicitation/create` 表单也使用同一交互。Agena 将 `openai/imagePicker` 的标题和 ID 映射为选项，提交选中的 ID。没有响应流标识的旧版连接在只有一个在途请求时才能将服务端主动发起的交互归到该调用。

MCP 工具的异步任务沿用原工具调用的授权。MCP 2026-07-28 使用 `io.modelcontextprotocol/tasks` 扩展，协商后按服务端给出的间隔查询任务。工具结果和错误随最终状态返回给当前调用。任务可以在运行中请求表单、链接或工作区根目录。Agena 用 `tasks/update` 提交应答，并按请求 ID 避免重复弹出同一表单。[MCP Tasks 扩展](https://modelcontextprotocol.io/extensions/tasks)

MCP 2025-11-25 在服务和工具都声明 Task 支持时启用异步调用，通过 `tasks/result` 取得结果。Agena 按任务 ID 路由进行中的表单，并校验 HTTP 响应流所属的调用。两代协议均支持在配置超时内轮询。等待用户应答的时间单独计量。超时或调用取消后，Agena 在原连接上尝试向支持取消的服务发送 `tasks/cancel`。服务返回任务 ID 后，后续错误会结束当前调用，防止重复启动同一个工具。[MCP 2025-11-25 Tasks](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks)

`sampling/createMessage` 支持有会话工具调用授权的旧版 MCP 回调和 MCP 2026-07-28 的 `input_required`。采样支持文本、经 MIME 与魔数校验的多媒体（`image`、`audio`）、服务声明的 `tools`/`toolChoice`（含 `tool_use` / `tool_result` 多轮工具调用循环），以及 `includeContext`（`thisServer` / `allServers`，按有界字节预算汇总会话上下文）。合并后的上下文、媒体摘要和工具声明都会在调用模型前完整预览给用户确认；模型完成后，产生的文本或工具调用也会再次由用户确认后才交还给 MCP 服务。Agena 使用已配置的辅助模型或会话模型，限制输出到请求上限和模型上限以内。 [MCP Sampling](https://modelcontextprotocol.io/specification/2025-11-25/client/sampling)

Claude 远程 MCP 的 `headersHelper` 在连接时从插件目录内通过 Agena 沙盒运行。命令需要在 10 秒内输出字符串值组成的 JSON 对象，同名动态 header 覆盖静态值。使用 Streamable HTTP 或旧 SSE 调用工具时，如果在交互开始之前遇到 HTTP 401 或 403，Agena 会重新运行 helper、重新连接并重试一次。

Claude `mcpServers` 可引用带 `server.mcp_config` 的本地 `.mcpb` 和 `.dxt`。从 marketplace 安装时，Agena 会一并下载在 manifest 中声明的远程 bundle。Agena 将其解包至实例缓存，应用当前平台的覆盖配置，再按 bundle 声明的 stdio 命令启动。插件目录变量、Claude 标量用户选项和选项环境变量也可用于 bundle。

Codex 的 `startup_timeout_sec` 和旧版 `startup_timeout_ms` 约束连接与首次工具发现，`tool_timeout_sec` 约束单次工具调用。Agent Plugins v1 包可以在同名 Codex 覆盖条目中声明这些字段。Agena 在工具目录和实际调用入口都按原始工具名应用 `enabled_tools` 和 `disabled_tools`。明确指定空的 `enabled_tools` 会关闭该服务的全部工具，禁用名单优先。

Claude 每个 MCP 服务的 `timeout` 支持至少 1000 的整毫秒值，适用于工具及该调用创建的异步任务。Agena 的会话执行上限仍适用。[Claude Code MCP 配置](https://code.claude.com/docs/en/mcp)

声明 `io.modelcontextprotocol/skills` 的 MCP 服务可以通过 `mcp.skills.list` 分页发现技能。`server` 可填 Agena 服务名，或者 `plugin:<已安装插件 ID>:<manifest 服务名>`。已知技能的 `SKILL.md` URI 也可直接交给 `mcp.skills.read`。有固定清单的技能会通过 `skills/get` 公布文件清单。用户确认本次读取后，Agena 校验文件 URI、字节数和 SHA-256，还会核对 `SKILL.md` 的 YAML frontmatter。读取配套文件时，传入同一服务、技能 URI、`file_uri` 和上次读取返回的 `version`。清单变化后要重新读取 `SKILL.md`。`mcp.resources.directory.list` 可查询声明 `directoryRead` 的服务。技能内容以带来源标记的工具结果返回，工具权限继续由 Agena 会话决定。[MCP Skills 扩展](https://modelcontextprotocol.io/extensions/skills/overview)

服务用 `resources: "dynamic"` 声明生成式技能。Agena 在目录里标记 `dynamic: true`，每次读取都请求当次同意，并核对 `SKILL.md` 的 frontmatter。可直接读取已知技能 URI，也可以通过 `mcp.resources.directory.list` 发现配套文件。读取动态配套文件时只需提交同一 `server`、技能 `uri` 和目录内的 `file_uri`。Agena 在结果中保留来源和动态标记。[MCP Skills 规范](https://github.com/modelcontextprotocol/ext-skills/blob/main/specification/beta/skills.mdx)

### Web 中的 MCP Apps

Agena 通过 `io.modelcontextprotocol/ui` 协商 MCP Apps。模型调用带 `_meta.ui.resourceUri` 的工具后，Web 会话在对应工具回执旁显示「打开应用」。打开时，Agena 重新检查会话对原 MCP 服务的权限，再读取指定 `ui://` 资源。应用收到引发渲染的工具参数和结果，通过同一服务的 `tools/list`、`tools/call` 和 `resources/read` 交互。`tools/list` 按 `nextCursor` 分页返回当前服务配置允许且对应用可见的工具描述。工具的 `visibility` 分别约束模型和应用。应用仅在拥有原工具回执的会话内运行。

应用可通过 `resources/list` 和 `resources/templates/list` 分页浏览原服务的资源和模板。资源目录列出服务公开的普通资源和当前应用的界面资源。模板目录提供普通资源模板。两项调用沿用当前会话的工具和传输权限。 [MCP Apps SDK](https://apps.extensions.modelcontextprotocol.io/api/classes/app.App.html#listserverresources)

Agena 向应用声明 `serverTools.listChanged`，并在应用可见的工具目录变化时发送 `notifications/tools/list_changed`。stdio、旧 SSE 和 WebSocket 在服务声明支持通知时监听原连接。Streamable HTTP 和未声明通知的服务通过定期刷新目录检测变化。每次刷新重新检查当前会话权限和原工具回执。应用关闭后，Web 会结束目录观察。 [MCP Apps 协议](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/draft/apps.mdx#capability-negotiation)

应用获得资源目录读取权限后，Agena 可声明 `serverResources.listChanged`。stdio、旧 SSE 和 WebSocket 监听原连接上的资源变更通知。Streamable HTTP 2026-07-28 使用 `subscriptions/listen`，较早的 Streamable HTTP 协议使用原 MCP 会话的 SSE GET 流。Agena 在应用发起观察时才打开 Streamable HTTP 接收流，观察结束时关闭它。收到 `notifications/resources/list_changed` 后，Agena 重新检查会话权限和原工具回执，再通知应用刷新目录。

对于使用轮询的 Streamable HTTP 服务，Agena 按应用可见的资源描述比较目录，最多扫描 16 页，超出扫描范围的目录可继续通过服务端通知更新。无状态应用可在 Agena 重启后从原工具回执重新打开，继续观察目录。

在 `ui/initialize` 声明 `tools` 的应用可以把浏览器界面里的工具提供给当前 Agena 会话。Agena 通过应用的 `tools/list` 读取定义，模型可用 `tools_list` 查找带 `mcp_app__` 前缀的工具，再用 `tools_help` 和 `tools_call` 调用。调用沿用 Agena 的工具授权，权限配置使用 `agena.mcp.apps.call`。应用发送 `notifications/tools/list_changed` 后，模型可查看更新的目录。

调用在对应 iframe 执行，参数和最终结构化结果按应用声明的 schema 校验。每个应用最多登记 50 个工具，工具目录、单次参数和单次结果各限 128 KiB。工具初次应答最多等待 30 秒。关闭应用、清空会话或重新载入插件会撤回工具并结束等待中的调用。浏览器意外断开时，登记在 40 秒内失效。

应用发布的工具可以通过 `execution.taskSupport` 声明旧版 MCP Tasks 支持。Agena 向该视图发送带 `task` 的 `tools/call`，再用 `tasks/get` 和 `tasks/result` 取回最终输出。应用在 `ui/initialize` 声明 `io.modelcontextprotocol/tasks` 扩展时，Agena 也能识别 `resultType: "task"` 并从新版 `tasks/get` 读取结果。任务和取消操作绑定到启动它的视图及任务 ID，单次调用合计最多等待五分钟。任务在视图仍在线时超时或被取消，Agena 会尝试发送 `tasks/cancel`。[MCP Apps 草案](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/draft/apps.mdx#long-running-app-tools)

新版任务返回 `input_required` 时，Agena 通过当前会话呈现表单或 URL 确认，并支持沿用会话采样权限请求文本模型输出。采样前和交回结果前均需用户同意。Agena 用 `tasks/update` 把响应按请求 ID 送回原视图。单个任务最多处理 128 项交互，同一个请求 ID 只处理一次。关闭视图或取消调用后，仍待处理的交互会随之结束。[MCP Tasks 扩展](https://modelcontextprotocol.io/extensions/tasks/overview)

应用发布的普通工具也可以直接返回 `resultType: "input_required"`。Agena 在发起调用的会话中处理表单、URL 确认和文本采样，再向原视图发送带新请求 ID 的 `tools/call`，保留原工具名和参数，按请求 ID 提交应答并原样回传本轮的 `requestState`。仅携带状态的结果按退避间隔重试。一次调用最多发送十轮 `tools/call`，视图工具及其本地任务合计最多处理 128 项交互。续接后可以直接返回结果，也可以移交给已协商的 Tasks。Agena 用工具声明的 schema 校验最终结果。关闭视图或取消调用会结束仍在等待的会话交互。[MCP 多轮请求](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr)

普通 `callServerTool()` 调用由 Agena 在原连接跟踪服务返回的 Tasks，并向应用交付最终工具结果。Web 在等待期间用应用提供的进度 token 发送心跳，使支持进度续时的 SDK 可以继续等待。

应用的 `tools/call` 和 `resources/read` 在原连接处理 MCP 2026-07-28 的 `input_required`，也可接收旧版的会话内回调。Agena 在 Web 应用卡片中显示表单或链接确认，再向原服务提交结果。续接需要的 `requestState` 和服务端请求 ID 由 Agena 保管。等待用户应答的时间与 MCP 网络请求的超时分开计量。关闭视图会结束这些请求。

Web 声明 `hostCapabilities.sampling`。应用可用 `sampling/createMessage` 请求 Agena 生成文本。Agena 使用当前会话配置的辅助模型或会话模型，按模型和请求设定输出上限，采样输入来自应用提供的文本。应用卡片先展示请求供用户确认，生成后再展示结果供用户确认交回。普通服务调用在续接期间请求采样时，沿用同一交互流程，结果交回原服务。[MCP Apps Sampling](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/draft/apps.mdx#standard-mcp-messages)

应用声明新版 Tasks 扩展，或为旧版 `tools/call` 显式传入 `task` 时，可在服务创建任务后立即得到一个视图范围的代理句柄。应用可查询或取消自己持有的任务。新版应用通过 `tasks/get` 读取状态、待答请求和最终结果，并通过 `tasks/update` 分批提交表单、URL 确认或文本采样应答。Agena 校验应答并在原连接转交，工作区根目录请求由 Agena 处理。后端使用旧版 Tasks 时，Agena 把 `tasks/result` 期间收到的回调转换为新版应用的待答请求，HTTP 回调回复沿用原任务的网络授权。旧版应用在等待 `tasks/result` 时，可用自己的 JSON-RPC 处理器接收对应回调并返回应答。[MCP Tasks 扩展](https://modelcontextprotocol.io/extensions/tasks/overview)

应用把代理句柄作为当前模型工具的返回值时，Agena 接管调用。已经接受的应答继续沿原连接转交，剩余输入在当前 Agena 会话中呈现。关闭 iframe 后，Agena 会继续跟踪这次调用，并按应用工具的 schema 校验最终结果。句柄只能交给同一视图正在执行的一个工具调用。视图关闭时，Agena 停止跟踪未交接的任务，并在服务支持时发送取消请求。每个视图最多保留 16 个句柄，保留时间最多五分钟。[MCP Apps 草案](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/draft/apps.mdx#long-running-app-tools)

网页为应用使用两层隔离 iframe，支持初始化握手、尺寸变化、主题、语言，以及应用声明的内联、全屏和画中画模式。用户或应用可以发起关闭，Agena 会请求应用完成清理并等待应答。应用可以在卡片中请求打开 HTTP(S) 链接、发送消息或加入模型上下文，用户确认后生效。

Agena 优先使用 `resources/read` 内容项的 `_meta.ui`。内容项省略该字段时，Agena 用最多五秒在 `resources/list` 中查找同一资源的静态声明，最多扫描 16 页。经会话网络权限允许的精确来源及严格 TLS 子域通配来源（`https://*.example.com`，`connectDomains` 另支持 `wss://*.example.com`；拒绝裸 `*`、明文通配、IP、本地保留域及宿主域自身，且需当前会话允许无限制出站 HTTPS）以及声明的设备权限（`camera`、`microphone`、`geolocation`、`clipboardWrite`）会列在打开界面。用户可按本次打开选择启用这些站点和权限，或使用默认限制；仅在用户明确批准后才会写入 iframe `allow` 属性和 `hostCapabilities.sandbox.permissions`。嵌入的 iframe 可以与应用交互，Agena 的消息通道绑定到最初加载的应用文档。 [MCP Apps 协议](https://modelcontextprotocol.io/extensions/apps/overview)

`ui/message` 和 `ui/update-model-context` 支持文本、图片、音频及包含这些内容的 MCP 资源。对于资源链接，用户可以通过原 MCP 连接读取并预览实际内容，再确认发送或添加到上下文。每次最多四个媒体文件，解码后合计 2 MiB。确认后沿用普通会话的附件和模型权限校验。用户批准的模型上下文可以在发送前移除，发送失败时可重试。应用的 `notifications/message` 进入限流的浏览器控制台。

应用可用 `ui/download-file` 请求下载最多四个内嵌文本、二进制或 MCP 链接资源。链接在用户确认后通过原会话的 MCP 资源入口读取。每个文件在卡片中单独点击保存。

使用 Streamable HTTP、旧 SSE 或 WebSocket 的有状态应用沿用原模型工具调用的 MCP 连接。Agena 为每次应用请求重新检查当前会话的工具、网络和沙盒权限。旧 SSE 和 WebSocket 的服务端回调使用当前进行中的请求授权。应用操作遇到明确的网络策略拒绝时关闭原连接。会话清空、关闭、配置重载或闲置 15 分钟也会释放保留的连接。有状态连接释放后，重新调用模型工具可创建新的应用实例。

使用会话范围授权启动的 stdio 应用绑定到产生工具回执的进程连接。Agena 在打开界面、读取资源或调用工具时重新计算会话的进程沙盒权限。进程的工作目录、附加目录及沙盒策略均匹配时，应用可以读写原进程中的状态。进程退出、权限变化、会话清空或配置重载后，旧回执里的应用会提示连接不可用。再次运行原工具会生成与新连接绑定的回执。仅获当次授权的 stdio 工具正常返回结果，进程随调用结束。

持久数据位于实例的 `plugin-data/agent-plugins/` 中。需要写入时可通过 Agena 的路径权限授予对应目录的权限。[工具沙盒](sandbox.md)

### DeepSeek Harness 配置

Agena 依次合并 bundle 内的 patch，并对已映射组件的配置和禁用条件求值 `!!js` 表达式。表达式可读取显式启用的环境变量和平台信息，结果必须可转换为 JSON。某个条目的表达式求值失败时，Agena 记录该条目的错误，并继续导入有效条目。`dsh-lsp-stdio` 的 `config.servers` 注册到 Agena 的 LSP 目录，进程由 Agena 按会话授权启动。`dsh-skill-filesystem` 的 `customSkillDirs` 和 `bundledSkillDir` 可使用包内相对路径，加载这些位置中的 `SKILL.md` 技能包。

Agena 读取 `engines.dsh`，缺少时检查 DeepSeek Harness 的 peer 依赖版本声明。当前桥接参考版本为 `0.2.2-rc.5` 和 `0.2.3-alpha.2`。版本区间与这两个版本均不匹配时，安装和校验会给出诊断，已识别的组件仍可导入。[DeepSeek Harness npm 发布版本](https://www.npmjs.com/package/@deepseek-ai/dsh?activeTab=versions)

对 DSH 基础配置中 `skill-filesystem` 的覆盖也能登记包内技能。表达式的 `baseUrl` 指向当前包目录，可用 `new URL()` 和 `process.getBuiltinModule('node:url').fileURLToPath()` 解析路径。解析出的技能路径受包目录边界约束。

Cordis 模块可以由包的 `exports`、patch 内的相对路径或 `dependencies` 中的包名声明。DeepSeek 安装器从同一 npm registry 获取普通依赖和可选依赖，校验 tarball 摘要及包名、版本，然后将依赖与主包一起发布到缓存。支持 npm alias、本地包内已有的 `node_modules` 和不同层级的依赖版本。安装或预览会报告无法获取的依赖。

Agena 在有执行时间和内存上限的 JavaScript 引擎中运行 Cordis 模块，支持 `apply`、默认导出的函数和类，以及数组或对象形式的 `inject`。模块可使用 `ctx.commands.register()` 并导入包内或依赖包 `exports` 提供的 ESM。`@deepseek-ai/dsh-llm` 的 `createUserMessage()` 和 `@deepseek-ai/dsh-commands/brand` 的命令标识构造函数可直接导入。`@hawk2048/oh-my-dsh` 安装后可输入 `/deepseek.hawk2048-oh-my-dsh.omd-plan <任务>`。

含有 `skills` 注入的模块可以使用 `ctx.skills.registerProvider()` 注册提供者，或用 `ctx.skills.register()` 贡献内联技能。Agena 调用提供者的 `list()` 和 `get()`，传入查询工作区的 `cwd`，并按同一包内的 `rank` 处理重名技能。项目会话的技能选择器和模型工具查询与 Cordis 命令和工具共享模块状态，按会话隔离。技能库的工作区查询启动独立模块，适用于尚未选定会话时的浏览。模型通过 `agena.skills.list` 和 `agena.skills.get` 读取允许模型调用的条目。包内目录可用作技能资源目录，文本资源沿用 `agena.skills.read_resource` 的路径权限。URL 和 opaque 资源基准作为提示交给调用者。内置模块支持 `node:url`（含 `pathToFileURL`、`URLSearchParams`）、`node:path`、`node:buffer`（及全局 `Buffer`）、`node:crypto`（`randomUUID`、`randomBytes`、`createHash`、`timingSafeEqual`）、`node:events`（`EventEmitter`）、`node:util`、`node:assert` / `node:assert/strict`、`node:querystring`、`node:os`（受限静态平台信息）、`TextEncoder`/`TextDecoder`、`structuredClone`，以及受包目录边界约束的 `node:fs/promises`（读取 UTF-8 或二进制包内文件）。

Web 和 TUI 用户可在输入框选择允许用户调用的 Cordis 技能，也可以在项目会话消息正文中直接输入 `/技能名` 触发已启用 DeepSeek 插件声明的允许用户调用技能（同时支持 Cordis 注册技能与包内磁盘 `SKILL.md` 技能；同名时 Cordis 定义优先于磁盘并先选定定义再核对 `user-invocable`）。磁盘技能的解析遵守当前会话的路径读取权限与插件开关。路径片段、URL 与 `//技能名` 不会触发；TUI 句首 `//技能名` 会去掉前导 `/` 并仅将句首视为字面文本，句中其他 `/技能名` 仍正常解析；同一消息同时用芯片选择和 `/技能名` 触发会自动去重。Agena 核对提供插件、技能 ID、内容修订号和会话开关，将说明和资源位置附到模型输入。仅允许用户调用的条目也可通过这条路径使用。已提交的说明随消息保存，重试时沿用同一份内容。编辑排队消息时，Agena 按编辑时刻重新解析和核对保留或新增的技能，移除芯片或 `/技能名` 时也移除其说明。单个技能说明块最多 100,000 字节，同一条消息的技能说明合计最多 200,000 字节。

同一 Cordis 模块同时提供技能和监听 MCP 状态时，Agena 根据各自的会话权限启动这两项能力。用户授予 MCP 状态权限后，Agena 在下一次查询技能或调用命令时为原模块接上监听。

`agena plugin install deepseek:@deepseek-ai/dsh-experimental-badge-skill-bundle@alpha` 安装官方 DSH Badge bundle。安装成功后在技能库里搜索 `dsh-badge`，即可查看说明和素材目录。 [DeepSeek Harness DSH Badge](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/experimental/skill-badge)

`ctx.tools.register(defineTool(...))` 把工具登记到 Agena 的会话工具目录。支持嵌套参数 schema、JSON 输入与输出、文本及多媒体渲染（`image`、`audio`、`document`/`file` 块，支持 base64、`Buffer`/TypedArray、data URL 与 `source` 结构，按文件头、MIME 和图片尺寸校验后作为模型路由绑定的附件交付，单次最多 8 个、单个最多 4 MiB、合计最多 8 MiB），以及按模型步骤条件激活（`options.when` 与 `deferLoading`，按实时会话模块状态在工具目录暴露和调用前求值拦截）。工具记录可保存原始 JSON 对象、数组、字符串、数字和布尔值。工具调用与斜杠命令共享同一会话的模块状态。模块还可声明 `inject: ['llm']`，通过 `ctx.llm.complete()` / `generate()` / `chat()` 使用当前会话或辅助模型执行有界生成（受输入字节、消息数、输出 token、超时及辅助任务并发上限约束）。`ctx.systemPrompt.section()` 和 `.variable()` 在用 `--enable-hooks` 安装后生效，模型请求读取实时状态并按包内顺序追加段落，遵循会话插件开关；将段落设为 `complete: true` 且返回非空文本时可完整替换系统提示词，多个激活的 `complete: true` 段落会拒绝组装。

声明 `input.attachments: true` 的 Cordis 命令可在 Web 和 TUI 的项目会话中接收附件。命令按选择顺序收到文件名、字节数和内容标识，栅格图片还带有从实际内容验证的 MIME 类型和尺寸。每次最多接收八个附件，单个不超过 20 MiB，合计不超过 40 MiB。命令可以直接返回状态，此时无需选择模型。也可以通过 `invocation.agent.followup()` 提交一条用户消息，自行选择要转发的原附件及其顺序。Agena 在同一个会话里提交该消息，普通文件使用会话内的只读副本，图片遵循所选模型的输入能力。命令或提交失败时，输入框保留原草稿，已有新输入时可恢复原草稿。

`dsh-mcp-client` 启用 `expandTemplates: true` 时，可以用 `${credential:NAME}` 读取安装时通过 `--env NAME` 声明的 Agena 服务环境变量。模板可以用于 HTTP 请求头、URL 路径和查询参数，以及 stdio 参数和环境变量。端点的协议、主机、端口和 stdio 可执行文件及工作目录使用静态配置。Agena 在 MCP 返回值和错误里遮蔽已解析的凭据及其 URL 编码形式。

`dsh-mcp-manager` 从 `config.servers` 读取 stdio 和 Streamable HTTP 服务。包可以覆盖默认的 `mcp-manager` 条目，也可以在包内 Cordis 模块的 `apply` 中调用 `ctx.mcpManager.register(name, config)`。声明式条目和注册条目都可使用 credential templates，遵循会话授权。名称冲突时优先使用已有声明，包括已禁用的条目。Agena 会在包重载时重新读取定义。`startupTimeoutMs`、`toolCallTimeoutMs`、`closeTimeoutMs` 同样适用于 `dsh-mcp-client`，分别约束启动与首次发现、单次工具调用及连接关闭。

Cordis 命令中的 `ctx.mcpManager.list()` 只读取当前项目会话允许访问的 DeepSeek 管理服务状态。列表先返回配置里的声明，再返回模块的注册项，保留配置中禁用的服务。唯一的服务名沿用原名。重名服务显示为 `plugin:<Agena 插件 ID>:<服务名>`，命令还可以用原名管理自己包中的同名服务。探测和重启使用服务声明所在包的配置及当前命令的会话权限。

`probe(name, options)` 通过独立连接协商协议并读取原始工具名，禁用条目也可探测。它接受 `timeoutMs`、`maxPages`、`maxToolNames` 和 `signal`，单独报告探测结果与关闭结果。探测默认最多耗时 10 秒、读取 16 页和保留 512 个工具名，单次探测的 Agena 上限为 60 秒、128 页和 4096 个工具名。

`restart(name)` 按当前会话的授权重新建立连接，禁用条目继续显示为 `disabled`。启用 Agena MCP 后台重连时，项目会话中的 `restart` 可以建立会话级连接。stdio 使用当前沙盒策略，HTTP 每次请求重新检查网络授权。后台重连遵循各服务的 `reconnect.enabled`、`initialDelayMs`、`maxDelayMs` 和 `maxAttempts`，默认分别为 `true`、500 毫秒、30 秒和 10 次。重连预算耗尽后可用 `restart` 重新开始。单次调用的沙盒授权随调用结束而失效。会话清空、删除或配置重载会结束会话级连接。[DeepSeek Harness MCP Manager](https://github.com/deepseek-ai/deepseek-harness/blob/4b399c39a6d1479cd84ca591b4d7c88e24d38a96/packages/mcp/mcp-manager/README.md)

普通 MCP 工具调用、工具搜索、资源和提示词操作可以用项目会话的持久授权建立并复用受 DeepSeek 管理的 Streamable HTTP 会话。每次请求同时检查调用和连接的网络权限。仅对当前调用授权时，连接随调用结束。移除服务声明后，`list()` 会在关闭期间或关闭未确认时保留 `configured: false` 的记录。原连接关闭未确认时，恢复声明也会阻止重新打开同一服务。

`ctx.on('mcp-manager/status', listener)` 的闭包驻留在当前项目会话的 Cordis 模块中，命令结束后还能接收状态更新，后续命令可读取同一模块状态。`ctx.once()` 接收一次更新，注册函数也会返回可提前解除监听的函数。会话开始时可通过 `--enable-hooks` 启动监听，直接调用模块命令也会启动监听。后台监听可读取 `list()`，主动执行 `probe` 和 `restart` 需要正在运行的命令。`watch(signal)` 先返回完整列表，随后返回状态和可见条目的变化，随命令、配置重载或 `AbortSignal` 结束。`AbortSignal.any()` 可合并调用方取消与超时。JavaScript 执行时间和宿主 I/O 分开计量。

`dsh-mcp-client`、`dsh-mcp-manager` 的配置条目和 Cordis 注册都支持 `toolLoading: on-demand`。默认的 `eager` 模式通过 `mcp.tools.search` 提供完整的工具目录。按需模式在系统提示词中展示原始工具名和简短描述。模型通过 `tools_help` 查看 `mcp__<serverName>__load_tools` 的用法，再用 `tools_call` 传入 `{"tools":["原始工具名"]}`。加载器会先校验整个列表，再注册请求的工具。加载结果会标明本次新增和已经加载的工具。`dsh-mcp-client` 可用 `catalogDescriptionMaxLength` 调整每项简介的长度，默认为 160。

加载后的工具在当前 Agena 会话的 `tools_list` 和 `tools_help` 中提供完整 schema。服务目录更新时，Agena 会更新已加载工具的定义，并撤回服务已删除的工具。新建、分支、清空或在新进程恢复的会话都从空选择开始。多个安装包使用相同服务名时，Agena 会分配不同的别名来区分加载器。需要在工具调用中授权连接时，可以用 `mcp.tools.search` 加上 `on_demand_catalog: true` 请求目录，再调用加载器。目录较长时，将返回的 `next_offset` 传给下一次搜索的 `catalog_offset`。[DeepSeek Harness MCP 按需加载](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/mcp/mcp-client/README.md#on-demand-tool-loading)

`dsh-hooks-claude-code` 支持 `pluginRoot` 和 `projectDir`。相对路径分别以 bundle 目录和会话工作目录为基准。两个值用于补齐 hook 进程的 Claude 环境变量，进程在会话工作目录的沙盒中运行。只写 `command` 的 hook 按命令处理器导入。

`agena plugin install deepseek:@dreamor/dsh-memvault --enable-hooks` 启用 MemVault 自动集成。`attach` 模式连接配置的 MCP URL，`spawn` 模式以 stdio 启动插件依赖的 `memvault-proxy`。Agena 可读取 `memory://session-inject` 组装系统提示词，并在项目会话成功提交回合后，将该回合的用户文本和助手最终文本发送给 `notify_response`。MCP 调用沿用会话的工具、进程、网络和路径权限。可用 `injectOnAssemble` 和 `extractOnTurnEnd` 分别控制两个行为。

将 `inferOutcomeOnSessionEnd` 设为 `true` 后，显式删除项目会话会调用 `infer_outcome`。Agena 从已保存的会话记录读取最近一次清空上下文后的用户任务、工具重试、拒绝、纠正与最终状态，再把有限长度的上下文交给当前包的 MemVault 服务。重启前保存的记录也会计入。并发删除由同一会话的删除流程串行处理。

### Cordis 存储和工作流

Cordis 模块可以通过 `ctx.storageDomain.open(spec)` 或 `ctx.storage.domain.open(spec)` 打开域，也可以调用 `ctx.storage.backend.get(name).kv.open(descriptor)` 直接操作记录。`dsh-storage-domain` 的 `backend` 和 `routes` 可以把同一个包的不同域交给 JSON 或 SQLite 后端。Agena 以实例和已安装插件 ID 隔离数据。`root`、`path` 及 `dshHomePath()` 表达式标识该包的存储。配置 `path: ":memory:"` 时，SQLite 内容随当前插件运行时释放。

JSON 单文档域沿用已保存的数据。`per-record` 域在 Agena 的私有 SQLite 数据库里按行保存每条 JSON 记录和版本，可启动迁移已接受的旧单文档记录。当前版本和 `compatibleVersions` 决定各条记录是否可读。`backupRecord()` 和 `invalidRecords: backup-and-skip` 保留原记录的备份后，将其从活跃记录中移出。`dsh-storage-sqlite` 使用单元级版本及 SQLite 事务，支持 `wal`、`delete`、`truncate` 和 `persist` journal 模式。插件可在会话及进程重启后继续读取磁盘数据。

域支持表、全局值和写入成功后的 `domain/changed` 事件。并发写入同一个单元发生冲突时，重新调用命令可读取最新快照。JSON 单文档存储每个单元最多 128 KiB，每个命名空间最多 64 个单元，总快照上限为 512 KiB。逐记录 JSON 和 SQLite 各支持 64 个单元、4096 条活跃记录、每条 128 KiB 和总快照 2 MiB。

包含 `@deepseek-ai/dsh-tool-workflow` 的包可以调用 `<插件 ID>.workflow`，传入 `script`、`meta` 和可选 `args`。Cordis 工具可通过 `ctx.workflowEngine.start()` 在当前项目工具调用中运行前台脚本。`agent()` 通过 Agena 启动受会话权限约束的子 agent，支持 JSON Schema 结果及每个子任务的模型选路。`dsh-workflow-ptc` 可配置 `provider`、并发数、子任务总量、批次大小和初始 JavaScript 执行上限。

`provider` 默认为 `spawn`，每个子 agent 从自己的任务开始。选择 `fork` 时，Agena 在每次创建子 agent 时冻结父会话已完成的回合，当前运行和排队中的输入排除在外。父会话清空上下文后，子 agent 从清空后的回合开始继承。前台和后台工作流都支持这两种模式。包可通过 `dsh-subagent-fork-in-process` 或 `dsh-subagent-spawn-in-process` 的 `providerName` 声明名称，Cordis 代码也可用 `subagentProvider` 为单次运行选路。工作流只收集子 agent 自己的输出。Agena 原生 `agena.agent.spawn` 可用 `inherit_parent_context: true` 启用相同的继承方式。[DeepSeek Harness fork provider](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/subagent/subagent-fork-in-process)

传入 `"run_in_background": true` 时，`workflow` 返回 `{kind: "background", jobId, runId}`。Agena 接管这次运行和它启动的子 agent，原工具和原模型轮次结束后仍继续执行。包可声明 `@deepseek-ai/dsh-tool-jobs` 配置等待时长、`quiet` 或 `wakeup` 完成通知、连续唤醒次数和内联输出预算。只声明启用后台模式的 workflow 工具时，Agena 自动注册这三个控制工具。`enableRunInBackground: false` 会关闭该 workflow 工具的后台参数。

- `<插件 ID>.job_list` 列出当前会话、当前插件的运行中任务和最近的已完成任务。结构化结果的 `jobs` 字段是列表。
- `<插件 ID>.job_output` 接收 `{"job_id":"…","wait":true,"timeout_ms":30000}`。结果包含工作流状态、上次成功读取后的已持久化进度，以及首次读取时的最终结果。等待超时会返回当前状态。有新用户消息排队时，等待提前结束，工作流继续执行。
- `<插件 ID>.job_kill` 接收 `{"job_id":"…","reason":"…"}`，请求结束工作流及它仍在运行的子 agent。

未读输出和最终结果可在 Agena 重启后继续读取。工具结果或内联完成通知提交后会保存读取位置。首次结果已出现在会话历史中的任务，后续查询仍可查看状态。服务异常退出时，运行中的任务会记录为中断。会话停止、清空、删除和所属插件卸载会结束运行中的任务。后台任务启动时按会话权限检查子 agent 委派。每个子任务启动前重新检查当时有效的权限。

Cordis 工具和项目会话命令都可以注入 `jobs`，用 DeepSeek Harness 的 `ctx.jobs.start(JobSpec)` 同步取得 `<kind>-<16 位十六进制>` 任务 ID。Agena 先检查当前项目会话、控制器与后台名额，再调用 `run(job)`，启动成功才登记任务。用 `agena plugin install deepseek:<npm 包名>` 或 `agena plugin install <本地包目录>` 安装即可。同一项目会话中的任务归属发起插件。`JobSpec.owner` 在工具里可用 `exec.agent.id`，在命令里可用 `agent.id`，省略时归属当前会话。

同一次命令启动的后台任务共用一条会话记录，各自保留任务 ID、正文和日志。命令返回后，仍可通过 Cordis 注册表或 `job_list`、`job_output`、`job_kill` 查询和控制这些任务。启用 `wakeup` 时，完成通知可唤醒尚未发送过消息的项目会话，模型会收到系统通知。重启后仍可按原任务 ID 查阅已保存的输出。

项目子 agent 也可以启动 `JobSpec` 和本地后台工作流。作业归属发起它的子会话，子 agent 结束或取消本次执行时，Agena 先收尾该次执行仍持有的作业，再报告子 agent 的结果。工作流内的 `agent()` 调用遵守当前会话的委派权限。

包可以声明 `dsh-tool-jobs`，使用 Agena 提供的 `job_list`、`job_output`、`job_kill` 和完成通知。如果包有后台工作流或 Cordis 任务生产者，而且没有自带控制器，Agena 会加载这组工具。自带控制器的模块在 `apply()` 中调用 `ctx.jobs.attachController(name)`，可为其会话提供控制能力。同名注册独立生效，各自返回解除注册的函数。会话结束会释放模块的注册。解除最后一个控制器后，已有作业还能查询和取消，新的启动请求会在调用 `run` 前被拒绝。

`run(job)` 返回的 `done` Promise 和 `cancel(reason)` 回调保存在会话模块里。任务可以在发起它的工具或命令返回后继续用 `node:timers/promises` 或定时回调推进，调用 `job.append()`、`job.updateProgress()`，也可以用 `output` 声明增量拉取源。Agena 在注册后立即拉取一次，并在结算前完成最后一次拉取。每个作业在会话记录中的正文和日志各有 128 KiB 捕获预算。Cordis 作业注册表另外保留原始分块，控制工具和完成通知从注册表取得增量输出和结果。插件作业的 `log` 通道供观察者和用户查看。Agena 的 `workflow` 控制工具继续在输出中显示工作流进度。重启后可按原任务 ID 查询已保存的输出和中断状态。

拉取源可在 `read(fromByte)` 返回值中用 `spillPath` 指向它保存的完整输出文件。作业视图的 `output.spillPaths` 按源的顺序列出去重后的路径。源可以在后续读取中替换或撤回路径，空增量也会更新引用。即使原分块超出保留预算，结算时保存的路径仍可随作业状态在重启后查询。读到输出缺口时，`job_output` 会在输出预算内提示可用文件。文件读取沿用当前会话的路径权限。

Cordis 代码可用 `ctx.jobs.list(owner)` 列出近期任务、用 `get(id, owner)` 读取状态、用 `wait(id, timeoutMs, owner, signal)` 等待结算。工具用 `exec.agent.id` 作为 `owner`，命令使用处理函数中的 `agent.id`。查询范围是当前插件和项目会话。省略 `owner` 时，`list()` 返回空列表。等待超时返回当时的状态，`AbortSignal` 可结束等待。已结算的任务直接返回最终状态。等待时长的 Agena 上限为 600,000 毫秒。

`ctx.jobs.readAt(id, from, owner)` 返回 `{chunks, next, lossy}`，供观察者按自己的位置重读。每块带 UTF-8 绝对字节偏移 `at`、原始文本及可选的 `channel` 和 `gapBefore`。一次读取会返回与所请求区间重叠的完整分块。用返回的 `next` 继续读取。`ctx.jobs.read(id, owner)` 与 `job_output`、内联完成通知共用持久的消费位置。它返回新增分块和作业状态，首次消费结算结果时还返回 `result`。插件直接调用 `read()` 时当场提交消费位置。通过 `job_output` 读取则随成功工具结果提交。

`ctx.jobs.kill(id, owner, reason)` 可在同一模块或同包的其他模块中同步调用。对于 JavaScript 生产者，Agena 在生产者模块中调用 `cancel(reason)`。回调正常返回时，作业进入 `stopping` 并返回 `requested`。回调抛出错误时，错误传给调用方。已结算的作业返回 `already-finished`。最终状态为 `killed` 时，结算详情会附上最近一次成功请求提供的原因。

`ctx.jobs.remove(id, owner)` 可移除已结算的作业。写入完成后会发出 `removed` 事件。重启后，Cordis 注册表和模型控制工具继续隐藏该作业，其会话记录和已保存的输出仍可从会话历史读取。

`dsh-jobs-local` 可配置 `retainBytes`、`settledRetainBytes`、会话并发上限和输出源检查间隔。运行中默认保留 256 KiB，结算结果消费后默认保留 16 KiB。Agena 每项字节配置上限为 1 MiB，每个作业保留至多 4096 块，插件提交的结算文本最多保留 64 KiB。桥接队列溢出时，字节偏移保留缺口，并通过 `gapBefore` 或 `lossy` 报告。尚未消费的结果和保留下来的输出在重启后仍能读取。旧作业的已保存正文可作为一块读取并沿用原消费位置。

`ctx.jobs.events.subscribe(filter, listener)` 可监听通过 `JobSpec` 创建的任务和后台工作流。已初始化的同包模块可接收 `registered`、`progress`、`stopping`、`settled`、`removed` 和文本 `output` 事件。工作流的 `phase()` 对应作业进度和工作流阶段事件。事件携带不可变的状态快照或文本流字节位置，结算包含原因及是否释放了等待者，随后发送最后一个 `output` 事件。该接口按会话投递实时事件，返回解除订阅的函数。队列积压时可通过 `list` 或 `get` 重新读取状态。

当前台调用挂起等待 Promise 或宿主结果时，同一模块里的作业仍可推进定时器、输出拉取和取消处理。后台回调可用会话范围的作业查询权限，前台调用的临时工具、进程和网络权限保留在该调用内。长时间同步执行的 JavaScript 仍按模块串行运行。

同一包中的 Cordis 模块可以使用 `ctx.on()` 或 `ctx.once()` 订阅前台和后台工作流事件，包括 `workflow/start`、`workflow/phase`、`workflow/log`、`workflow/agent-start`、`workflow/agent-end` 和 `workflow/end`。跨模块监听器在模块初始化时注册，随会话保留。首次调用该模块的命令或工具时启动监听，允许 hooks 的包也可以在会话启动时初始化监听器。事件通过会话内的有界队列投递，携带运行标识、元数据及进度或状态数据。后台进度文本可从持久输出继续读取。

发起前台工作流的模块在 `start()` 返回前收到 `workflow/start`，随后在等待 `run.result` 期间接收进度。监听器可调用 `run.cancel(reason)` 取消该运行。`workflow/end` 携带停止原因、启动的子 agent 数及适用的错误信息，最终结果通过 `run.result` 读取。其他模块可以在工作流仍运行时收到进度，繁忙模块按各自的执行顺序处理队列。事件积压超过队列容量时，监听器可能漏收事件。

### Cordis 子代理

通过 `agena plugin install deepseek:<npm 包名>` 或 `agena plugin install <本地包目录>` 安装包含 `inject: ['subagents']` 模块的包。工具把 `exec.agent` 和 `exec.signal` 传给 `ctx.subagents.startActivation()`，项目会话的斜杠命令使用处理函数收到的 `agent` 和 `signal`。子代理由 Agena 本地会话运行，沿用父会话的委派权限和子角色配置。命令可使用已经授予的权限，模型工具也可以走当前调用的审批流程。

`getProvider(name)` 和 `listProviders()` 返回本地 provider 及插件通过 `ctx.subagents.registerProvider(provider)` 注册的自定义 JS provider。`spawn` 在新会话执行任务，`fork` 在子会话里继承父会话已完成的上下文。包可以通过 `providerName` 重命名这两种 provider。`startActivation()` 支持文本及经校验的多媒体块（图片、音频、文档，按目标模型输入能力交付或安全降级）、插件预留的 `childId`（树内唯一校验并体现在目录与寻址中）、任务标签和初始工作目录。相对目录以父会话当前目录为基准，解析结果须在项目工作区内。子代理的相对工具路径从自己的工作目录解析，项目路径权限继续从工作区根目录计算。省略 `name` 时分配第一个可用的 `agent-N`。`agentOptions` 支持 provider、model、reasoningEffort 和每次模型请求的 maxTokens。Spawn 的 `agentPreset` 对应 Agena 的子角色名。`outputSchema` 使用对象根 JSON Schema，结果中的 `structured` 是校验后的 JSON。单次结果最多捕获 64 KiB 文本。

子会话接收首条消息后，返回 `childId`、`name`、`messageId` 和 `result` Promise。同一会话的模块可以在命令结束后继续处理 Promise，并在后续命令中用 `dispose()` 停止它指向的执行尝试。启动信号的取消范围截止于首条消息被接收。`delivery: 'caller'` 通过 Promise 交付结果，`delivery: 'parent'` 另外向父会话投递完成消息。发起模块以及同一插件内发起会话与各存活祖先会话中已启动的模块都可监听这次运行的 `subagent/start` 和 `subagent/end`。可用 `--enable-hooks` 随会话启动监听，也可显式调用该模块的命令。子会话和执行历史持久保存，JavaScript 句柄随会话模块的生命周期释放。

`rootOf()`、`namesOf()`、`listDescendants()` 和 `resolveName()` 读取当前代理树。根会话名为 `lead`，嵌套成员以 `reviewer/checker` 这样的路径寻址。完成的子代理仍在目录中，可以用 `sendMessage()` 送达文本和经校验的多媒体内容块。活动成员在后续模型步骤接收，空闲成员通过 Agena 的持久收件箱启动后续回合。`interrupt()` 允许祖先中断后代当前的执行尝试。读取目录、投递和中断都校验会话树归属，执行操作使用 Agena 的工具权限。

`waitForTreeWork(agent)` 可在树的根会话等待已获准启动、正在执行或正在完成结果交付的后代，也会等待收件箱启动的后代回合。确实等待过这些工作时返回 `true`。后代均空闲时立即返回 `false`，从普通成员调用时也返回 `false`。模型工具等待期间让出工具并发槽。

声明 `dsh-tool-subagent` 和 `dsh-tool-subagent-control` 的包可以提供 `subagent`、`subagent_fork`、`list_agents`、`send_message`、`interrupt_agent` 和 `wait_agent`。`modelDiscovery: true` 启用 `list_subagent_models`，可列出当前会话可用的全部提供商、已配置模型及各模型推理档位。`modelSelection: true` 时，模型可以显式填写任意已发现的提供商、模型与推理档位路由。`wait_agent` 等待收件箱输入或用户排队消息，并让出工具并发槽。

`subagent` 的 `maxActiveSubagents` 为整棵树设定并发上限，默认为八个。新委派与向空闲 Cordis 子代理发送收件箱消息（含重启恢复）都遵循该树配额；树配额已满时，空闲子代理的收件箱消息会保持待投递状态，待其他子代理结算释放名额后按投递顺序自动唤醒，`waitForTreeWork()` 会持续等待至所有延迟唤醒的后代一并完成。`capacityOf()` 提供当前调用开始时的树内占用和上限，并计入同次调用发起且尚未结束的委派。Cordis 子代理可以继续委派。`maxDepth` 校验从根会话起的绝对深度，`maxDepth: 0` 拒绝启动，`resolveMaxDepth()` 读取包配置的深度设置。[DeepSeek Harness Subagent Service](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/subagent/subagent)

## 尚未适配的宿主语义

Codex 的设备验证交互、原生 rollout 限额与调度策略暂缓适配，账号连接器（`.app.json` / `apps`）不在兼容范围内。

Claude 的 evals、用户和托管设置（含对应的 `ConfigChange`）、项目设置中的权限及环境变量、`userConfig` 表单和凭据存储、workflow 云端运行与用量限制暂停，以及团队共享任务列表仍需适配。Worktree 的顶层会话入口、远端刷新和原宿主清理策略仍有差异。Claude 用户及管理指令和工作区外部导入授权仍需适配。Windows PowerShell 的会话环境继承仍有差异。Claude 技能的 shell 展开使用 Agena 的前台执行和输出预算，批次以展开开始时的工作目录为基准。MCP 技能通过会话工具读取，合并到统一技能目录、斜杠命令和版本授权管理仍需适配。

DeepSeek Harness 的后台回调主动执行工具、进程和网络操作还需要相应的会话权限。`agentTeams` 等协同服务，以及涉及子进程、原始网络与宿主机系统的 Node.js API 仍需适配。直接启动原宿主 CLI 或连接其专有 RPC 的插件脚本需要相应的协议桥。

MCP 异步 Tasks 仍缺跨进程恢复和完整进度展示。

[Agent Plugins 规范](https://agent-plugins.org/specification) · [Claude Code 插件格式](https://code.claude.com/docs/en/plugins/manifest-reference) · [Claude Code Hooks](https://code.claude.com/docs/en/hooks) · [Claude Code Mods](https://code.claude.com/docs/en/plugins/mods/create) · [DeepSeek Harness CLI](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/cli/README.md)
