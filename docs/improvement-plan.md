# Agena 完善计划

本文档用于把 Agena 对标 codex / Claude Code / opencode 后发现的功能欠账拆成可逐步实现、逐步提交、逐步推送的清单。

约定：

- 暂时不处理 OS 级沙盒隔离。
- 每完成一个可独立验收的步骤，就单独 commit 并 push。
- 每步至少跑 `cargo check --workspace --locked`；涉及前端时额外跑对应包的类型检查 / 构建。
- 不接受只挂空壳、`todo!()` 或未接入主流程的“假完成”。

## 阶段 0：基线与整理

### 0.1 建立本计划文档

- 目标：把后续工作拆成可执行清单，作为逐步实现与 commit 的依据。
- 主要文件：
  - `docs/improvement-plan.md`
- 验收标准：
  - 文档覆盖 Hooks、AGENTS.md/CLAUDE.md、Auth keyring、Resume、CLI、MCP Server、IDE app-server、apply_patch、流式协议、compaction、TUI、slash command、Studio、Telemetry、工具补齐、provider preset、plugin lifecycle。
- 建议提交信息：
  - `docs: add agena improvement plan`

## 阶段 1：Hooks 与用户可配置自动化

### 1.1 配置层增加用户 hooks schema

- 目标：允许用户在配置文件中声明 hooks，而不是只能通过插件 SDK 使用 hook。
- 主要文件：
  - `crates/agena/src/config/`
  - `config.example.toml`
  - `docs/plugins.md`
- 设计要点：
  - 支持事件：`user_prompt_submit`、`pre_tool_use`、`post_tool_use`、`post_tool_use_failure`、`permission_request`、`stop`、`notification`。
  - 支持 hook 类型：`command`、`prompt`、`http`。
  - 支持字段：`condition`、`timeout_ms`、`async`、`once`、`env`。
  - command hook 先走普通进程执行，不做 OS 沙盒，但要复用现有权限控制与超时。
- 验收标准：
  - 配置可解析、可 validate。
  - 示例配置完整。
  - 非法 hook 配置能给出清晰错误。
- 建议提交信息：
  - `feat(config): add user hook configuration schema`

### 1.2 Runtime 增加 hook dispatcher

- 目标：把 hooks 从配置转换成运行时可调度的事件系统。
- 主要文件：
  - `crates/agena/src/runtime/`
  - `crates/agena/src/event/`
  - `crates/agena/src/error.rs`
- 设计要点：
  - 新增 `HookEvent`、`HookInvocation`、`HookResult`、`HookDecision`。
  - command hook 返回非 0 时按事件类型决定 fail-open / fail-closed。
  - prompt hook 可返回修改后的 prompt。
  - permission hook 可返回 allow / deny / ask。
- 验收标准：
  - dispatcher 有单元测试。
  - timeout、once、async 行为可测试。
- 建议提交信息：
  - `feat(runtime): add hook dispatcher`

### 1.3 接入 session 与 tool 调用链路

- 目标：在真实 turn loop 中触发用户 hooks。
- 主要文件：
  - `crates/agena/src/session/processor.rs`
  - `crates/agena/src/tool/orchestrator.rs`
  - `crates/agena/src/permission/`
- 设计要点：
  - user prompt 进入模型前触发 `user_prompt_submit`。
  - tool 执行前触发 `pre_tool_use`。
  - tool 成功后触发 `post_tool_use`。
  - tool 失败后触发 `post_tool_use_failure`。
  - 需要用户审批时触发 `permission_request`。
  - turn 完成时触发 `stop`。
- 验收标准：
  - 集成测试覆盖 pre/post hook 修改或阻断 tool。
  - HTTP API / TUI 均能观察到 hook 影响。
- 建议提交信息：
  - `feat(session): dispatch hooks during turns and tool calls`

## 阶段 2：项目指令加载

### 2.1 支持 AGENTS.md / CLAUDE.md / AGENA.md 发现

- 目标：自动加载项目级指令文件，补齐 Claude Code / codex 的项目上下文体验。
- 主要文件：
  - `crates/agena/src/memory/`
  - `crates/agena/src/session/prompt_window.rs`
  - `docs/`
- 设计要点：
  - 从当前 workspace 向上查找，到仓库根或文件系统根停止。
  - 支持文件名：`AGENTS.md`、`CLAUDE.md`、`AGENA.md`。
  - 后发现的更局部文件优先级更高。
  - 限制单文件和总大小，避免 prompt 爆炸。
- 验收标准：
  - 单元测试覆盖嵌套目录、多文件合并、大小限制。
  - prompt window 中能看到 project instruction section。
- 建议提交信息：
  - `feat(memory): load project instruction files`

### 2.2 加入用户级全局指令

- 目标：支持 `~/.agena/AGENTS.md` 或 `~/.agena/AGENA.md` 作为用户级长期偏好。
- 主要文件：
  - `crates/agena/src/memory/`
  - `config.example.toml`
- 验收标准：
  - 用户级指令与项目级指令顺序稳定。
  - 可通过 config 关闭。
- 建议提交信息：
  - `feat(memory): support global instruction files`

## 阶段 3：Auth 安全存储

### 3.1 抽象 AuthStore trait

- 目标：把当前文件存储从具体实现抽象成可替换后端。
- 主要文件：
  - `crates/agena/src/provider/auth/`
- 设计要点：
  - 保留 `FileAuthStore`。
  - 提供 `AuthStore` trait。
  - `AuthManager` 泛型或 trait object 化。
- 验收标准：
  - 现有 OAuth / API key 测试不退化。
  - 文件格式保持兼容。
- 建议提交信息：
  - `refactor(auth): abstract credential storage`

### 3.2 新增 keyring 后端

- 目标：优先使用系统 keyring 存储 secret。
- 主要文件：
  - 新 crate：`crates/agena-keyring-store/`
  - `Cargo.toml`
  - `crates/agena/src/provider/auth/`
- 设计要点：
  - Linux Secret Service / macOS Keychain / Windows Credential Manager。
  - keyring 不可用时 fallback 到 file，并给出 warning。
  - auth list 只返回摘要，不返回 secret。
- 验收标准：
  - mock keyring 测试通过。
  - 迁移旧 `auth.json` 不丢 token。
- 建议提交信息：
  - `feat(auth): add keyring credential store`

### 3.3 CLI 登录登出命令

- 目标：提供统一 `login/logout/auth` 用户入口。
- 主要文件：
  - `apps/agena-cli/`
  - `crates/agena/src/cli.rs`
  - `crates/agena/src/provider/auth/`
- 命令草案：
  - `agena auth list`
  - `agena login openai --api-key ...`
  - `agena login openai --browser`
  - `agena login github-copilot --device`
  - `agena logout <provider>`
- 验收标准：
  - CLI 可写入 keyring/file store。
  - HTTP API auth 状态与 CLI 一致。
- 建议提交信息：
  - `feat(cli): add auth login and logout commands`

## 阶段 4：Session 持久化、Resume、Fork

### 4.1 持久化 turn runtime state

- 目标：把 resume 需要的状态完整落库。
- 主要文件：
  - `crates/agena/src/session/store.rs`
  - `crates/agena/src/session/model.rs`
  - `crates/agena/src/db/`
- 设计要点：
  - 保存 active / pending / blocked 状态。
  - 保存 pending tool calls。
  - 保存 latest event seq / provider metadata / prompt cache key。
- 验收标准：
  - 进程重启后可恢复 session state。
  - migration 测试通过。
- 建议提交信息：
  - `feat(session): persist turn runtime state`

### 4.2 CLI resume / continue / fork

- 目标：补齐非交互式会话恢复入口。
- 主要文件：
  - `apps/agena-cli/`
  - `crates/agena/src/session/manager.rs`
- 命令草案：
  - `agena sessions list`
  - `agena resume [SESSION_ID]`
  - `agena continue --last`
  - `agena fork <SESSION_ID> --at <EVENT_SEQ>`
- 验收标准：
  - 可列出最近 session。
  - 可恢复最后一个 session。
  - fork 后原 session 不变。
- 建议提交信息：
  - `feat(cli): add session resume and fork commands`

### 4.3 Resume 端到端测试

- 目标：避免 resume 只是 API 存在但实际不可用。
- 主要文件：
  - `crates/agena/src/session/`
  - `crates/agena-api-server/`
- 验收标准：
  - 启动 turn → 中断 → 重启 manager → continue。
  - blocked permission → 重启 → permission reply。
- 建议提交信息：
  - `test(session): cover resume and blocked turn recovery`

## 阶段 5：CLI 生产化

### 5.1 一次性 exec 命令

- 目标：支持 CI / 脚本 / IDE 调用的非交互模式。
- 主要文件：
  - `apps/agena-cli/`
  - `crates/agena/src/session/manager.rs`
- 命令草案：
  - `agena exec "fix this bug"`
  - `agena exec --workspace . --model ... --json "..."`
- 验收标准：
  - 支持 plain text / json 输出。
  - 非 0 退出码可表达失败。
- 建议提交信息：
  - `feat(cli): add non-interactive exec command`

### 5.2 apply / review / debug 辅助命令

- 目标：补齐常见开发工作流。
- 命令草案：
  - `agena apply <patch-file>`
  - `agena review [--base main]`
  - `agena debug session <id>`
- 验收标准：
  - 命令可调用现有工具和 session 能力。
- 建议提交信息：
  - `feat(cli): add developer workflow commands`

### 5.3 Shell completion

- 目标：补齐 bash/zsh/fish completion。
- 主要文件：
  - `apps/agena-cli/`
  - `ops/`
- 验收标准：
  - `agena completion fish` 可输出 completion。
- 建议提交信息：
  - `feat(cli): add shell completions`

## 阶段 6：MCP Server

### 6.1 新增 agena-mcp-server crate

- 目标：让其他 MCP 客户端可以调用 Agena 工具。
- 主要文件：
  - 新 crate：`crates/agena-mcp-server/`
  - `Cargo.toml`
  - `crates/agena/src/tool/catalog.rs`
- 设计要点：
  - 将 `ToolCatalog` 暴露为 MCP `tools/list` / `tools/call`。
  - 支持 resources/list/read 映射 session / workspace。
  - 支持 prompts/list/get 映射 skills / slash commands。
- 验收标准：
  - stdio transport 可被 MCP inspector 调用。
  - first-party entry schema 正确。
- 建议提交信息：
  - `feat(mcp): add agena mcp server crate`

### 6.2 CLI mcp-server 命令

- 目标：提供 `agena mcp-server` 入口。
- 主要文件：
  - `apps/agena-cli/`
- 验收标准：
  - `agena mcp-server --transport stdio` 可启动。
- 建议提交信息：
  - `feat(cli): add mcp server command`

### 6.3 MCP server 权限与审计

- 目标：外部客户端调用工具时仍走 Agena 权限策略。
- 主要文件：
  - `crates/agena-mcp-server/`
  - `crates/agena/src/permission/`
- 验收标准：
  - dangerous tool call 会被 ask/deny。
  - tool call event 落 session event log。
- 建议提交信息：
  - `feat(mcp): enforce permissions in mcp server`

## 阶段 7：IDE app-server / JSON-RPC

### 7.1 定义 app-server protocol

- 目标：建立面向 IDE 的稳定 JSON-RPC 协议。
- 主要文件：
  - 新 crate：`crates/agena-app-server-protocol/`
- 设计要点：
  - 请求：create session、submit turn、reply permission、list sessions、read messages、cancel turn。
  - 通知：message delta、tool event、permission request、session state changed。
- 验收标准：
  - protocol 类型可 serde roundtrip。
- 建议提交信息：
  - `feat(app-server): define jsonrpc protocol`

### 7.2 实现 app-server transport

- 目标：提供 stdio / websocket / unix socket 传输。
- 主要文件：
  - 新 crate：`crates/agena-app-server/`
  - `apps/agena-studio-server/` 或独立 app-server 入口
- 验收标准：
  - stdio JSON-RPC smoke test。
  - websocket 可订阅 session events。
- 建议提交信息：
  - `feat(app-server): add jsonrpc transports`

### 7.3 VS Code 最小插件原型

- 目标：验证协议能服务 IDE。
- 主要文件：
  - 新 package：`packages/agena-vscode/`
- 验收标准：
  - 能启动 Agena app-server。
  - 能发送 prompt 并显示流式回复。
- 建议提交信息：
  - `feat(vscode): add minimal agena extension`

## 阶段 8：apply_patch 与 diff 可视化

### 8.1 apply_patch 支持 move/rename

- 目标：补齐 patch 操作类型。
- 主要文件：
  - `crates/agena/src/tool/apply_patch.rs`
- 验收标准：
  - add/update/delete/move 全覆盖测试。
- 建议提交信息：
  - `feat(tool): support move operations in apply_patch`

### 8.2 apply_patch 增量事件

- 目标：边解析 patch 边向 UI 发送进度。
- 主要文件：
  - `crates/agena/src/tool/apply_patch.rs`
  - `crates/agena/src/event/`
- 验收标准：
  - 大 patch 可见逐文件进度。
  - 失败时能指出具体 hunk。
- 建议提交信息：
  - `feat(tool): stream apply_patch progress events`

### 8.3 TUI / Studio diff viewer

- 目标：让用户能审查模型修改。
- 主要文件：
  - `apps/agena-tui/`
  - `packages/agena-studio-web/`
- 验收标准：
  - tool result 中显示折叠 diff。
- 建议提交信息：
  - `feat(ui): render tool diffs`

## 阶段 9：Provider 流式协议鲁棒化

### 9.1 统一 stream replay 接口

- 目标：让 `ProviderStreamReplayConfig` 真正生效。
- 主要文件：
  - `crates/agena/src/provider/`
  - `crates/agena/src/session/processor.rs`
- 验收标准：
  - 模拟断流后可 retry。
  - 已输出内容不会重复进入消息历史。
- 建议提交信息：
  - `feat(provider): add stream replay support`

### 9.2 并发 tool call

- 目标：允许模型一次返回多个 tool call 并并发执行安全的工具。
- 主要文件：
  - `crates/agena/src/session/processor.rs`
  - `crates/agena/src/tool/orchestrator.rs`
- 设计要点：
  - 只并发执行标记为 safe/readonly 的工具。
  - 写操作默认串行。
- 验收标准：
  - 两个 read/glob 可并发。
  - read + write 保持安全顺序。
- 建议提交信息：
  - `feat(session): support concurrent safe tool calls`

### 9.3 usage / reasoning / cache metadata

- 目标：完整记录 provider usage。
- 主要文件：
  - `crates/agena/src/provider/types.rs`
  - 各 provider 实现
- 验收标准：
  - cache read/write、reasoning tokens、provider metadata 可落库和显示。
- 建议提交信息：
  - `feat(provider): record reasoning and cache usage metadata`

## 阶段 10：Context compaction

### 10.1 固化 summarization prompt

- 目标：提供稳定、可测试的摘要 prompt。
- 主要文件：
  - `crates/agena/src/session/context_policy.rs`
  - `crates/agena/src/session/context_governor.rs`
- 验收标准：
  - prompt 模板有 snapshot test。
- 建议提交信息：
  - `feat(session): add context summarization prompt`

### 10.2 mid-turn auto compaction

- 目标：长会话中在必要时自动压缩，不等用户手动处理。
- 主要文件：
  - `crates/agena/src/session/processor.rs`
  - `crates/agena/src/session/manager.rs`
- 验收标准：
  - 超阈值时自动生成摘要并继续 turn。
- 建议提交信息：
  - `feat(session): add automatic context compaction`

### 10.3 remote compaction worker

- 目标：用独立任务或轻模型执行摘要，减少主 turn 阻塞。
- 主要文件：
  - `crates/agena-scheduler/`
  - `crates/agena/src/session/`
- 验收标准：
  - compaction 可异步排队。
- 建议提交信息：
  - `feat(session): add remote compaction worker`

## 阶段 11：TUI 生产体验

### 11.1 Permission prompt modal

- 目标：TUI 中完整展示 permission request。
- 主要文件：
  - `apps/agena-tui/`
- 验收标准：
  - allow once / allow session / deny 可用。
- 建议提交信息：
  - `feat(tui): add permission prompt modal`

### 11.2 Tool card 折叠与结果分页

- 目标：工具调用可读，不被大输出刷屏。
- 主要文件：
  - `apps/agena-tui/`
  - `crates/agena/src/tool/truncation.rs`
- 验收标准：
  - 大输出默认折叠，可展开分页。
- 建议提交信息：
  - `feat(tui): add collapsible tool cards`

### 11.3 图片粘贴与附件

- 目标：支持图像输入。
- 主要文件：
  - `apps/agena-tui/`
  - `crates/agena/src/message/`
- 验收标准：
  - 粘贴图片可作为 message part 发送。
- 建议提交信息：
  - `feat(tui): support image attachments`

## 阶段 12：Slash command 与 Skills 集成

状态：已完成（`~/.agena/commands/*.md` 与 `.agena/commands/*.md` 可作为用户 slash command 加载、补全并执行）。

### 12.1 用户 markdown command loader

- 目标：`~/.agena/commands/foo.md` 或 `.agena/commands/foo.md` 自动成为 `/foo`。
- 主要文件：
  - `crates/agena-skills/`
  - `crates/agena/src/tool/skill.rs`
  - `apps/agena-tui/`
- 验收标准：
  - markdown frontmatter 可声明 name、description、allowed_tools、model、aliases。
- 建议提交信息：
  - `feat(skills): load user slash commands from markdown`

### 12.2 Slash command dispatch

- 目标：本地 UI slash commands 与 runtime entry dispatch 明确分层。
- 主要文件：
  - `apps/agena-tui/src/commands.rs`
  - `apps/agena-tui/src/app.rs`
  - `crates/agena/src/plugins/bundled/skills_fs.rs`
- 验收标准：
  - `/help` 能列出本地 UI 命令。
  - `/review` 等 workflow 命令通过 runtime entry registry 分发。
  - markdown-discovered commands 以 dynamic entries 形式暴露。
- 建议提交信息：
  - `refactor(tui): split local slash commands from runtime entry dispatch`

## 阶段 13：Studio Web

状态：已完成（Web 可回复 blocked session 权限请求、管理 provider/auth 与权限规则，并通过 Workspace 页面浏览文件树；Chat 保留 tool patch diff 展示）。

### 13.1 Permission UI

- 目标：Web/Studio 中可处理权限请求。
- 主要文件：
  - `packages/agena-studio-web/`
  - `crates/agena-api-server/`
- 验收标准：
  - blocked session 可在 Web 中恢复。
- 建议提交信息：
  - `feat(studio): add permission request UI`

### 13.2 Settings UI

- 目标：管理 providers、auth、hooks、permissions、MCP。
- 主要文件：
  - `packages/agena-studio-web/`
- 验收标准：
  - 至少能查看和编辑 provider/auth 状态。
- 建议提交信息：
  - `feat(studio): add settings pages`

### 13.3 File tree 与 diff viewer

- 目标：Studio 中浏览 workspace 文件与模型修改。
- 主要文件：
  - `packages/agena-studio-web/`
  - `crates/agena-api-server/`
- 验收标准：
  - 可查看文件树。
  - 可查看 tool 修改 diff。
- 建议提交信息：
  - `feat(studio): add file tree and diff viewer`

## 阶段 14：Telemetry / OTel

状态：已完成（新增默认关闭的 OTel/OTLP tracing 配置与 CLI/TUI 初始化，关键 session/provider/tool/hook 链路已打 span，并提供 CLI/TUI 脱敏 diagnostics 入口）。

### 14.1 新增 telemetry 基础设施

- 目标：提供 opt-in 观测能力。
- 主要文件：
  - 新 crate：`crates/agena-otel/`
  - `crates/agena/src/runtime/`
- 设计要点：
  - 默认关闭。
  - 支持 env/config 开启。
  - span 覆盖 session turn、provider request、tool call、hook call。
- 验收标准：
  - 本地 OTLP exporter smoke test。
- 建议提交信息：
  - `feat(telemetry): add opt-in otel tracing`

### 14.2 Feedback command

- 目标：收集用户反馈与 issue 上报入口。
- 主要文件：
  - `apps/agena-cli/`
  - `apps/agena-tui/`
- 验收标准：
  - 可生成脱敏诊断包。
- 建议提交信息：
  - `feat(cli): add feedback diagnostics command`

## 阶段 15：工具补齐

状态：已完成（新增 NotebookEdit 的 replace / insert / delete 单元格编辑；复查并保持 LSP diagnostics / definition / references 工具可用；新增 Windows PowerShell 专用工具并提供非 Windows 显式不可用反馈）。

### 15.1 NotebookEdit

- 目标：支持 `.ipynb` 单元格编辑。
- 主要文件：
  - `crates/agena/src/tool/notebook_edit.rs`
  - `crates/agena/src/tool/catalog.rs`
- 验收标准：
  - replace / insert / delete cell 测试通过。
- 建议提交信息：
  - `feat(tool): add notebook edit tool`

### 15.2 LSP 工具

- 目标：提供 diagnostics / definition / references。
- 主要文件：
  - 新 crate 或模块：`crates/agena/src/tool/lsp.rs`
- 验收标准：
  - 可对 Rust 项目返回 diagnostics。
- 建议提交信息：
  - `feat(tool): add lsp inspection tools`

### 15.3 PowerShell 工具适配

- 目标：Windows 上提供 PowerShell 专用工具。
- 主要文件：
  - `crates/agena/src/tool/powershell.rs`
- 验收标准：
  - Windows cargo check / 基础命令测试。
- 建议提交信息：
  - `feat(tool): add powershell tool`

## 阶段 16：Provider preset 与 native provider

状态：已完成（新增常用 provider 内置 preset 默认值；`ollama` preset 改为 native provider；新增 Ollama `/api/tags` 模型列表、`/api/chat` 非流式与 JSONL 流式支持；OpenAI Realtime / WebSocket 能力已在现有 OpenAI/OpenAI-compatible provider 中保留验证）。

### 16.1 常用 openai-compatible preset

- 目标：减少用户配置成本。
- 主要文件：
  - `config.example.toml`
  - `crates/agena/src/provider/model_metadata.rs`
- Provider：
  - Ollama
  - LM Studio
  - OpenRouter
  - DeepSeek
  - xAI
  - Groq
  - Mistral
- 验收标准：
  - preset 解析、模型列表、默认 model 可用。
- 建议提交信息：
  - `feat(provider): add common compatible provider presets`

### 16.2 Ollama native provider

- 目标：支持 Ollama 原生模型列表、健康检查和本地默认行为。
- 主要文件：
  - `crates/agena/src/provider/ollama.rs`
- 验收标准：
  - `/api/tags` 模型列表可用。
  - generate/chat 流式可用。
- 建议提交信息：
  - `feat(provider): add native ollama provider`

### 16.3 OpenAI Realtime / WebSocket

- 目标：跑通实时流式能力。
- 主要文件：
  - `crates/agena/src/provider/openai.rs`
  - `crates/agena/src/provider/sse.rs`
- 验收标准：
  - WebSocket stream smoke test。
- 建议提交信息：
  - `feat(provider): add openai realtime streaming`

## 阶段 17：Status line 与通知

状态：已完成（新增 TUI 本地 `[status_line]` 命令配置与周期刷新；新增 `notification` shell hook，可在回合完成与权限请求时触发桌面通知命令）。

### 17.1 TUI status line 配置

- 目标：允许用户自定义底栏显示。
- 主要文件：
  - `apps/agena-tui/`
  - `crates/agena/src/config/`
- 验收标准：
  - 可配置 command 生成 status line。
- 建议提交信息：
  - `feat(tui): add configurable status line`

### 17.2 Desktop notification hook

- 目标：长任务完成或需要用户操作时通知。
- 主要文件：
  - `crates/agena/src/runtime/`
  - `apps/agena-tui/`
- 验收标准：
  - `notification` hook 可触发桌面通知。
- 建议提交信息：
  - `feat(runtime): add notification hook support`

## 阶段 18：Plugin lifecycle 扩展

状态：已完成（新增 `pre_turn` / `post_turn` 生命周期 hook，兼容 `permission_request`、`pre_compaction`、`post_compaction` manifest 别名；运行时 turn loop 接入 pre/post turn 广播；`provider.list` patch 已接入 provider registry，example echo plugin 覆盖新 lifecycle hook 并注册 `echo-mock` provider）。

### 18.1 扩展 plugin hook surface

- 目标：让插件可参与 turn、permission、compaction 生命周期。
- 主要文件：
  - `crates/agena-plugin-sdk/`
  - `crates/agena-plugin-host/`
- 新增 hook：
  - `pre_turn`
  - `post_turn`
  - `permission_request`
  - `pre_compaction`
  - `post_compaction`
- 验收标准：
  - example plugin 覆盖新 hook。
- 建议提交信息：
  - `feat(plugin): extend lifecycle hooks`

### 18.2 Plugin 注册 provider

- 目标：插件可以提供新 provider。
- 主要文件：
  - `crates/agena-plugin-sdk/`
  - `crates/agena-plugin-host/`
  - `crates/agena/src/provider/registry.rs`
- 验收标准：
  - example plugin 注册 mock provider。
- 建议提交信息：
  - `feat(plugin): allow plugins to register providers`

## 推荐执行顺序

1. `docs: add agena improvement plan`
2. `feat(config): add user hook configuration schema`
3. `feat(runtime): add hook dispatcher`
4. `feat(session): dispatch hooks during turns and tool calls`
5. `feat(memory): load project instruction files`
6. `feat(memory): support global instruction files`
7. `refactor(auth): abstract credential storage`
8. `feat(auth): add keyring credential store`
9. `feat(cli): add auth login and logout commands`
10. `feat(session): persist turn runtime state`
11. `feat(cli): add session resume and fork commands`
12. `test(session): cover resume and blocked turn recovery`
13. `feat(cli): add non-interactive exec command`
14. `feat(cli): add developer workflow commands`
15. `feat(cli): add shell completions`
16. `feat(mcp): add agena mcp server crate`
17. `feat(cli): add mcp server command`
18. `feat(mcp): enforce permissions in mcp server`
19. `feat(app-server): define jsonrpc protocol`
20. `feat(app-server): add jsonrpc transports`
21. `feat(vscode): add minimal agena extension`
22. `feat(tool): support move operations in apply_patch`
23. `feat(tool): stream apply_patch progress events`
24. `feat(ui): render tool diffs`
25. `feat(provider): add stream replay support`
26. `feat(session): support concurrent safe tool calls`
27. `feat(provider): record reasoning and cache usage metadata`
28. `feat(session): add context summarization prompt`
29. `feat(session): add automatic context compaction`
30. `feat(session): add remote compaction worker`
31. `feat(tui): add permission prompt modal`
32. `feat(tui): add collapsible tool cards`
33. `feat(tui): support image attachments`
34. `feat(skills): load user slash commands from markdown`
35. `refactor(tui): split local slash commands from runtime entry dispatch`
36. `feat(studio): add permission request UI`
37. `feat(studio): add settings pages`
38. `feat(studio): add file tree and diff viewer`
39. `feat(telemetry): add opt-in otel tracing`
40. `feat(cli): add feedback diagnostics command`
41. `feat(tool): add notebook edit tool`
42. `feat(tool): add lsp inspection tools`
43. `feat(tool): add powershell tool`
44. `feat(provider): add common compatible provider presets`
45. `feat(provider): add native ollama provider`
46. `feat(provider): add openai realtime streaming`
47. `feat(tui): add configurable status line`
48. `feat(runtime): add notification hook support`
49. `feat(plugin): extend lifecycle hooks`
50. `feat(plugin): allow plugins to register providers`
