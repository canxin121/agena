# Agena in-tree plugin-entry inventory（简体中文）

- **Document version:** plugin-entry-only runtime
- **Language:** 简体中文
- **Mirror:** `docs/builtins.md`

本文档记录的是 **随 `agena` 二进制一起交付的 in-tree 扩展面**。

它只关注产品自身在编译期就确定存在的内容：

- 由运行时静态注册的 first-party plugins
- 编译期固定存在、对模型可见的 plugin entries
- 编译进二进制的 bundled workflow markdown
- in-tree TUI slash commands
- in-tree provider 实现

它 **不** 试图枚举那些名字依赖用户文件或配置、只在运行时生成的能力，例如：

- 通过配置在运行时加载的 plugins（`cdylib`、`stdio`、`http`、`wasm`）
- 从 `.agena/skills`、`~/.agena/skills`、`~/.claude/skills` 生成的 entries
- 从 `.agena/commands` 或 `~/.agena/commands` 生成的 entries
- 由已配置 MCP server 生成的 tools/resources/prompts entries
- plugin 在运行时追加的 providers
- 由用户配置驱动的 shell hooks

## 权威来源

本文档主要依据以下源码位置整理：

- `crates/agena/src/config/registry.rs` —— 把 first-party static plugins 注册进运行时
- `crates/agena/src/entry/catalog.rs` —— 从 first-party plugin manifest 投影编译期固定的 model-visible entries
- `crates/agena/src/plugins/bundled/` —— first-party plugin 的具体实现
- `crates/agena/src/plugins/bundled/skills_fs.rs` —— 扫描 skills/commands markdown 并注册 dynamic entries 的 discovery plugin
- `crates/agena-skills/src/bundled/` —— 通过 `include_str!` 编译进去的 bundled workflow markdown
- `apps/agena-tui/src/commands.rs` —— in-tree slash command 列表
- `crates/agena/src/provider/` —— in-tree provider 实现

---

## 1. In-tree first-party plugins

这些 plugin 都是编译进二进制的，并由 `ResolvedConfig::build_plugin_host_with_previous_and_mcp()` 在 `crates/agena/src/config/registry.rs` 中静态注册。

### 1.1 始终注册的 first-party plugins

| Plugin ID | 作用 | 对模型可见的表面 |
|---|---|---|
| `agena.lsp` | LSP 可观测性与导航 plugin | 编译期固定 entries |
| `agena.cron` | 调度 plugin | 编译期固定 entries |
| `agena.fs` | 文件系统 plugin | 编译期固定 entries |
| `agena.shell` | shell 与 monitor plugin | 编译期固定 entries |
| `agena.web` | web-fetch / web-search plugin | 编译期固定 entries |
| `agena.workflow` | workflow / orchestration plugin | 编译期固定 entries |
| `agena.skills_fs` | markdown skills 与 commands 的 discovery plugin | 运行时 dynamic entries |
| `agena-memory` | 内置 memory 子系统 plugin | 不暴露 model-visible entries |
| `agena-shell-hooks` | 内置 shell hook bridge | 不暴露 model-visible entries |

### 1.2 条件注册的 first-party plugins

| Plugin ID | 注册条件 | 说明 |
|---|---|---|
| `agena.mcp` | 存在 MCP manager 时注册 | 对模型暴露的 entry 名称来自 live MCP server snapshot，因此本文不枚举这些运行时动态名字 |

### 1.3 哪些并行 runtime substrate 已经不再存在

运行时已经不再把下面这些当成与 plugin entries 并列的一等扩展层：

- 旧的 built-in executor substrate
- 旧的 skills substrate
- 旧的 `skill_run` 兼容桥
- 面向模型工作流的独立 skill registry
- 面向 runtime markdown commands 的独立 command registry

现在，对模型可见的能力统一来自 plugin entries：

- first-party plugin manifest 声明的编译期 entries
- 通过 plugin host entry registry 注册的 dynamic entries

---

## 2. 编译期固定的 model-visible entries

编译期固定的 model-visible catalog 由 `crates/agena/src/entry/catalog.rs` 组装，通过合并 first-party plugins 的 entry 声明得到。

### 2.1 文件系统 entries（`agena.fs`）

定义于 `crates/agena/src/plugins/bundled/fs.rs`。

| Entry | 行为类型 | 作用 |
|---|---|---|
| `read` | Read-only | 读取文本文件 |
| `view_file` | Read-only | 把本地文件重新作为多模态输入附加回对话 |
| `glob` | Read-only | 按 glob 模式搜索文件 |
| `grep` | Read-only | 按正则搜索文件内容 |
| `apply_patch` | Write-sandboxed | 应用结构化文件补丁 |
| `notebook_edit` | Write-sandboxed | 编辑 Jupyter notebook 的 cell |

### 2.2 Shell entries（`agena.shell`）

定义于 `crates/agena/src/plugins/bundled/shell.rs`。

| Entry | 行为类型 | 作用 |
|---|---|---|
| `bash` | Write-sandboxed | 在工作区中执行 shell 命令 |
| `powershell` | Write-sandboxed | 执行 PowerShell 命令 |
| `monitor` | Write-sandboxed | 启动、列出、读取、停止长时间运行的受监控进程 |

### 2.3 Web entries（`agena.web`）

定义于 `crates/agena/src/plugins/bundled/web.rs`。

| Entry | 行为类型 | 作用 |
|---|---|---|
| `web_fetch` | Read-only | 抓取 URL 内容并转成 markdown |
| `web_search` | Read-only | 使用当前配置的后端执行网页搜索 |

### 2.4 Workflow / orchestration entries（`agena.workflow`）

定义于 `crates/agena/src/plugins/bundled/workflow.rs`。

| Entry | 行为类型 | 作用 |
|---|---|---|
| `init` | Read-only | 生成 bundled init workflow prompt |
| `review` | Read-only | 生成 bundled review workflow prompt |
| `security-review` | Read-only | 生成 bundled security-review workflow prompt |
| `task` | Task | 创建或恢复一个委派子任务会话 |
| `tool_search` | Read-only | 搜索工具目录，并可按需加载 deferred tools |
| `todo_write` | Read-only | 替换当前会话的 todo 列表 |
| `ask_user` | Read-only | 向用户发起简短提问并等待回答 |
| `enter_plan_mode` | Read-only | 进入 plan mode |
| `exit_plan_mode` | Read-only | 退出 plan mode |
| `enter_worktree` | Write-sandboxed | 创建或附着到一个 worktree |
| `exit_worktree` | Write-sandboxed | 离开当前 worktree |

### 2.5 LSP entries（`agena.lsp`）

定义于 `crates/agena/src/plugins/bundled/lsp.rs`。

| Entry | 行为类型 | 作用 |
|---|---|---|
| `lsp_servers` | Read-only | 列出已配置的 LSP servers |
| `lsp_definition` | Read-only | 跳转到符号定义 |
| `lsp_references` | Read-only | 查找符号引用 |
| `lsp_hover` | Read-only | 读取 hover / 类型信息 |
| `lsp_diagnostics` | Read-only | 读取文件诊断信息 |

### 2.6 调度 entries（`agena.cron`）

定义于 `crates/agena/src/plugins/bundled/cron.rs`。

| Entry | 行为类型 | 作用 |
|---|---|---|
| `cron_create` | Read-only | 创建一个周期性调度任务 |
| `cron_list` | Read-only | 列出当前调度任务 |
| `cron_delete` | Read-only | 删除一个调度任务 |
| `schedule_wakeup` | Read-only | 安排一次性的 wake-up prompt |

### 2.7 完整的编译期固定 entry 清单

为了方便检索，当前编译期固定存在的 model-visible entry 名称如下：

- `read`
- `view_file`
- `glob`
- `grep`
- `apply_patch`
- `notebook_edit`
- `bash`
- `powershell`
- `monitor`
- `web_fetch`
- `web_search`
- `init`
- `review`
- `security-review`
- `task`
- `tool_search`
- `todo_write`
- `ask_user`
- `enter_plan_mode`
- `exit_plan_mode`
- `enter_worktree`
- `exit_worktree`
- `lsp_servers`
- `lsp_definition`
- `lsp_references`
- `lsp_hover`
- `lsp_diagnostics`
- `cron_create`
- `cron_list`
- `cron_delete`
- `schedule_wakeup`

也就是说，当前共有 **31 个编译期固定、对模型可见的 entries**。

---

## 3. 运行时动态生成的 entry 表面

### 3.1 `agena.skills_fs`

定义于 `crates/agena/src/plugins/bundled/skills_fs.rs`。

`agena.skills_fs` 是一个 first-party discovery plugin。它会扫描这些根目录：

- `.agena/skills`
- `~/.agena/skills`
- `~/.claude/skills`
- `.agena/commands`
- `~/.agena/commands`

它还会读取 `crates/agena-skills/src/bundled/` 中的 bundled markdown 内容。

所有发现到的内容都会通过 `host/entry.register` 注册成 **dynamic plugin entries**。这意味着 markdown skills 和 markdown commands 已经不再是独立 runtime substrate，而是共享 entry registry 里的普通 plugin entries。

这些名字依赖运行时文件内容，因此不属于上面那份固定编译期清单。

### 3.2 `agena.mcp`

定义于 `crates/agena/src/plugins/bundled/mcp.rs`。

`agena.mcp` 是已配置 MCP servers 的 first-party adapter plugin。它会把 server 状态投影成对模型可见的 entries，例如：

- `mcp:<server>:tool:<tool>`
- `mcp:<server>:resources:list`
- `mcp:<server>:resources:read`
- `mcp:<server>:prompts:list`
- `mcp:<server>:prompts:get`

这些名字来自 live MCP snapshot，因此也属于运行时动态生成的 entry 名称。

---

## 4. 编译进二进制的 bundled workflow markdown

bundled workflow markdown 位于 `crates/agena-skills/src/bundled/`，并通过 `crates/agena-skills/src/bundled/mod.rs` 中的 `include_str!` 编译进二进制。

这些内容是 **供 first-party plugins 消费的源材料**，不是一个单独的 runtime registry。

### 4.1 `init`

源码：`crates/agena-skills/src/bundled/init.md`

- **Name:** `init`
- **Alias:** `bootstrap`
- **Description:** initialise an `AGENTS.md` or `CLAUDE.md` describing the codebase
- **Purpose:** 为仓库初始化项目说明 / memory 文件

### 4.2 `review`

源码：`crates/agena-skills/src/bundled/review.md`

- **Name:** `review`
- **Description:** review the current branch as a senior code reviewer
- **Allowed tools:** `read`, `glob`, `grep`, `view_file`
- **Purpose:** 对当前分支执行结构化代码评审

### 4.3 `security-review`

源码：`crates/agena-skills/src/bundled/security_review.md`

- **Name:** `security-review`
- **Description:** audit the current branch for security regressions
- **Allowed tools:** `read`, `glob`, `grep`
- **Purpose:** 对当前分支执行安全导向评审

这些工作流在运行时的 canonical compile-time entries 由 `agena.workflow` 暴露为 `init`、`review`、`security-review`。

---

## 5. In-tree TUI slash commands

in-tree slash commands 定义在 `apps/agena-tui/src/commands.rs` 中。

这些命令是 TUI 自身在编译期就实现好的能力。纯 UI / 本地命令继续留在本地，而 `/review` 这类 workflow 命令则通过 runtime entry registry 分发。

当前命令集合请以 `apps/agena-tui/src/commands.rs` 为准。

### 5.1 会话与导航类命令

| Command | Aliases | 说明 |
|---|---|---|
| `/help` | `/?` | 显示帮助 |
| `/commands` | `/palette` | 打开命令面板 |
| `/new` | `/clear` | 创建新会话 |
| `/sessions` |  | 聚焦会话，或切换到 all / roots / subtree 视图 |
| `/resume` | `/switch`, `/recent` | 打开全局会话切换器，恢复最近工作 |
| `/lineage` | `/branch-history`, `/branches` | 打开当前或所选会话的分支历史选择器 |
| `/rewind` | `/backtrack` | 打开消息选择器，把当前会话回退到某条消息 |
| `/search` |  | 搜索会话，或打开会话搜索界面 |
| `/find` |  | 搜索 transcript，或打开 transcript 搜索界面 |
| `/rename` | `/title` | 重命名当前或所选会话 |
| `/timeline` | `/events` | 打开当前或所选会话的可搜索事件时间线 |
| `/continue` | `/resume-run` | 继续当前被阻塞或挂起的会话 |
| `/fork` | `/branch` | 从当前会话创建一个子会话 |
| `/children` | `/child` | 打开子会话选择器 |
| `/parent` |  | 跳转到父会话 |
| `/queue` | `/q` | 查看或管理待处理消息队列 |
| `/btw` | `/aside`, `/side` | 在不打断当前回合的情况下，把侧问题发到子会话 |

### 5.2 运行时检查与状态查看命令

| Command | Aliases | 说明 |
|---|---|---|
| `/plugins` | `/plugin` | 打开可搜索的 plugin 检查器，查看运行状态与最近日志 |
| `/mcp` |  | 打开可搜索的 MCP 检查器 |
| `/lsp` |  | 打开可搜索的 LSP 检查器 |
| `/skills` | `/skill` | 打开可搜索的 skills 检查器 |
| `/runtime` | `/operator` | 打开可搜索的 runtime summary 检查器 |
| `/cost` | `/usage` | 查看会话 token 与成本使用情况 |
| `/permissions` | `/perm` | 查看持久化 permission rules |
| `/config` |  | 查看 resolved config 和 runtime configuration |
| `/worktree` | `/wt` | 查看 active 和 managed worktrees |
| `/git` | `/repo` | 查看 git branch、diff 与 worktree 状态 |
| `/diagnostics` | `/feedback` | 显示用于反馈的脱敏诊断摘要 |
| `/status` |  | 显示当前 runtime override 摘要 |
| `/memory` | `/mem` | 浏览、编辑或删除已保存的 memory 文件 |

### 5.3 Git / review 工作流命令

| Command | Aliases | 说明 |
|---|---|---|
| `/review` |  | 在当前会话中分发 runtime `review` entry |
| `/commit` |  | 基于已暂存变更创建 commit |
| `/pr` |  | 用最小化 gh 参数创建 pull request |

### 5.4 Provider 与模型选择命令

| Command | Aliases | 说明 |
|---|---|---|
| `/providers` |  | 列出已配置 providers |
| `/provider` |  | 选择 provider，或清除当前 provider/model override |
| `/models` |  | 列出某个 provider 的 models |
| `/model` |  | 选择 model，或清除当前 model override |
| `/temperature` | `/temp` | 设置或清除 temperature override |
| `/max-output` | `/max-tokens` | 设置或清除最大输出 token override |
| `/system` |  | 设置或清除 system prompt override |

### 5.5 输入、审批与编辑器相关命令

| Command | Aliases | 说明 |
|---|---|---|
| `/user-input` | `/reply` | 回复第一个待处理的 user-input 请求 |
| `/allow` |  | 一次性允许第一个待处理的 permission 请求 |
| `/allow-always` |  | 永久允许第一个待处理的 permission 请求 |
| `/deny` |  | 一次性拒绝第一个待处理的 permission 请求 |
| `/deny-always` |  | 永久拒绝第一个待处理的 permission 请求 |
| `/attach` | `/file` | 打开文件附加面板 |
| `/editor` | `/edit` | 为 composer 打开外部编辑器 |
| `/image` | `/paste-image` | 从剪贴板附加图片 |
| `/copy` | `/yank` | 复制当前已加载 transcript |
| `/copy-visible` |  | 复制当前可见 transcript 视口 |
| `/export` | `/save` | 将当前 transcript 导出为 markdown 并在编辑器中打开 |
| `/pager` | `/view`, `/less` | 在终端分页器中打开当前 transcript |

---

## 6. In-tree provider 实现

provider 实现位于 `crates/agena/src/provider/` 下，并通过 `crates/agena/src/config/registry.rs` 与 `crates/agena/src/provider/registry.rs` 中的 provider registry 组装。

它们不是 model tools，但依然属于 in-tree 扩展面，因为它们定义了运行时可实例化的 model backends。

### 6.1 In-tree provider backends

| Provider 实现 | 源码文件 |
|---|---|
| Anthropic | `crates/agena/src/provider/anthropic.rs` |
| OpenAI | `crates/agena/src/provider/openai.rs` |
| OpenAI-compatible | `crates/agena/src/provider/openai_compatible.rs` |
| Amazon Bedrock | `crates/agena/src/provider/amazon_bedrock.rs` |
| Gemini | `crates/agena/src/provider/gemini.rs` |
| Google Vertex | `crates/agena/src/provider/google_vertex.rs` |
| Ollama | `crates/agena/src/provider/ollama.rs` |
| GitHub Copilot | `crates/agena/src/provider/copilot.rs` |
| Codex | `crates/agena/src/provider/codex.rs` |
| GitLab | `crates/agena/src/provider/gitlab.rs` |
| Opencode | `crates/agena/src/provider/opencode.rs` |
| Cloudflare AI Gateway | `crates/agena/src/provider/cloudflare_ai_gateway.rs` |

### 6.2 provider registry 与 runtime-added providers 的边界

本文只覆盖上面这些 in-tree provider 实现。

本文不覆盖：

- 在配置里声明的 provider aliases
- plugin 通过 `provider_list` 在运行时注入的 provider 增删
- 运行时加入的远程或自定义 provider

---

## 7. 明确排除项

为了让文档始终聚焦在“编译期 in-tree 能力”而不是“运行时内容”，下面这些能力被有意排除。

### 7.1 磁盘发现的 markdown skills 与 commands

markdown discovery 系统会把这些文件交给 `agena.skills_fs`，再通过 entry registry 注册成 dynamic plugin entries。

它们属于运行时内容，不是固定编译期 entry 名称。

### 7.2 MCP 动态生成的 entries

`agena.mcp` 是 in-tree first-party plugin 实现，但它暴露出来的具体 entry 名称依赖运行时配置的 MCP servers。

### 7.3 配置驱动的 shell hooks

`agena-shell-hooks` 是 in-tree 的，但它执行的 hook 命令来自用户配置，因此不属于固定编译期能力。

---

## 8. 维护检查清单

更新本文档时，建议优先核对以下位置：

1. `crates/agena/src/config/registry.rs` —— 静态注册了哪些 first-party plugins
2. `crates/agena/src/entry/catalog.rs` —— 编译期固定 entry catalog 如何投影
3. `crates/agena/src/plugins/bundled/*.rs` —— 对模型可见的 entry 名称
4. `crates/agena/src/plugins/bundled/skills_fs.rs` —— dynamic discovery 行为
5. `crates/agena-skills/src/bundled/` —— bundled workflow markdown
6. `apps/agena-tui/src/commands.rs` —— in-tree slash commands 列表
7. `apps/agena-tui/locales/en-US/main.ftl` —— command summaries
8. `crates/agena/src/provider/mod.rs` 与 `crates/agena/src/provider/` —— in-tree provider 实现

如果未来新增 first-party plugin，建议先判断它属于以下哪一类，再更新本文档：

- 编译期固定的 model-visible entries
- 运行时动态生成的 entry surfaces
- bundled workflow 内容
- in-tree slash commands
- in-tree providers

然后同步更新本文档。
