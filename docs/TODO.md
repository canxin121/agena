# Agena 改进 TODO

基于对 codex / claude-code / opencode 三家的源码对比，以下为 Agena 当前缺口与改进计划。

## 一、Agena 已有能力

**架构层**
- Workspace：5 apps（cli、tui、http-api-server、studio-server、studio-desktop）+ 11 crates
- 核心 `agena` crate：agent / runtime / session / tool / provider / permission / config / memory / message / event / storage / db
- 三层 API：HTTP REST（v1，35+ 端点）、WebSocket/SSE/Unix-IPC（v2）、Rust Client SDK
- 插件体系：`agena-plugin-host` + `agena-plugin-sdk`（动态 / WASM 插件，ABI 稳定）
- `procwarden` 沙盒已**移除**（#15 第一阶段）— 改为权限规则 store 兜底；`tool::shell` 提供超时 + loader-env 清洗

**Provider（13 家，领先同类）**
OpenAI / OpenAI-compatible / Codex(Responses) / Anthropic / Gemini / Bedrock / Vertex / Cloudflare AI Gateway / GitHub Copilot（device code） / GitLab OAuth / opencode 中继

**Tool（22 个 builtin + MCP + Skill + Plugin）**
bash / read / glob / grep / view_file / apply_patch / web_fetch / web_search / task / monitor / monitor_tool / todo_write / ask_user / cron / plan / worktree / skill / subtask / tool_search / orchestrator …

**Session / Context**
- ContextGovernor + ContextPolicy + PromptWindow + history/transcript + history/view
- 会话状态机、乐观并发（version + If-Match）、fork / rewind / continue

**其他**
- Skills（YAML + frontmatter，allowed_tools 约束）
- Scheduler（cron 表达式）
- MCP 客户端（stdio / http）
- 权限规则 store（文件级 glob / 工具级，allow / ask / deny）
- 0 个 TODO/FIXME，代码质量高

---

## 二、能力差距矩阵

| 能力 | Codex | Claude Code | OpenCode | **Agena** |
|---|---|---|---|---|
| Agent loop | ✅ | ✅ | ✅ | ✅ |
| 自动 compact | ✅ inline+remote | ✅ 时间/token | ✅ session compaction | ⚠️ 有痕迹，策略不完整 |
| 沙盒 / 命令隔离 | ✅ landlock+bwrap+seatbelt | ⚠️ 规则匹配（无系统沙盒） | ⚠️ 规则匹配（无系统沙盒） | ⚠️ 已弃用 procwarden，#15 第一阶段完成（`tool::shell`），第二阶段权限层增强待办 |
| Hooks 事件 | ✅ | ✅ 8 种事件 | ⚠️ plugin 钩子 | ✅ 7 事件 + shell + HTTP hook 形态（#1 第二阶段进行中） |
| Plan Mode | ✅ | ✅ Enter/ExitPlanMode | ✅ plan agent | ⚠️ 有 plan.rs，未与 session 接通 |
| Subagent | ✅ SubAgentSource | ✅ Task tool | ✅ @subagent_type | ⚠️ 调度协议不完整 |
| Slash commands | ✅ 27+ | ✅ 50+ | ✅ commands/*.md | ❌ 缺中心 dispatcher |
| 自定义命令 | ✅ | ✅ | ✅ | ⚠️ 后端加载器已完成（#2），TUI / CLI dispatcher 待接入 |
| Skill 系统 | ✅ | ✅ 17+ | ✅ | ✅ |
| Memory（CLAUDE.md） | ✅ 两阶段 | ✅ MEMORY.md 索引 | ✅ AGENTS.md | ⚠️ 自动写回未做 |
| MCP（OAuth+动态注册） | ✅ | ✅ | ✅ | ⚠️ OAuth 待补 |
| LSP 集成 | ❌ | ❌ | ✅ **招牌** | ❌ **缺失** |
| Worktree | ❌ | ✅ | ✅ | ⚠️ 有但未对外 |
| Resume / share | ✅ | ✅ | ✅ snapshot+share | ⚠️ rollout 有，UX 未做 |
| TUI | ✅ ratatui | ✅ Ink+Vim | ✅ Solid+OpenTUI | ⚠️ Phase 2 推进中 |
| 多 Provider | ✅ | 单家 | ✅ | ✅ **领先** |
| Cost/Token UX | ✅ /cost | ✅ /cost+budget | ✅ | ⚠️ 后端有 UI 未透出 |
| Doom-loop 检测 | ⚠️ | ⚠️ | ✅ ≥3 重复阻止 | ❌ **缺失** |
| 后台任务+通知 | ✅ Monitor | ✅ Monitor+Push | ⚠️ | ⚠️ 通知通道未接 |

---

## 三、改进任务（按优先级）

### P0：核心 UX 缺口（4–6 周）

- [x] **#1 Hooks 系统（第二阶段进行中）** — 参考 Claude Code `settings.json`
  - [x] 配置驱动的 shell hook：`agena.toml` 中 `[[hooks]]` 段（`event` + `command` + `matcher.tool` glob + `timeout_ms`）
  - [x] `crates/agena/src/hooks/` 模块：`HookEvent { UserPromptSubmit / ToolBefore / ToolAfter / ToolFailure / AgentStop / SessionStart / SessionEnd }` + `ShellHookPlugin`（实现 SDK `Plugin` trait，注入丰富 `AGENA_*` env vars）
  - [x] 输出 patch 通过 stdout JSON 解析（如 `{"prompt": "..."}` / `{"additional_context": "..."}` / `{"continue_with_message": "..."}` / `{"title_override": "..."}`）；空 / 非 JSON / 非零退出仅记 warn 不阻断
  - [x] 注册到 `build_plugin_host` 与 builtins / memory 并列；7 单测 + 1 端到端 config 测试全过
  - [x] **HTTP hook 形态**：`[[hooks]] url = "https://..."` 时 POST `{event, env, payload}` JSON 到端点，parse response body 为 patch；blocking client 跑在独立线程，复用同一 timeout / patch 解析路径；3 个新单测（含端到端的 in-test TCP 服务器）
  - 待办：
    - 内置 Prompt hook 形态（让 hook 调用一个内部小 LLM 子任务）
    - `~/.agena/settings.json` 命名空间镜像（目前 `agena.toml` 已就位）
    - PreToolUse 审批 UI（`PermissionRuntime` 后端已就绪，hook 已能 deny → 待 TUI/CLI 接入弹窗）

- [ ] **#2 Slash Commands 中央 dispatcher + 自定义命令（自定义命令后端已完成 ✅）**
  - [x] **自定义命令加载器**（`crates/agena/src/commands/`）：扫 `.agena/commands/*.md`（向上 walk）+ `~/.agena/commands/*.md`，frontmatter 支持 `description / argument-hint / allowed_tools / model / aliases`，body 支持 `$1..$N` + `$ARGUMENTS` 替换；项目覆盖用户、aliases 解析为同一 command；8 个单测覆盖（含 walk-up 发现、冲突优先级、aliases 去重）
  - 待办：内置 slash 中央 dispatcher（`/help /clear /compact /plan /resume /cost /memory /init /review /worktree /tasks /config /share /doctor`），TUI / CLI 共用

- [ ] **#3 Plan Mode 接通会话流**
  - `plan.rs` 已有 `PlanRegistry`，扩展为模式：进入 plan 后强制 `tool_policy = read_only`，禁止 Edit/Write/Bash 写
  - 新增 `EnterPlanMode` / `ExitPlanMode` 工具，写计划到 `plan.md` 草稿
  - session 上记录 `mode: plan|build`

- [ ] **#4 自动 Compact 策略落地**
  - `context_governor.rs` 基础上加触发器：剩余 token < 阈值（默认 15%）/ 用户 `/compact`
  - 双策略：本地 LLM 总结（template `templates/compact.md`） + provider 原生（Anthropic prompt cache、OpenAI Responses）
  - 写入 `CompactedItem` 到 transcript，UI 显示折叠条

- [ ] **#5 Memory 自动管理**（CLAUDE.md 等价）
  - 入口：`AGENA.md` / `CLAUDE.md` / `AGENTS.md` 三文件链式查找
  - `MEMORY.md` 索引 + 单文件 frontmatter（type=user/feedback/project/reference）
  - `memory/writer.rs`：自动记忆 + 去重 + staleness 标记
  - Slash：`/memory edit|list|forget`

### P1：Agent 协作（4 周）

- [ ] **#6 Subagent 调度协议完善**
  - `subtask.rs` 加：parent → child 上下文裁剪、子代理结果回流摘要、超时/中断
  - `Agent` 加 `subagent_type` 注册表（`.agena/agents/*.md` 定义）
  - 内置 subagent types：`explorer / planner / reviewer / refactorer`，配模板提示词

- [ ] **#7 后台任务 + 推送通知通道**
  - `monitor_tool.rs` 加桌面通知后端（notify-rust）+ 手机推送（webhook 可配）
  - 通知速率限制 + "silence is not success" 守则写入文档

- [x] **#8 Doom-loop / 重复检测** ✅
  - `session::doom_loop` 模块 + `DoomLoopPolicy { repeat_threshold: 3 }` 默认启用
  - 在 `manager::run_until_stable` 每轮迭代前扫描历史，若同 `(tool, args_json)` 连续 ≥ N 次则中断 turn 并发布 `RunFailed` 事件
  - 5 个单测覆盖：连续命中、输入差异不命中、关闭策略、跨消息聚合、不同工具重置计数

- [ ] **#15 移除 procwarden 沙盒，改用 opencode / Claude Code 风格的权限审批系统** 🔥（**第一阶段完成**）
  - 背景：procwarden（landlock+bwrap）在跨发行版 / 容器 / Mac 上几乎不可用，价值不抵成本
  - **第一阶段：删除 procwarden（已完成 ✅）**
    - [x] 删除 `procwarden` crate dep（`crates/agena/Cargo.toml`）
    - [x] 新增 `crates/agena/src/tool/shell.rs`：`ExecutionPolicy { ReadOnly / WorkspaceWrite / DangerFullAccess }` + `ShellRequest` / `ShellOutput` / `ShellError` + `execute()`，含 watchdog 超时与 loader-env 清洗（`LD_PRELOAD` / `DYLD_*` / `BASH_FUNC_*` 等），6 个单测覆盖
    - [x] `tool/mod.rs` / `tool/bash.rs`：`SandboxPolicy` → `ExecutionPolicy`、`execute_sandboxed_command` → `execute_shell_command`、`ToolError::Sandbox` → `ToolError::Shell`，删除无状态 `sandbox_manager` 字段
    - [x] `agena-plugin-sdk/src/host_api.rs`：删除 `execute_sandboxed_command` 钩子与 `SandboxCommandRequest` / `SandboxCommandResponse` / `SandboxMode` 类型（零外部调用）
    - [x] `cargo test -p agena --lib -- --test-threads=1` 367/367 通过（含 `bash_builtin_blocks_obvious_write_commands_in_read_only_policy` 关键回归测试）
  - **第二阶段：权限层增强（进行中）** — 参考 opencode `permission.{edit,bash,read,task,...}` + Claude Code `settings.json [permissions]`
    - [x] **bash 命令模式 allow/ask/deny**：`ToolPermissionPolicy::with_bash_pattern_rule` + `[[permission.bash]]` 配置段（`pattern` + `mode`），首匹配胜出，仅作用于 `BuiltinToolInput::Bash`，4 个单测 + 1 个端到端 config 测试
    - [ ] 三档权限模式：`auto` / `ask` / `plan`（继承 plan 模式的只读语义）
    - [ ] PreToolUse 钩子前置：UI/CLI 弹审批，记忆"始终允许"决策（`PermissionRuntime` 已有 AllowAlways/DenyAlways 持久化路径，待 UI 接入）
    - [ ] 把 `[permissions]` 同步进 `~/.agena/settings.json` 命名空间（目前只有 TOML config）
  - 文档：新 `docs/PERMISSIONS.md` 取代原计划的 `docs/SANDBOX.md`

### P2：横向能力（4–6 周）

- [ ] **#9 MCP OAuth + 动态注册**
  - `agena-mcp-client` 加 OAuth 流（device code / authorization code），keychain 存储
  - 工具变化时通过 `notifications/tools/list_changed` 动态刷新

- [ ] **#10 LSP 集成**（OpenCode 招牌，差异化）
  - 新 crate `agena-lsp`：tower-lsp 客户端 + per-project server pool
  - 工具：`lsp_definition / lsp_references / lsp_diagnostics / lsp_rename`
  - apply_patch 后自动取诊断回喂模型

- [ ] **#11 Worktree 生命周期对外**
  - 现有 `worktree.rs` 注册表，TUI / CLI 暴露 `EnterWorktree / ExitWorktree`
  - `.agena/worktrees/` 自动清理钩子 + tmux 集成可选

- [ ] **#12 Resume / Share / Snapshot UX**
  - `agena-rollout` 加 replay 命令：`agena resume <session-id>`
  - Share：序列化为只读链接（http-api-server 路由 `/share/:id`）

### P3：打磨与差异化（持续）

- [ ] **#13 TUI Phase 2/3**：ratatui slash-command 弹层、cost 面板、并行 subagent 树视图、Vim 模式
- [x] **#14 Provider 补充（部分）** ✅ ：内置 `ollama` / `lmstudio` 本地 preset（OpenAI 兼容、`OLLAMA_HOST` / `LMSTUDIO_HOST` 自动识别）。仍待补：xAI Grok、Mistral 专用 preset。
- [ ] **#16 Cost / Token 透出**：`/cost` 命令、TUI 状态栏 token 计数、按 provider 价格估算

---

## 四、建议落地顺序

| Sprint | 周期 | 内容 |
|---|---|---|
| Sprint 1 | 2 周 | #1 Hooks + #2 Slash Commands dispatcher（其他工作的载体） |
| Sprint 2 | 2 周 | #15 移除 procwarden + 权限审批系统升级（P1，先做以解锁后续工具默认放行策略） |
| Sprint 3 | 2 周 | #3 Plan Mode 接通 + #4 自动 Compact 策略 |
| Sprint 4 | 2 周 | #5 Memory 自动管理 + #8 Doom-loop ✅（doom-loop 已完成） |
| Sprint 5 | 3 周 | #6 Subagent 调度 + subagent types 模板 |
| Sprint 6+ | — | #10 LSP / #9 MCP OAuth / #11 Worktree UX / #12 Share / #13 TUI Phase 2 |
