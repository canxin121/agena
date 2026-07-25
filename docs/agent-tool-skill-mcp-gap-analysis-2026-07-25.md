# Agena 内置 Tool、Skill 与 MCP 能力差距审计

> 审计日期：2026-07-25（Asia/Shanghai）
> 审计对象：Agena、OpenAI Codex、Anthropic Claude Code、Google Gemini CLI、SpaceXAI Grok Build
> 输入基准：`/Users/canxin/Downloads/agent_tools_skills_mcp_2026-07-24.md`
> 目标：明确 Agena 已有能力、真实可用边界、相对缺口、可借鉴设计，以及可直接进入工程计划的改进方案。

> 阅读提示：第 1–12 节保留对 commit `ee978771...` 的审计快照与当时的缺口判断；第 13 节是本轮实施后的事实状态。凡两者冲突，以第 13 节和 machine-readable capability manifest 为准。这样既保留“为什么改”，也避免把已修复问题继续写成当前缺口。

> 交付复核（2026-07-25）：本报告已重新以本机 `codex`、`gemini-cli`、`grok-build` 源码和 Agena 当前工作树核验。Tool 的统计包含模型可直接调用的工具、延迟发现后可调用的工具、以及 MCP server 在运行时提供的工具；第三方 MCP 的具体 `tools/list` 数量是运行时开放集合，故报告审计其发现、调用、认证、权限和内容保真机制，而不虚构一个静态“全量第三方工具表”。本地 `claude-code` 是第三方 sourcemap 整理镜像，仍只作为实现模式参考，Claude 的产品事实以给定调研笔记所引用的官方文档为准。

> 当前结论：Agena 已经从审计基线的 18 plugins / 62 个静态 Tool / 3 个 bundled Skill 演进到经 manifest 验证的 **22 / 100 / 14**；Skill 激活（包括会话私有、hash 校验的重建恢复）、request-driven refresh、跨平台 filesystem watcher 的安全失效通知、单 Skill enable policy、path-gated implicit invocation 与 plugin manifest contribution、混合 Tool 暴露及 route 级 Direct schema budget、MCP 协议保真与标准 OAuth、RFC 7009 可选 revocation、bearer→OAuth 显式迁移诊断、MCP include/exclude 与 annotations 高风险审批、OAuth 凭据健康投影、带累计 token/cost budget 的异步任务、Shell Monitor、SQLite Scheduler、Notebook、交互浏览器和受控下载等基线 P0/P1 已有可执行闭环。当前不应再以“补工具数量”为主要目标；尚未具备完整闭环的高风险项是跨 job history 的集中 retention/export、统一图像生成/编辑 Host API，以及浏览器对动态/JS/cookie 驱动跳转的 CDP Fetch 逐请求代理式拦截。每一项的源码证据、边界和落地顺序见第 13 节。

## 1. 审计基线结论（实施前快照）

Agena 并不缺少一个“工具框架”。相反，在审计基线 commit 上，它已经具备一套相当完整且有辨识度的底座：默认 feature 下 bundled catalog 的 18 个插件条目、62 个源码静态工具定义、统一 Tool API gateway、动态插件注册表、细粒度权限标签、LSP、MCP client/server、结构化计划、后台进程、子 Agent、持久记忆、Web 搜索/抓取/爬取、repo snapshot，以及多 Provider 的原生工具适配层。这里的 18/62 是当时的源码能力上限，不代表单个会话一定全部注册；`schema-lab` 受默认 feature 控制，MCP execution plugin 只有在 runtime 拿到 MCP manager 时才进入实际注册表。

真正的差距集中在四类问题：Skill 体系只有外形没有完整执行语义、所有工具都经过五函数 gateway 带来可靠性与额外回合成本、MCP 协议支持尚未达到生产级完整度、异步任务和长时监控尚未形成统一工具闭环。它们比只追求工具数量更优先。

### 1.1 基线最高优先级结论

| 优先级 | 结论 | 当前证据 | 建议 |
| --- | --- | --- | --- |
| P0 | Skill 默认发现实际被关闭 | [`agena-skills/src/discovery.rs`](../crates/agena-skills/src/discovery.rs) 的 `default_roots` 和 `default_command_roots` 都返回空数组 | 恢复标准 user/workspace/compat roots，并加入 trust、precedence、watch、错误诊断 |
| P0 | Skill 元数据没有执行语义 | `allowed_tools`、`model`、`aliases` 被解析，但 [`skills.rs`](../crates/agena-runtime-plugins/src/plugins/provided/skills.rs) 的 `run` 只返回 prompt 文本；没有工具收窄、模型切换或 alias 解析 | 设计真正的 Skill activation/injection 生命周期，约束必须由 runtime 强制执行 |
| P0 | MCP 会截断协议语义 | resources/prompts 只请求第一页；没有 resource templates；`ContentBlock::Other` 被丢弃；没有 structured content/audio/annotations | 补齐 pagination、content fidelity、capability negotiation 与变更通知 |
| P0/P1 | 五函数 gateway 不应覆盖所有高频本地工具 | 当前模型只见 `tools_list/search/help/tags/call`；大量纠错提示与测试都在处理 execution-tool 名称误当 function name | 改成 Direct + Deferred + Hidden 的混合暴露：高频核心工具直出，长尾/动态工具继续 gateway |
| P1 | 子 Agent 只能同步 `tasks.run` | 可创建或恢复子任务，但没有 background/list/wait/read/cancel/message/follow-up/team | 采用任务句柄和异步状态机，补齐并行协作工具组 |
| P1 | Monitor 内核存在但没有融入模型可见的 shell 生命周期 | host API 已有 start/list/read/stop，`agena.shell` 已有 run/list/logs/stop，但两者尚未形成统一句柄与参数 | 在 `shell.run` 增加 monitor 条件参数，由现有 `shell.list/logs/stop` 统一查询、读取和停止，不新增独立插件或工具 |
| P1 | Scheduler 名称与实际 durability 不一致 | crate 顶部称 persistent、还声称 runtime 使用外部 SeaORM adapter，但当前 runtime 实际调用 `build_in_memory` | 增加 SQLite `JobStore`，明确重启、错过触发、幂等和时区语义 |
| P1 | 缺少官方 Skill 生态基本件 | 只有 `init`、`review`、`security_review` | 首批增加 `skill-creator`、`skill-installer`、`plugin-creator`、`doctor`、`run`、`verify`、`simplify` |
| P2 | 缺少 notebook、交互浏览器、图像生成/编辑等一等工具 | 有附件读取、浏览器渲染和 provider-native config，但没有完整可调用闭环 | 根据 Provider 能力做条件注册，不要先做不可执行的静态占位 |

### 1.2 Agena 最值得保留的设计

以下能力不是竞品的简单复刻，应该保留并继续强化：

1. 插件工具统一进入同一个 registry、权限、hook、输出和 UI 体系，而不是为 MCP、内置工具、第三方工具各维护一条执行链。
2. `ToolDefinition` 同时具有 input/output schema、权限路径、网络目标、tag、并发安全、streaming 和展示策略，契约维度比多数对照项目更完整。
3. Tool API gateway 能稳定 Provider schema、减少动态工具导致的 KV cache 失效，并适配不支持原生 function calling 的 prompt-envelope Provider。
4. `agena mcp-server` 不仅导出工具，还导出 resources 和由 Skill 投影的 prompts；这一点比只导出固定两项工具的最小 server mode 更有扩展潜力。
5. Agent profile 的权限 ceiling、tool allowlist 与父子会话交集语义是正确方向，优于仅靠 prompt 提醒子 Agent 不要越权。
6. Web 插件同时处理搜索、实际页面抓取、JS 渲染、robots、SSRF/redirect 复核、缓存、爬取和本地索引，工程完整度较高。
7. repo snapshot 把隔离工作区抽象成工具，能够覆盖 Git worktree/Rift 等后端，而不把用户界面绑死在 Git 实现上。

因此，建议不是重写 Agena，而是在现有插件/runtime 边界上补齐安全、Skill、MCP 与工具暴露策略。

## 2. 审计范围、版本与证据边界

### 2.1 本地仓库快照

| 仓库 | 本地 commit | 说明 |
| --- | --- | --- |
| Agena | `ee97877139254183bc0d40bbb916edb4479a2624` | 本报告的主要审计对象 |
| Codex | `58b427722857117ac3e702b9eb406d47616022e2` | 官方 `openai/codex`，比输入报告固定 SHA 稍晚 |
| Gemini CLI | `69b51f8fa2af0abf717daaba4dca1c627023d82d` | 与输入报告固定 SHA 一致 |
| Grok Build | `6e386420825bd44ae648c63e7c8cba12fcec9401` | 官方 `xai-org/grok-build`，比输入报告固定 SHA 稍晚 |
| `claude-code` | `a371abbe75ffa0d0a3c92290e2bbf56a7ef54367` | 第三方镜像/整理仓库，不是 Anthropic 官方公开源码仓库 |

输入报告的固定快照为 Codex `f47f28...`、Claude Code `2982f9...`、Gemini CLI `69b51f...`、Grok Build `69f0ba...`。对工具数量和公开能力的横向表，优先使用这份已固定版本并交叉核验过的输入报告；对架构借鉴，使用本地当前源码。

### 2.2 Claude Code 的特殊边界

本地 `claude-code` remote 是 `yasasbanukaofficial/claude-code`。其 README 明确称内容来自 npm sourcemap 的第三方整理，并明确声明不是 Anthropic 官方产品仓库。因此：

- 它可用于观察实现模式，例如异步 MCP 连接、ToolSearch、doctor、worktree、agent teams、memory consolidation。
- 它不能作为官方稳定 API、许可状态或当前产品行为的唯一证据。
- Claude Code 的 43 项 canonical tool、13 个 bundled Skill 和 MCP transport 结论，仍以输入报告中引用的官方 tools/commands/MCP 文档为准。

### 2.3 计数规则

本报告继续区分以下概念：

- “静态工具定义”指源码可注册的 callable tool，不代表每个会话都可见。
- Provider-native hosted tool 是 Provider API 的特殊能力，不与 Agena plugin execution tool 混计。
- MCP server 的第三方工具数量运行时无上限，不可能静态穷举。
- reminder、hook、slash command 和 Agent profile 不自动算 tool。
- Agena 的五个 Tool API gateway function 是 provider-facing callable function；其余 57 个是 gateway 后面的 execution tool。

## 3. Agena 当前能力全景

### 3.1 工具执行架构

Agena 当前是两层工具协议：

```text
Provider / model
  └─ 5 个稳定 function：tools_list / tools_search / tools_help / tools_tags / tools_call
       └─ Tool API binding
            └─ plugin tool registry
                 ├─ 57 个内置 execution tool
                 ├─ 用户 cdylib / stdio / HTTP / WASM plugin tool
                 └─ MCP bridge tool
```

关键实现：

- gateway 与 execution tool 类型隔离：[`tool_registry.rs`](../crates/agena-runtime-tools/src/tool/tool_registry.rs)
- Provider function 描述和错误修复：[`prompt_tool_transport.rs`](../crates/agena-runtime-provider/src/provider/prompt_tool_transport.rs)、[`completion.rs`](../crates/agena-runtime-provider/src/provider/registry/completion.rs)
- 插件静态注册入口：[`plugins/sources.rs`](../crates/agena-runtime-plugins/src/plugins/sources.rs)
- 工具执行、权限与 hook：[`tool/executor`](../crates/agena-runtime-tools/src/tool/executor)
- 插件 manifest：[`agena-plugin-sdk/src/manifest.rs`](../crates/agena-plugin-sdk/src/manifest.rs)

这套设计的优点是 Provider-facing schema 恒定，动态插件和 MCP 变化不会每回合重写几十个 function declarations。缺点是高频工具也被迫走嵌套 envelope，弱模型很容易把 `fs.read` 当 function name，或者把 `tools_call` 错放进 `tool` 字段。

从源码中大量“misrouted through tools_call”“only allowed Tool API function names”“execution-tool identifiers are not function names”的纠错分支可以看出，这不是理论风险，而是已经需要专门兼容层处理的实际模型行为。

### 3.2 内置插件与静态工具定义

默认 feature 包含 `schema-lab`。按当前源码和现有工具参考正文统计，bundled catalog 有 18 个插件条目、源码中共有 62 个静态工具定义；其中 5 个是 Tool API gateway，57 个是 execution tool。实际 runtime registration 的基线是 16 个无条件插件，加默认启用的 `schema-lab` 后为 17 个；只有传入 MCP manager 时才注册第 18 个 `agena.mcp` 插件及其 5 个工具。因此下表是完整源码能力表，不是“任意单会话都能看到 62 个工具”的承诺。

| 插件 | 数量 | 工具 |
| --- | ---: | --- |
| `agena.agent` | 2 | `switch`, `restore` |
| `agena.code` | 2 | `search_ast`, `syntax_tree` |
| `agena.cron` | 4 | `list`, `create`, `delete`, `wakeup` |
| `agena.fs` | 4 | `read`, `glob`, `grep`, `apply_patch` |
| `agena.interaction` | 2 | `ask`, `notify` |
| `agena.lsp` | 5 | `servers`, `definition`, `references`, `hover`, `diagnostics` |
| `agena.mcp` | 5 | `resources.list`, `resources.read`, `prompts.list`, `prompts.get`, `tools.call` |
| `agena.memory` | 5 | `search`, `get`, `list`, `write`, `delete` |
| `agena.plan` | 4 | `get`, `set`, `update`, `clear` |
| `agena.schema_lab` | 2 | `inspect`, `echo` |
| `agena.session` | 2 | `get`, `rename` |
| `agena.settings` | 7 | `get`, `list`, `inspect`, `set`, `delete`, `patch`, `validate` |
| `agena.shell` | 4 | `run`, `list`, `logs`, `stop` |
| `agena.skills` | 3 | `list`, `get`, `run` |
| `agena.snapshot` | 2 | `enter`, `exit` |
| `agena.tasks` | 1 | `run` |
| `agena.tools` | 5 | `list`, `search`, `help`, `tags`, `call` |
| `agena.web` | 3 | `fetch`, `crawl`, `search` |
| 合计 | 62 | 57 execution + 5 gateway |

现有 [`plugins-and-tools-reference.md`](plugins-and-tools-reference.md) 的标题写“61 个工具”，但正文有 62 个 `### agena.*` 工具章节，各插件表相加也是 62。应把 runtime manifest 生成和文档计数纳入 CI，避免继续手工漂移。

### 3.3 Provider-native tool

Agena 还定义了 9 个逻辑 Provider-native tool route：

```text
web_search
file_search
code_execution
image_generation
computer
bash
text_editor
url_context
remote_mcp
```

当前真正接通的 hosted 组合是：

- OpenAI Responses：`web_search`、`file_search`、`code_execution`。
- Anthropic：`web_search`。
- Gemini：`web_search`、`url_context`、`code_execution`。

仍未形成完整执行闭环的包括 `image_generation`、`remote_mcp` 和 provider-harness 的 `computer` / `bash` / `text_editor`。这些不能因为 config schema 已存在就写成“内置可用工具”。当前文档对此边界说明是准确的。

另一个限制是 Gemini hosted tool 不能和 Agena 的五个 custom Tool API functions 同请求出现。这意味着 Agena 的工具暴露规划必须是 per-provider/per-model，而不是全局固定策略。

### 3.4 内置 Agent profiles

当前内置 8 个 profile：

| Profile | 定位 | 默认限制摘要 |
| --- | --- | --- |
| `build` | 主工程 Agent | 继承全部 live tools，权限为主边界 |
| `general` | 通用研究/混合任务 | workspace 写 deny，shell ask |
| `explore` | 只读代码探索 | 明确列出 fs/code/LSP/web/shell read 路径 |
| `scout` | 外部文档与 API 研究 | web/MCP resource，可询问 shell |
| `implement` | 定向实现 | 继承工具，写操作 ask |
| `verify` | 测试与回归验证 | 只读 + shell ask |
| `planner` | 规划 | 只读 + plan + ask |
| `reviewer` | 代码审查 | 只读 + shell ask |

Agent profile 发现顺序是 built-in → `~/agena/agents/*.md` → `.agena/agents/*.md` → JSON config → runtime registration。子会话权限和工具集合只能在父会话基础上继续收紧，且子会话不能再调用 `tasks.run`。这部分设计成熟，主要短板在协作工具表面，而非 profile 数据模型。

### 3.5 Skill 当前状态

内置 Skill 只有 3 个：

| Skill | 用途 | 当前问题 |
| --- | --- | --- |
| `init` | 生成 `AGENA.md` | 可用，但没有输出路径/覆盖冲突的结构化确认 |
| `review` | 分支代码审查 | `allowed_tools` 用短名 `read/glob/grep`，当前 runtime 不执行这些约束 |
| `security_review` | 安全审计 | 同上；只提供 prompt，没有结构化 finding 协议 |

目前 Skill 存在以下实质缺口：

1. `default_roots()` 和 `default_command_roots()` 返回空，所以 user/workspace Skill 和 slash command 实际不会被默认发现。
2. `aliases` 只在 `Skill::matches()` 中实现，但 `SkillsPlugin::resolve_tool()` 不调用它，因此 `bootstrap` 和 `security-review` alias 无法通过 `skills.run` 解析。
3. `allowed_tools` 只在 frontmatter 中保存，`skills.run` 不修改 session allowlist，也不创建受限子会话。
4. `model` 只解析不使用。
5. `skills.run` 把 Skill body 作为普通 tool output 返回，而不是受类型约束的 system/developer context fragment；后续模型可以遵守，也可以忽略。
6. 列表/读取结果不返回 `source_path`、scope、provider/plugin provenance、content hash、trust 状态或依赖。
7. 没有 invalid Skill 诊断列表；扫描错误仅写 warning log。
8. 没有 hot reload/file watch。
9. Skill 不能携带并通过受控 API 读取 `scripts/`、`references/`、`assets/` 等资源包。
10. 插件 manifest 只有 tools、commands、hooks 和 UI contribution，没有标准 `skills` contribution。
11. 没有 Skill enable/disable policy、implicit invocation policy、path gating、产品限制或上下文预算。
12. 没有 Skill 依赖解析，例如缺少 MCP server 时提示安装/登录。

因此，当前 Agena 的 Skill 更接近“命名 prompt 模板”，还不是 Codex/Gemini/Grok 意义上的完整 Agent Skill runtime。

### 3.6 MCP 当前状态

#### Client

已实现：

- STDIO。
- Streamable HTTP。
- 静态 headers、直接 bearer、env bearer、file token store bearer、自定义 headers。
- tools、resources、prompts 的 list/read/get/call 基本路径。
- 运行时 `add_server` / `remove_server` host API。
- MCP tool 通过 `agena.mcp.tools.call` 间接调用。

缺失或不完整：

- OAuth discovery、authorization code、PKCE、dynamic client registration、refresh、logout。
- token keyring；当前默认 JSON token store 虽在 Unix chmod 600，仍不是系统密钥存储。
- SSE/WebSocket。SSE 已是旧 transport，优先级可以较低；OAuth 不能因此延后。
- initialization `instructions` 的读取和注入。
- roots/list 与 roots/list_changed。
- resources/templates/list。
- tools/list_changed、resources/list_changed、prompts/list_changed 的实时处理。
- server 启动 timeout、tool timeout、per-tool timeout、include/exclude tools。
- CLI `mcp add/list/get/remove/login/logout/enable/disable`。
- 连接状态、重连/backoff、延迟连接和 turn-1 非阻塞策略。
- 完整分页：`list_resources(None)` 和 `list_prompts(None)` 只获取首批结果；返回了 `next_cursor`，但 tool input 无 cursor，调用方无法取下一页。
- 完整 content：协议只保留 text/image/resource，其他 block 被映射为 `Other` 后丢弃；没有 audio、annotations、resource link、structured content、output schema、`_meta`。
- 工具风险分类：所有第三方 MCP tool 在 Agena manifest 上都由一个 `mutating` 的 `mcp.tools.call` 代表，无法按具体 tool 的 read-only/destructive/open-world hints 做细分审批。

#### Agena 作为 MCP server

`agena mcp-server` 已通过 STDIO 导出：

- 当前 runtime execution tools，且是直接 tool，不是五函数 gateway。
- `agena://workspace` 和可选 `agena://sessions` resources。
- 当前 Skills/commands 作为 MCP prompts。

缺口包括：

- 仅 STDIO，无 Streamable HTTP server transport。
- list 返回不分页，未声明 list_changed。
- tool result 只输出最终 text，丢失 Agena `payload`、metadata、attachments 和 structured content。
- MCP server 调用使用 `session_id = -1`，工具的会话关联、交互 ask、后台任务归属和审计上下文需要进一步明确。
- resources 暴露会话列表时需要更清楚的隐私/授权范围。

## 4. 跨产品能力矩阵

符号：`✓` 已形成公开可用闭环；`△` 有部分实现或条件能力；`—` 未发现。

| 能力 | Agena | Codex | Claude Code | Gemini CLI | Grok Build |
| --- | --- | --- | --- | --- | --- |
| 高频文件工具直接暴露 | — | ✓ | ✓ | ✓ | ✓ |
| 延迟工具发现/meta-tool | ✓，所有 execution tool | ✓，Deferred/Code Mode | ✓，ToolSearch | △，registry/discovery | ✓，MCP search/use |
| 精确 patch/edit | ✓ apply_patch | ✓ | ✓ Edit | ✓ replace/write | ✓ 多套 edit |
| write/create 专用工具 | — | △ apply_patch/shell | ✓ | ✓ | ✓ |
| 批量读文件 | — | shell/exec | Read 可组合 | ✓ read_many_files | — |
| LSP | ✓ 5 项 | 条件/扩展 | ✓ | —（主清单） | ✓ |
| AST 结构搜索 | ✓ | shell 可调用外部 | LSP/Grep | — | — |
| 后台 shell | ✓ | ✓ exec/write_stdin | ✓ Task/Bash | ✓ 两个专用工具 | ✓ |
| OS 级 shell sandbox | — | ✓ 多平台 | ✓/平台相关 | ✓ 可配置 | ✓ workspace policy |
| 结构化 ask user | ✓ | ✓ | ✓ | ✓ | ✓ |
| Plan mode/state | ✓ 状态机+autorun | ✓ update_plan | ✓ Enter/Exit | ✓ Enter/Exit + tracker | ✓ Enter/Exit + todo/goal |
| Goal 工具 | — | ✓ 3 项 | Task 系统可替代 | Tracker | ✓ update_goal |
| 异步子 Agent 管理 | — | ✓ 6 项 v2 | ✓ Agent/team/task | △ invoke_agent | ✓ 统一 task handles |
| 同步子 Agent | ✓ tasks.run | ✓ | ✓ | ✓ | ✓ |
| Monitor 工具 | 内核有、未暴露 | 环境 wait | ✓ | 后台进程工具 | ✓ |
| Scheduler/Cron | ✓，当前内存态 | clock sleep 条件 | ✓ | — | ✓ |
| 持久记忆 | ✓ | ✓ extension | ✓/动态 | △ auto memory | ✓ backend 条件 |
| 公网 search + fetch | ✓ | ✓ extension/hosted | ✓ | ✓ | ✓ |
| 交互浏览器/computer use | △ 只渲染抓取 | extension/插件条件 | Chrome/Artifact 条件 | — | 条件工具表面 |
| 本地图片查看 | ✓ 通过 `fs.read` attachment | ✓ view_image | Read/Artifact 条件 | read_file 多模态 | read_file/media |
| 图像生成/编辑 | 配置有，闭环未完成 | ✓ imagegen | Artifact 条件 | — | ✓ |
| Notebook edit | — | —（固定表） | ✓ | — | — |
| MCP resources | ✓ list/read | ✓ list/templates/read | ✓ list/read | ✓ list/read | 经 index/dynamic |
| MCP prompts | ✓ list/get | host 表面依赖 | ✓ slash command | ✓ slash command | 插件/表面依赖 |
| MCP OAuth | — | ✓ | ✓ | ✓ | ✓ |
| MCP roots/change events | — | 部分/宿主 | ✓ roots | ✓ roots | △ index fingerprint |
| 自身 MCP server | ✓ STDIO | ✓ | ✓ | — | — |
| User/workspace Skill 默认发现 | —，代码主动关闭 | ✓ | ✓ | ✓ | ✓ |
| Bundled Skill 数量 | 3 | 6 | 13 | 1 | 服务端动态 |
| Skill 资源包 | — | ✓ | ✓/插件 | ✓ 目录可读 | ✓ |
| Skill creator/installer | — | ✓/✓ | generator | ✓/— | create-skill bundle |
| Plugin marketplace | ✓ | ✓ | ✓ | extensions | ✓ |
| Plugin 携带 Skill | — | ✓ | ✓ | ✓ | ✓ |
| Doctor/health Skill/命令 | diagnostics 命令 | doctor CLI | ✓ `/doctor` | `/doctor` 类能力分散 | inspect |

## 5. 对照实现中最值得借鉴的部分

### 5.1 Codex：工具暴露级别，而不是全直出或全 gateway

Codex 在 [`spec_plan.rs`](../../codex/codex-rs/core/src/tools/spec_plan.rs) 中把 runtime tool 分为 `Direct`、`Deferred`、`DirectModelOnly`、`Hidden`，再按 model、environment、feature 和 Code Mode 规划本回合工具。其核心价值是：

- 高频且模型熟悉的 shell、patch、image 等直接出现。
- 大量 app/MCP/extension tool 可以 deferred，通过 `tool_search` 或 Code Mode 获取。
- handler registry 和 model-visible spec 分离，隐藏工具仍可被内部 dispatcher 使用。
- 不同模型可拥有不同 surface，而不是把 provider 差异塞进 prompt 纠错。

Agena 应借鉴的是 exposure planner，而不是复制 Codex 的工具名称。

建议给 `ToolDefinition` 增加：

```rust
enum ToolExposure {
    Direct,
    Deferred,
    Hidden,
    Internal,
}
```

再由 per-model planner 产生：

- Provider function declarations。
- Tool API catalog entries。
- internal-only registry。

### 5.2 Codex：Skill 是带来源、接口、依赖与策略的资源包

Codex 的 [`skills/src/model.rs`](../../codex/codex-rs/skills/src/model.rs) 为 Skill 定义：

- scope、path、plugin id。
- display name、short description、icons、brand color、default prompt。
- tool dependencies，包含 transport/command/url。
- implicit invocation policy 和 product restriction。

其 Skills extension 又做了：

- host/executor/orchestrator 多 authority provider。
- `skills.list` / `skills.read` 的严格 handle、分页和响应大小上限。
- Skill catalog 的 token/character budget；即使超预算，也先保留名称和 locator。
- Skill MCP dependencies 的安装确认、OAuth 和 runtime refresh。
- invalid skill warning、watcher、enable/disable config 和 invocation telemetry。

Agena 当前已有插件 source/provenance、Tool API pagination、权限系统和 Studio runtime 页面，具备实现类似能力的基础，缺的是把这些基础连接到 Skill。

### 5.3 Gemini CLI：标准 roots、folder trust 与激活确认

Gemini [`skillManager.ts`](../../gemini-cli/packages/core/src/skills/skillManager.ts) 的发现顺序清楚：built-in → extension → user `.gemini/skills` / `.agents/skills` → trusted workspace `.gemini/skills` / `.agents/skills`。workspace 未 trusted 时不加载。

其 `activate_skill` 还会：

- 对非 built-in Skill 显示说明和将共享给模型的资源目录，要求确认。
- 把 Skill 根目录加入 workspace context，允许后续受控读取资源。
- 返回明确的 `<activated_skill>` 结构，而不是无类型普通文本。
- 动态更新 skill name schema。

Agena 可直接借鉴 trust + provenance + activation confirmation，但应比 Gemini 更进一步：`allowed_tools` 必须成为 runtime 强制边界。

### 5.4 Grok Build：核心工具直出，只有 MCP 使用 search/use

Grok Build 默认把本地文件、shell、task、scheduler、monitor、goal 等 18 项核心工具直接提供给模型，只把动态 MCP 工具放在 `search_tool` + `use_tool` 后面。这恰好是 Agena gateway 改造最合适的参考点。

其 [`search_tool`](../../grok-build/crates/codegen/xai-grok-tools/src/implementations/search_tool/mod.rs) 还提供：

- BM25 排序，而非简单 substring。
- 按 server 分组的结果。
- input schema 同结果返回。
- server/tool fingerprint 和新增、更新、断开的 delta reminder。
- 描述长度限制与稳定 hash。

另外，Grok 的 [`SkillDiscoveryReminder`](../../grok-build/crates/codegen/xai-grok-tools/src/reminders/skill_discovery.rs) 会在文件工具访问路径后发现附近 Skill，并对 `paths:` gated Skill 做动态激活。这一机制适合 Agena 的 plugin hook 体系，但应避免每次工具调用做无界目录遍历。

### 5.5 Claude Code：产品化能力而非单个工具

Claude Code 可借鉴的重点不是 43 个名称，而是以下完整产品闭环：

- `TaskCreate/Get/List/Update/Stop` 加后台输出，而不是只有一个同步 Agent 调用。
- Worktree enter/exit 与 batch 并行 workflow。
- `/doctor` 统一诊断安装、配置、MCP、Plugin、Skill 和上下文。
- `/fewer-permission-prompts` 从历史调用生成更精确的 allowlist。
- `/run`、`/verify`、`/run-skill-generator` 把“启动应用并验证”产品化。
- `ReportFindings` 提供结构化 review 输出协议。
- MCP 在 UI 中有 status、认证、启停、重连，连接在交互模式下不会无条件阻塞首屏。

这些多数可以先做成 Agena Skill + 少量工具，而不必都进入核心 runtime。

## 6. 详细缺口与改进设计

### 6.1 当前明确非目标：OS 级 sandbox

Agena 当前选择不建设 OS 级 sandbox。`agena.shell.run` 继续使用 `filesystem_effects`、`network_effects`、permission rule、用户确认和审计日志作为权限治理手段；这是一套 approval/policy 机制，不应在产品文案中描述成强制文件系统或网络隔离。

本轮工程不引入 Seatbelt、bubblewrap、Landlock、seccomp、Windows restricted token 或容器执行层，也不把这些工作列为交付阻塞项。仍应保留以下低复杂度约束：

- 对模型声明与 command analyzer 结果不一致的调用提高审批级别并记录原因。
- 日志和 `settings.inspect` 中继续隐藏环境 secret。
- 超时、取消和停止必须回收完整进程树。
- Plugin/MCP/shell 的权限标签继续进入统一 permission contract。
- 文档和 UI 明确说明 effect declaration 是审批信息，而不是 OS enforcement。

### 6.2 P0：把 Skill 从 prompt 模板升级为执行能力

#### 推荐发现层

默认 roots 建议按低到高优先级：

1. bundled/system。
2. plugin-provided。
3. `~/agena/skills`。
4. `~/.agents/skills` 兼容目录。
5. `<workspace>/.agena/skills`。
6. `<workspace>/.agents/skills`。
7. 显式 config roots。

对 `.claude/skills`、`.gemini/skills`、`.grok/skills`、`.cursor/skills` 使用显式 compat 开关，不应默认无条件扫描所有生态目录。workspace roots 仅在 workspace trusted 时加载。

#### 推荐 Skill 模型

```yaml
---
name: verify
description: Run focused validation for the current change.
aliases: [check]
allowed-tools:
  - agena.fs.read
  - agena.fs.glob
  - agena.fs.grep
  - agena.shell.run
model: inherit
user-invocable: true
allow-implicit-invocation: false
paths:
  - "**/*.rs"
dependencies:
  tools: [agena.shell.run]
  mcp: []
---
```

内部模型至少应包含：

- canonical name、aliases、description、short description。
- scope/source/provider/plugin、source path 或 opaque resource locator。
- body/main resource、resource root、content hash、mtime。
- allowed tools、model preference、invocation policy、path gating。
- dependencies、required environment、compatibility。
- enabled、trust、validation warnings。

#### 推荐 activation 语义

1. `skills.activate` 或统一 `skills.read` 返回 typed `SkillActivation`，不是普通 `ToolInvokeOutput`。
2. runtime 将正文作为有大小上限、可追踪来源的 developer/system fragment 注入下一回合。
3. `allowed_tools` 与当前 session allowlist 取交集；这是 runtime enforcement，不是 prompt 建议。
4. `model` 只有在 policy 允许时产生下一回合 selection override；否则返回明确诊断。
5. alias 在 resolver 层解析并返回 canonical name。
6. 非 built-in Skill 首次激活时展示来源、资源、请求的工具/网络依赖和 trust 状态。
7. 激活后只允许按 locator 分页读取 Skill 根目录内资源，防止 `../` 越界。
8. 为显式、隐式、slash、MCP prompt 四种 invocation 分别记录 telemetry。

#### 上下文预算

借鉴 Codex：可见 catalog 总预算限制为 context window 的固定比例，例如 2%，上限 4K tokens。超预算策略应是：

1. 截断长 description。
2. 保留所有 name + locator。
3. 再按 ranking 省略低相关 description。
4. 发出一次 bounded warning，而不是静默丢失。

### 6.3 P0/P1：从全 gateway 改成混合工具暴露

#### 建议默认直出的核心工具

第一阶段建议直出 10–14 个，不要一次把全部 57 项直出：

```text
fs.read
fs.glob
fs.grep
fs.apply_patch
shell.run
shell.list
shell.logs
shell.stop
interaction.ask
tasks.run（异步化后改 tasks.create）
plan.get
plan.update
web.search
web.fetch
```

`lsp.*` 可在配置了 server 时直出；`settings.*`、`memory.*`、`schema_lab.*`、低频 session/agent 工具保留 deferred。第三方 MCP 永远优先 deferred。

#### Planner 规则

模型可见工具应由以下输入共同决定：

- Provider/adapter 能力。
- model family/tool schema 限制。
- Agent profile allowlist 和 permission ceiling。
- 当前 plan mode。
- plugin enabled/health。
- runtime feature flags。
- tool exposure policy。
- schema token budget。

#### 缓存与稳定性

- 同一 session 的直出核心集合保持稳定，除非显式 profile/mode 切换。
- 动态 MCP/Plugin 变化只更新 deferred index，避免 function schema 抖动。
- 每个 tool definition 计算 fingerprint；只在 fingerprint 变化时失效缓存。
- 记录 direct/deferred 命中率、gateway 多余回合、schema validation retry 和错误修复次数，用数据决定哪些工具应该直出。

#### 兼容策略

- 保留五函数 gateway，prompt-envelope Provider 继续只用 gateway。
- provider-protocol 且支持 function calling 的模型启用 hybrid。
- 旧 session rollout 按保存的 tool protocol version 恢复，避免历史 tool call identity 失配。
- MCP server mode 继续导出直接 execution tools，不导出 gateway wrapper。

### 6.4 P0：补齐 MCP 协议与运维闭环

#### 协议层

按优先级补齐：

1. resources/prompts cursor 输入及 `list_all_*` helper。
2. resource templates list/read template URI。
3. initialization instructions 保存、限制大小、注入来源标记。
4. tools/resources/prompts list_changed notification。
5. roots/list 和 roots/list_changed；只暴露当前 workspace 与显式 add-dir roots。
6. structuredContent、audio、resource link、annotations、`_meta` 原样保留。
7. tool execution read-only/destructive/idempotent/open-world hints 映射到 Agena tags 和 permission mode。
8. sampling/elicitation 若不支持，应在 capabilities 中明确不声明。

#### OAuth 与 secret

实现标准 OAuth 流程：

- protected resource metadata / authorization server discovery。
- PKCE。
- dynamic client registration 与显式 client id 两种路径。
- loopback callback + headless/manual code fallback。
- refresh token、expiry、revocation/logout。
- keyring 优先，encrypted/file fallback 明确提示。
- HTTP headers 支持从 env/keyring 引用，不把 secret 展示在 `settings.inspect` 或日志。

#### 配置与 CLI

建议增加：

```text
agena mcp add
agena mcp list
agena mcp get
agena mcp remove
agena mcp enable
agena mcp disable
agena mcp login
agena mcp logout
agena mcp reconnect
```

每个 server 增加：

- `enabled`。
- `startup_timeout_ms`。
- `tool_timeout_ms` 与 per-tool override。
- `include_tools` / `exclude_tools`。
- `required`：required server 连接失败是否阻止会话。
- reconnect/backoff。
- trust/source/provenance。

#### 动态工具 index

不要把前 24 个 MCP tools 拼进 `mcp.tools.call` 的长 help。建立专门 index：

- BM25/fielded search：server、tool name、description、schema fields。
- 返回 exact qualified handle、schema fingerprint、read/write hints。
- `mcp.tools.call` 要求 handle fingerprint；过期时返回 stale tool 并触发重新 search。
- server 变更产生一次 delta event/reminder，不重复塞入每个 tool definition。

### 6.5 P1：异步多 Agent 与任务系统

把当前 `tasks.run` 拆成状态机：

```text
tasks.create      -> task_id
tasks.list        -> task summaries
tasks.get         -> metadata/status
tasks.output      -> incremental output with cursor
tasks.wait        -> wait one or many task_ids
tasks.cancel      -> cooperative then hard cancel
tasks.message     -> send message/input
tasks.followup    -> reuse context for a follow-up
```

设计要求：

- `background` 明确决定同步/异步。
- 支持一次 wait 多个 task id，以便真正并行。
- task 与 shell process 可共享统一 wait/output/kill 抽象，但 UI 要保留类型。
- 设最大 depth、最大并发、全线程总配额、token budget 和 timeout。
- 子 Agent 结果应包含 status、usage、changed files、tool calls、final text、error。
- 父会话结束时明确选择 cancel、detach 或持久化。
- 支持 dependency DAG，但首版不必实现自由 team chat。
- 消息工具必须保留 permission ceiling，不能借消息让子 Agent 执行父 Agent 无权操作。

### 6.6 P1：把 Monitor 融入 Shell 生命周期

不新增 `agena.monitor` plugin 或独立工具。复用已有 monitor host API，并把能力投影到现有四个 shell 工具：

- `shell.run` 增加可选 `monitor` 对象，支持成功/失败 string 或 regex、quiet period、timeout 和 persistent。
- `shell.run` 返回统一 `process_id`、kind=`process|monitor` 和初始状态；普通后台进程与监控进程使用同一句柄协议。
- `shell.list` 同时列出普通 background process 和 monitor，并允许按 kind/status 过滤。
- `shell.logs` 用 cursor 增量读取二者输出，同时返回条件匹配和最终状态。
- `shell.stop` 对二者统一执行 cooperative stop，再按超时 hard kill。

这样既覆盖持续健康检查和“babysit until done”场景，也避免模型在 `shell.*` 与 `monitor.*` 两套近似生命周期工具之间选择错误。

### 6.7 P1：让 Scheduler 真正持久化

当前 `InMemoryJobStore` 在进程重启后丢失。建议新增 SQLite store，并定义：

- cron 表达式时区。
- one-shot 过期后的 misfire policy：skip/run-now/reschedule。
- 至少一次 vs 至多一次投递。
- 幂等 key。
- 最大并发与失败 backoff。
- 创建者 session/profile/permission snapshot。
- job 修改、pause/resume、history 查询。
- scheduler 重启恢复测试。

工具表面可扩展为 `list/create/update/delete/pause/resume/history`，但 durable store 应先于新增工具。

### 6.8 P1：补足本地高频编辑与读取工具

`apply_patch` 很强，但并非所有模型都擅长补丁语法。建议增加：

- `fs.write`：创建/覆盖文件，默认覆盖已有文件需额外确认或 precondition hash。
- `fs.replace`：old/new 精确替换，支持 expected occurrences。
- `fs.read_many`：有总 byte/token budget 的多文件聚合读取。
- `fs.stat`：文件类型、大小、mtime、hash，不必通过 shell。

所有 mutating 工具应支持 optimistic concurrency：调用方可提供 `expected_sha256` 或读取时得到的 revision，避免并行 Agent 覆盖彼此修改。

### 6.9 P1/P2：结构化 review、doctor 与验证工作流

建议增加一个通用 `report.findings` tool，而不是让 review Skill 只输出 Markdown：

```json
{
  "findings": [
    {
      "severity": "high",
      "file": "src/auth.rs",
      "line": 42,
      "title": "...",
      "body": "...",
      "confidence": 0.95
    }
  ]
}
```

它可被 TUI/Studio/PR integration 统一渲染、去重和筛选。

同时新增 bundled Skills：

| Skill | 首版职责 |
| --- | --- |
| `doctor` | 检查 config、provider auth、shell permission/effect policy、MCP、plugin、Skill、LSP、DB、workspace trust |
| `run` | 识别并启动项目，返回可复用 process handle |
| `verify` | 根据 diff 和项目约定运行最小充分验证 |
| `run-skill-generator` | 为项目生成 `.agena/skills/run` / `verify` |
| `simplify` | 查重复、复杂度和无效抽象，可调用 reviewer 子 Agent |
| `debug` | 汇总日志、runtime snapshot、plugin/MCP health |
| `code-review` | 使用结构化 findings；可选修复但默认只报告 |
| `batch` | 基于 snapshot + async tasks 做独立改动并行化 |

### 6.10 P2：媒体、Notebook 与浏览器

#### 已有基础

- `fs.read mode=attachment` 可传 image/PDF/audio/video/file attachment。
- Web fetch 可选本地浏览器渲染 JS。
- Provider-native schema 已有 image generation/computer harness 概念。

#### 缺口

- 没有明确的 `view_image` UX 和 detail/original 控制。
- 没有 image generation/edit tool 的跨 Provider abstraction 和 artifact 保存语义。
- 没有交互浏览器 click/type/screenshot/DOM inspect。
- 没有 Notebook cell 级编辑。

#### 建议顺序

1. 先把 `fs.read attachment` 在模型侧明确呈现为一等多模态工具能力，并增加缩放/页码/帧选择。
2. 再接 `image.generate` / `image.edit`，输出必须落到 workspace 或 managed artifact store，不能只返回临时 URL。
3. browser harness 先支持 open/snapshot/click/type/screenshot/wait，所有导航继续复用网络 permission 与 SSRF policy。
4. NotebookEdit 作为独立插件实现 cell id/index 的安全编辑，避免用普通文本 patch 破坏 JSON。

### 6.11 P2：上下文与环境控制工具

Codex 暴露 `get_context_remaining`、`new_context_window`、`wait_for_environment`；Claude/Gemini 也有显式 context/compaction 产品面。Agena 已有 compaction 与 prompt budget，但模型没有结构化可见性。

建议只增加高价值的两个：

- `context.status`：剩余预算、压缩状态、最大单结果预算，不暴露敏感内部 prompt。
- `environment.wait`：等待 devcontainer 或 managed runtime readiness。

不要把任意“创建新上下文”直接交给模型，除非 rollout/session lineage、费用和用户可见性已经清楚。

## 7. 推荐的目标架构

### 7.1 Tool Planner

```text
Plugin/MCP/hosted tool sources
  -> canonical ToolDefinition registry
  -> policy projection
       - agent/profile permission
       - model/provider capability
       - runtime/process availability
       - plan/mode
  -> exposure planner
       - direct functions
       - deferred catalog
       - hidden/internal handlers
  -> provider adapter
       - native function protocol
       - prompt envelope fallback
       - hosted Provider tools
```

Tool registry 仍是单一事实源，但 provider-visible surface 不再等于“固定五函数”或“全部工具”。

### 7.2 Skill Runtime

```text
bundled / plugin / user / workspace / remote providers
  -> validate + normalize + provenance + trust
  -> catalog + ranking + context budget
  -> explicit/implicit activation
  -> dependency resolution and approval
  -> typed context injection
  -> runtime-enforced tool/model scope
  -> resource reads with bounded locators
```

### 7.3 MCP Runtime

```text
config/CLI/plugin sources
  -> connection supervisor
       - auth/token refresh
       - timeout/retry/backoff
       - capability negotiation
  -> per-server snapshots
       - tools/resources/templates/prompts/instructions
       - list_changed generations
  -> searchable qualified catalog
  -> exact tool call with schema fingerprint
  -> lossless MCP result -> Agena attachment/payload mapping
```

## 8. 分阶段工程计划

### Phase 0：事实源与回归基线（1 周）

目标：先让计数、能力与真实运行时一致。

- 从 plugin host manifest 生成 machine-readable `agena inspect --json`。
- 输出 plugins、execution tools、gateway functions、provider-native bindings、MCP servers、Skills、Agents。
- 自动生成或校验 `plugins-and-tools-reference.md` 的索引和计数。
- 建立 tool protocol metrics：direct/gateway、validation retry、misroute、help-before-call、平均额外回合。
- 修正 61/62 文档漂移。

验收：CI 在源码注册表变更但文档/manifest 未更新时失败。

### Phase 1：Skill 最小闭环（2–3 周）

- 恢复标准 roots 和 workspace trust。
- 修复 alias resolution。
- 增加 source/scope/trust/hash/diagnostics。
- 实现 typed activation 和 allowed-tools runtime intersection。
- 支持 bounded resource read。
- 加 watcher 和 enable/disable config。
- 新增 `skill-creator`、`doctor`、`verify` 三个 bundled Skills。

验收：项目 `.agena/skills/demo/SKILL.md` 在 trusted workspace 可发现、可激活、资源不可越界、allowed-tools 确实阻止未授权工具。

### Phase 2：MCP 生产化（3–5 周）

- cursor/resource templates/content fidelity。
- OAuth + keyring。
- connection supervisor、timeouts、retry、enable/disable。
- roots/instructions/list_changed。
- MCP searchable index 与 stale fingerprint。
- CLI 管理命令。

验收：用包含分页、OAuth、list_changed、audio/structuredContent 的 fixture 做 e2e；断线重连不重启 session。

### Phase 3：Hybrid Tool Exposure（2–4 周）

- `ToolExposure` 与 per-model planner。
- 第一批核心 direct tools。
- gateway 保留 long-tail/MCP。
- rollout protocol version 与 resume compatibility。
- A/B 对比工具成功率、完成回合数、缓存命中和 token。

验收：支持 native tools 的模型不再为 `fs.read` 先 search/help；prompt-envelope Provider 行为不回归。

### Phase 4：异步任务、Shell Monitor 与持久 Scheduler（3–5 周）

- task handles + list/output/wait/cancel/followup。
- 将 monitor 条件、状态和增量输出并入 `shell.run/list/logs/stop`。
- SQLite scheduler store、misfire/history。
- TUI/Studio 统一任务中心。

验收：同时运行多个子 Agent 和 shell/monitor，父会话可 wait any/all、取消、恢复，重启后 cron 仍存在。

### Phase 5：生态与高级工具（按需求）

- plugin-provided Skills 和 Skill marketplace。
- skill-installer/plugin-creator/run-skill-generator/simplify/batch。
- report.findings。
- image/browser/notebook。
- context.status/environment.wait。

## 9. 不建议直接照搬的设计

1. 不要复制 Claude Code 43 个 tool 名称。很多是托管表面、订阅、远程通知或产品内部能力，不适合本地优先 runtime。
2. 不要为了“清单更长”默认暴露全部 62 个 direct functions。会损失 cache、增加 schema token 并降低模型选择精度。
3. 不要把 Grok 服务端 bundled Skills 的测试名称当成稳定清单。Agena 若引入远程 Skill bundle，必须有签名、版本、cache 和 live inspect。
4. 不要默认加载所有兼容生态 Skill 目录。workspace trust 和显式 compat 开关必须先有。
5. 不要把 Skill `allowed_tools` 继续定义成“不是安全边界”。只要 UI 向用户展示了工具限制，它就必须由 runtime 执行。
6. 不要把 MCP OAuth 简化成把 bearer token 放 JSON。长期凭据必须进入 keyring 或受保护的 credential store。
7. 不要把 command parser 描述成安全隔离；它只用于 effect 推断、审批说明和审计，不能可靠解释任意脚本或二进制。
8. 不要让 plugin、MCP、Provider-native tool 各自形成不同审批和审计语义；应继续复用 Agena 的统一 permission contract。

## 10. 建议的首批具体 issue

以下 issue 可以独立拆分并按依赖排序：

1. `docs/runtime: generate authoritative tool manifest and fix 61/62 drift`
2. `skills: restore trusted user/workspace discovery roots`
3. `skills: resolve aliases and surface validation diagnostics`
4. `skills: add typed activation with enforced allowed-tools intersection`
5. `skills: add bounded package resource reader and provenance`
6. `skills: bundle skill-creator, doctor, and verify`
7. `mcp: add cursor-aware list-all resources and prompts`
8. `mcp: support resource templates and lossless content blocks`
9. `mcp: add OAuth/keyring credential flow and CLI management`
10. `mcp: supervise reconnect/list_changed and build fingerprinted search index`
11. `tools: introduce Direct/Deferred/Hidden exposure planner`
12. `tools: directly expose core fs/shell/interaction tools on supported providers`
13. `shell: integrate monitor conditions into run/list/logs/stop lifecycle`
14. `tasks: split synchronous run into asynchronous task lifecycle tools`
15. `scheduler: implement SQLite JobStore and restart recovery`
16. `fs: add write/replace/read_many/stat with optimistic concurrency`
17. `review: add structured report.findings tool`
18. `ecosystem: add browser/image/notebook/context/environment tool plugins`

## 11. 源码证据索引

### Agena

- 内置插件来源与条件注册：[`crates/agena-runtime-plugins/src/plugins/sources.rs`](../crates/agena-runtime-plugins/src/plugins/sources.rs)
- 内置插件 factory：[`crates/agena-runtime-plugins/src/tool.rs`](../crates/agena-runtime-plugins/src/tool.rs)
- Tool API 与 execution tool 隔离：[`crates/agena-runtime-tools/src/tool/tool_registry.rs`](../crates/agena-runtime-tools/src/tool/tool_registry.rs)
- Tool API plugin：[`crates/agena-runtime-plugins/src/plugins/provided/tool_api.rs`](../crates/agena-runtime-plugins/src/plugins/provided/tool_api.rs)
- Provider tool protocol 纠错：[`crates/agena-runtime-provider/src/provider/registry/completion.rs`](../crates/agena-runtime-provider/src/provider/registry/completion.rs)
- Prompt-envelope transport：[`crates/agena-runtime-provider/src/provider/prompt_tool_transport.rs`](../crates/agena-runtime-provider/src/provider/prompt_tool_transport.rs)
- Skill parser：[`crates/agena-skills/src/skill.rs`](../crates/agena-skills/src/skill.rs)
- 空默认 Skill roots：[`crates/agena-skills/src/discovery.rs`](../crates/agena-skills/src/discovery.rs)
- 内置 Skills：[`crates/agena-skills/src/bundled`](../crates/agena-skills/src/bundled)
- Skills plugin：[`crates/agena-runtime-plugins/src/plugins/provided/skills.rs`](../crates/agena-runtime-plugins/src/plugins/provided/skills.rs)
- Agent profiles：[`crates/agena-runtime-contracts/src/agents/mod.rs`](../crates/agena-runtime-contracts/src/agents/mod.rs)
- MCP client manager：[`crates/agena-mcp-client/src/manager.rs`](../crates/agena-mcp-client/src/manager.rs)
- MCP protocol projection：[`crates/agena-mcp-client/src/protocol.rs`](../crates/agena-mcp-client/src/protocol.rs)
- MCP config：[`crates/agena-runtime-config/src/mcp_config.rs`](../crates/agena-runtime-config/src/mcp_config.rs)
- MCP bridge plugin：[`crates/agena-runtime-plugins/src/plugins/provided/mcp.rs`](../crates/agena-runtime-plugins/src/plugins/provided/mcp.rs)
- Agena MCP server：[`crates/agena-mcp-server/src/lib.rs`](../crates/agena-mcp-server/src/lib.rs)
- MCP server CLI backend：[`crates/agena-cli/src/cli/mod.rs`](../crates/agena-cli/src/cli/mod.rs)
- Shell 明确无 OS sandbox：[`crates/agena-runtime-tools/src/tool/shell.rs`](../crates/agena-runtime-tools/src/tool/shell.rs)
- Shell effect 分析：[`crates/agena-tool/src/shell_analysis.rs`](../crates/agena-tool/src/shell_analysis.rs)
- Monitor 内核：[`crates/agena-runtime-tools/src/monitor.rs`](../crates/agena-runtime-tools/src/monitor.rs)
- Monitor host API：[`crates/agena-runtime/src/runtime/host_client/mod.rs`](../crates/agena-runtime/src/runtime/host_client/mod.rs)
- In-memory scheduler store：[`crates/agena-scheduler/src/store.rs`](../crates/agena-scheduler/src/store.rs)
- Web plugin：[`crates/agena-runtime-plugins/src/web/plugin.rs`](../crates/agena-runtime-plugins/src/web/plugin.rs)
- Provider-native 配置与边界：[`docs/configuration.md`](configuration.md)

### Codex

- 工具 exposure planner：[`codex-rs/core/src/tools/spec_plan.rs`](../../codex/codex-rs/core/src/tools/spec_plan.rs)
- App Server extensions：[`codex-rs/app-server/src/extensions.rs`](../../codex/codex-rs/app-server/src/extensions.rs)
- Skill metadata：[`codex-rs/skills/src/model.rs`](../../codex/codex-rs/skills/src/model.rs)
- Bundled Skill 安装：[`codex-rs/skills/src/lib.rs`](../../codex/codex-rs/skills/src/lib.rs)
- Skill provider sources：[`codex-rs/ext/skills/src/sources.rs`](../../codex/codex-rs/ext/skills/src/sources.rs)
- Skill catalog budget：[`codex-rs/ext/skills/src/render.rs`](../../codex/codex-rs/ext/skills/src/render.rs)
- Skill MCP dependencies：[`codex-rs/core/src/mcp_skill_dependencies.rs`](../../codex/codex-rs/core/src/mcp_skill_dependencies.rs)

### Gemini CLI

- 主工具注册：[`packages/core/src/config/config.ts`](../../gemini-cli/packages/core/src/config/config.ts)
- 工具名称常量：[`packages/core/src/tools/tool-names.ts`](../../gemini-cli/packages/core/src/tools/tool-names.ts)
- Skill manager：[`packages/core/src/skills/skillManager.ts`](../../gemini-cli/packages/core/src/skills/skillManager.ts)
- Skill loader：[`packages/core/src/skills/skillLoader.ts`](../../gemini-cli/packages/core/src/skills/skillLoader.ts)
- Skill activation：[`packages/core/src/tools/activate-skill.ts`](../../gemini-cli/packages/core/src/tools/activate-skill.ts)
- MCP client：[`packages/core/src/tools/mcp-client.ts`](../../gemini-cli/packages/core/src/tools/mcp-client.ts)

### Grok Build

- 编译期 ToolRegistry：[`xai-grok-tools/src/registry/types.rs`](../../grok-build/crates/codegen/xai-grok-tools/src/registry/types.rs)
- 默认/Workspace toolsets：[`xai-grok-agent/src/config.rs`](../../grok-build/crates/codegen/xai-grok-agent/src/config.rs)
- MCP search meta-tool：[`implementations/search_tool/mod.rs`](../../grok-build/crates/codegen/xai-grok-tools/src/implementations/search_tool/mod.rs)
- MCP use meta-tool：[`implementations/use_tool/mod.rs`](../../grok-build/crates/codegen/xai-grok-tools/src/implementations/use_tool/mod.rs)
- Skill discovery：[`implementations/skills/discovery.rs`](../../grok-build/crates/codegen/xai-grok-tools/src/implementations/skills/discovery.rs)
- 动态 Skill reminder：[`reminders/skill_discovery.rs`](../../grok-build/crates/codegen/xai-grok-tools/src/reminders/skill_discovery.rs)
- MCP config：[`xai-grok-config-types/src/mcp.rs`](../../grok-build/crates/codegen/xai-grok-config-types/src/mcp.rs)

## 12. 最终判断

Agena 当前的工具“数量”已经足够，甚至比几个对照产品的默认表面更丰富。短期继续追求工具数量会掩盖真正问题：模型要稳定调用工具，用户要能相信权限边界，Skill 要能真正改变受控执行上下文，MCP 要完整保留第三方协议语义。

本轮建设按以下主线推进；OS 级 sandbox 明确不在当前范围内：

```text
完整 Skill runtime
  + hybrid tool exposure
  + production-grade MCP
  + async task/shell-monitor/persistent scheduler
  + 完整补齐高价值文件、review、browser、image、notebook、context/environment 工具
```

完成这些建设后，Agena 的差异化将非常清楚：它既保留本地优先、多 Provider、插件统一治理的优势，又能达到 Codex/Claude/Gemini/Grok 在工具可靠性、Skill 生态、MCP 互操作和长时任务管理上的成熟度。

## 13. 实施后状态与剩余差距

本节记录本轮在审计基础上实际落地的改造。它不是把 Phase 0–5 全部标成完成：已实现、部分实现、仍缺失三种状态会明确分开。异步任务与 active Skill 的保守持久恢复、request-driven catalog refresh、受控 plugin Skill contribution、path-gated automatic selection 已经落地；OS watcher 与统一主动图像生成 Tool 等仍需后续工程。

### 13.1 当前权威能力快照

[`capability_manifest.rs`](../crates/agena-bundled-plugins/src/capability_manifest.rs) 现在从真正的 plugin manifest、`RegisteredTool` 和 bundled Skill catalog 生成确定性的、可序列化的能力清单。每个工具包含：

- plugin id 与 canonical tool name；
- Direct / Deferred / Hidden / Internal exposure；
- tags、effects、Host capabilities；
- input/output schema SHA-256 和完整 definition identity；
- bundled/conditional 注册属性；
- 每个 Skill 的 alias、allowed-tools、model preference、invocation policy、paths、依赖和 content hash。

默认 `schema-lab` feature 下的真实 source-level 计数是：

| 指标 | 当前值 | 说明 |
| --- | ---: | --- |
| Bundled plugins | 22 | `agena.mcp` 依赖 runtime MCP manager；`agena.schema_lab` 依赖 feature |
| Tool definitions | 100 | 不是“每个会话必然注册 100 个”的承诺 |
| Direct | 53 | 支持 provider function protocol 时直接暴露的高频本地工具 |
| Deferred | 40 | 通过 Tool API 搜索/help/call 的长尾、动态或低频工具 |
| Hidden | 2 | 默认不进入模型 discovery 的 schema-lab 工具 |
| Internal | 5 | `tools_list/search/help/tags/call` gateway handlers |
| Bundled Skills | 14 | 从审计时的 3 个扩展到 14 个 |

当前插件级计数：

| Plugin | Tools | Plugin | Tools |
| --- | ---: | --- | ---: |
| `agena.agent` | 2 | `agena.code` | 2 |
| `agena.context` | 1 | `agena.cron` | 8 |
| `agena.environment` | 1 | `agena.fs` | 9 |
| `agena.interaction` | 2 | `agena.lsp` | 5 |
| `agena.mcp` | 9 | `agena.memory` | 5 |
| `agena.notebook` | 1 | `agena.plan` | 4 |
| `agena.report` | 1 | `agena.schema_lab` | 2 |
| `agena.session` | 2 | `agena.settings` | 7 |
| `agena.shell` | 4 | `agena.skills` | 7 |
| `agena.snapshot` | 2 | `agena.tasks` | 9 |
| `agena.tools` | 5 | `agena.web` | 12 |

这解决了原先 61/62 以及 18-plugin 文档漂移的事实源问题。[`plugins-and-tools-reference.md`](plugins-and-tools-reference.md) 的顶部索引已改为 22/100/14；`bundled_capability_manifest` 的单元测试会读取并校验该索引的总数、exposure 分布和每个 plugin 工具数，避免再次手工漂移。其中旧的逐工具大段 schema 被明确标注为“实施前审计快照”，避免假装它已经自动覆盖所有新增工具。

### 13.2 需求到实现证据

| 需求 | 状态 | 当前实现与边界 |
| --- | --- | --- |
| 不建设 OS 级 sandbox | 符合 | 没有把 sandbox 重新加入计划或代码；shell 仍依赖 permission/effect/ask/audit，文档明确它不是强制隔离 |
| Monitor 必须融入现有 shell | 已实现 | `shell.run.monitor` 支持成功/失败 literal 或 regex、include pattern、quiet period、timeout、persistent、stderr 和 buffer 限制；统一由 `shell.list/logs/stop` 管理，没有新增 Monitor plugin/tool |
| 完整 Skill runtime | 已实现（有刻意边界） | roots、precedence、diagnostics、alias、typed activation、allowed-tools 强制、资源边界、dependency check、exact-hash trust、status/deactivate 已有；`skills.refresh` 与每次 Skill Tool/hook 的重新 discovery 提供 request-driven catalog refresh，回传 fingerprint/generation/diagnostics；config 可按 canonical name disabled 单个 Skill，并只接受 workspace-relative additional roots；plugin manifest 也可声明无路径扫描的贡献 Skill。非 bundled/插件贡献内容都经 exact-hash trust；active activation 写入 session-private storage，并只在当前发现的同名 Skill content hash 完全一致时 lazy restore；全限定 `provider/model` 的 model preference 已真正写入下一回合 session selection。`prompt.submit` 会在 `allow-implicit-invocation + paths` 的双 opt-in、path glob、exact-hash trust、dependency、candidate 和 instruction budget 全部满足时确定性激活一项，并不会隐式换模型或持久写入 activation。`notify` 的平台原生 watcher（FSEvents/inotify/Windows backend）只递增 catalog invalidation generation，下一次正常 Tool/hook request boundary 才重新 discovery；它不会读取/注入 body、trust、activate 或切换模型。已有 root 递归监听，不存在 root 则仅 non-recursive 监听最近存在父目录，避免意外递归监听整个 workspace |
| Hybrid tool exposure | 已实现 | Provider Protocol 使用 5 gateway + Direct tools；Prompt Envelope 保持 5 gateway；Deferred/Hidden/Internal 分类生效；provider 名冲突不会再静默丢工具。每个 `provider/model` route 还可用 `agena_tools.direct` 的 include/exclude、max_tools、max_schema_tokens 收缩 Direct surface，超出的工具仍经 gateway 可用 |
| MCP fidelity | 已实现 | pagination、resource templates、audio/image/resource link/embedded resource/unknown block、annotations/meta、structuredContent、input/output schema 均保留 |
| MCP lifecycle | 核心闭环已实现 | timeout、失败 spec 留存、redacted status、manual reconnect、tools.search、workspace roots、roots/list_changed、server instructions、tools/resources/prompts generations、tool-list 自动刷新、可配置 Weak-reference reconnect supervisor 与指数退避已实现；`agena mcp status/list/get/add/remove/enable/disable/reconnect/login/logout` CLI、keyring-first bearer 与标准 OAuth 已实现。OAuth 使用 protected-resource/authorization-server discovery、S256 PKCE、dynamic registration、RFC 9207 issuer 校验、keyring persistence 和自动 refresh；`mcp status`/`servers.status` 还会只读投影 auth mode、credential missing/configured/unreadable、expiry 和 refresh availability，绝不触发 refresh 或返回凭据。`mcp logout --oauth --revoke --url` 是显式的、no-redirect 的 RFC 7009 远端撤销，成功后才删除本地 record；status 还对双 keyring record 返回不含秘密的 bearer↔OAuth migration advisory，永不自动混用或迁移。server-level include/exclude 及 annotation 规范化 high-risk permission check 已强制执行 |
| Async Agent | 核心生命周期、保守 restart recovery 与 usage budget 已实现 | `tasks.create/list/get/output/cancel/message/followup/wait`；真实 child session cancellation、steer channel、cursor transcript、Notify wait；每个 handle 写入 session-private plugin storage，terminal state 跨 plugin reconstruction 恢复，未确认完成的 handle 以 `interrupted` 安全恢复（不盲重放 prompt）；non-recursive child policy、每 parent 4 个 active task admission boundary，以及 parent session end 默认 attached/cancel 策略已实现。`max_tokens` 和整数 `max_cost_microusd` 在 child session 每个新模型回合前强制检查，并投影 terminal budget_exceeded 状态 |
| Persistent scheduler | durable delivery 闭环已实现，集群级边界仍明确 | SQLite `JobStore`、runtime 组合、重启重建；强引用环已用 Weak 修复；`cron.update/pause/resume/history`、terminal job audit retention 和每 job 50 条持久 history 已有；`skip/run_once_now/reschedule` misfire、bounded exponential retry、pre-sink durable claim、stable delivery key 与 session message persisted dedup 已实现。当前 store 是单 runtime process 的 SQLite usage，不声称多 scheduler process 的分布式 exactly-once |
| 文件工具 | 已扩展 | `write/replace/read_many/stat/view_image`，revision/hash 与资源上限；普通 write/replace 仍可统一升级为同目录原子 rename |
| 结构化 review | 已实现 | `report.findings` 支持 severity、位置、置信度、code 和结构化 severity counts |
| Notebook | 已实现 | `notebook.edit_cell` 支持 replace/insert/delete、cell type、revision SHA-256、output 清理和原子写 |
| Context | 已实现 | `context.status` 暴露剩余预算、usage ratio、窗口、compaction 状态，不泄漏 system prompt |
| Environment | 已实现 | `environment.wait` 支持 path/TCP/HTTP readiness、状态/body 条件、超时和 permission 检查 |
| 交互浏览器 | 核心 lifecycle、普通 HTTP redirect preflight 与下载已实现 | `agena.web` 增加 open/list/close/snapshot/click/type/wait/screenshot/download，复用 Chromium/CDP 和网络/路径权限；target cleanup 已可由 close 完成，snapshot `ref` 可直接供 click/type 使用，type 调用 native prototype setter + React tracker reset + input/change/Enter event。open/download 在创建 target 前以 no-follow HEAD 遍历最多 10 个普通 HTTP redirect hops，并逐 hop 检查 host/DNS permission；download 使用 managed Chromium profile，写入 workspace artifact，稳定后回传 local attachment。JS/cookie/method-dependent redirect 仍缺 Fetch-domain 的逐请求拦截 |
| 图像能力 | Provider-native 生成闭环已加固；统一主动 API 仍待 | 已有 `fs.view_image`、`imagegen` Skill、OpenAI Responses provider-native image generation；base64 image result 只会落入 process-managed artifact，解码前后均限制 50 MiB，并回填 size 与 SHA-256 到统一 attachment。尚无统一可执行的 `agena.image.generate/edit` Host API，因此没有加入空壳工具 |
| 权威 manifest | 已实现 | 默认 catalog 测试验证 22 plugins、100 tools、14 Skills，并验证 exposure 总数守恒；`agena inspect --json` 直接暴露该确定性清单；reference overview drift test 同时验证文档的总数、exposure 与 plugin 行 |

### 13.3 Skill：从 Prompt 模板变成受控激活

当前默认发现顺序覆盖：

```text
$AGENA_HOME/skills 或 $HOME/agena/skills
  -> $HOME/.agents/skills
  -> workspace .agena/skills
  -> workspace .agents/skills
```

command roots 同样覆盖 Agena home、用户 `.agents`、workspace `.agena/.agents`，后出现的 scope 覆盖前面的同名 Skill。坏目录、坏 frontmatter 和读取错误进入结构化 diagnostics，而不是只写 warning。

frontmatter 新增或真正执行的字段包括：`allowed-tools`、`user-invocable`、`allow-implicit-invocation`、`paths`、`dependencies.tools/mcp/environment`。`skills.run` 会：

1. 解析正式名称或 alias；
2. 对非 bundled 内容展示 source path、SHA-256、allowed tools、resource paths 和依赖，并确认 exact content hash；
3. 检查 Host tool registry、MCP server registry 和环境变量；
4. 建立 session activation；
5. 通过 `chat.system.transform` 注入 typed context；
6. 通过 `tool.execute.before` 强制 allowed-tools，而不再只是提示模型；
7. 保留 `agena.skills.*` 与 `agena.tools.*`，以便读取资源、观察状态和安全退出；
8. 在 session end 清理内存和 session-private persisted activation。

`agena.skills` 从 3 个工具扩展为 7 个：`list/get/run/read_resource/refresh/status/deactivate`。`skills.refresh` 将当前扫描结果与 catalog fingerprint 比较，返回 monotonic generation、changed、技能/command 数和 diagnostics；在任意 Skill Tool、system injection 或 allowed-tools hook 执行时也会重新扫描，因此文件变化不会继续使用常驻旧 catalog。与此同时，`watcher.enabled`（默认 `true`）启用 `notify::recommended_watcher` 的平台原生通知后端：已有 root 递归监听；尚不存在的 Skill/command root 只 non-recursive 监听其最近存在的父路径，避免为了等待 `.agena/skills` 创建而递归监控整个 workspace。callback 只增加 atomic invalidation generation；下一次正常 request boundary 才按既有安全路径完整 discovery。它不会后台读取 Skill body、注入 prompt、授信、激活或更改 model，因此它是低副作用的 catalog 失效信号，而不是隐式执行器。`skills.refresh` / `skills.status` 投影 watcher enabled、watched path 数与 generation。资源读取拒绝绝对路径、`..` 和 symlink escape，并限制大小与 UTF-8。

Bundled Skills 当前为：

```text
batch, debug, doctor, imagegen, init, plugin_creator, review,
run, run_skill_generator, security_review, simplify,
skill_creator, skill_installer, verify
```

`model` 现在要求采用全限定 `provider/model`，并由 Host API 验证、持久写入 session selection；不能在 active generation 中切换，以避免改变 in-flight completion。它目前会在 deactivation 后保留为 session 的已选模型，直到用户或后续选择覆盖。

非 bundled Skill 的批准不再只存在于进程内：以 exact content SHA-256 作为 key，写入 workspace-private plugin storage 的 `skills.trust.v1` namespace；同一 workspace 在 plugin reconstruction/进程重启后对同一哈希不会重复询问，哈希改变则一定重新询问。storage 读取或写入不可用时只回退到本进程内存 trust，既不会扩大信任范围，也不会阻塞可用的 Skill 流程。`exact_hash_trust_survives_plugin_reconstruction` 覆盖了该重建路径。

active activation 也不再只是进程内 map：`skills.run` 将完整 typed activation 写入当前 session-private `skills.active.v1/active` record；`skills.status`、`chat.system.transform` 和 `tool.execute.before` 会在内存缺失时 lazy restore。恢复前会重新 discovery，并要求 canonical name 仍存在且当前 `content_hash` 与 persisted record 完全相同；Skill 删除、改写或 persisted JSON 损坏时，记录会被删除，旧 prompt 与旧 allowed-tools 不会重新进入会话。`active_skill_is_restored_after_reconstruction_and_discarded_when_changed` 覆盖了重建恢复、runtime allowlist enforcement 和内容变更后的安全失效。这个语义是单个 runtime 的 session storage restart/reconstruction restore，不宣称跨 workspace、跨 session 或多进程共享 activation。

`plugins.list.<id>.config` 中的 `agena.skills` 配置现在支持 `disabled`、`additional_roots`、`additional_command_roots`、`watcher.{enabled}` 以及 `implicit_invocation.{enabled,max_candidates,max_instruction_chars}`：即使关闭 watcher，request-driven refresh 仍保留；禁用按 canonical name 从 list/get/run 与 persisted restore 中一并移除；额外 root 必须非空、workspace-relative 且不得包含 `..`，其内容仍要逐 hash 获得激活确认。`prompt.submit` 的隐式选择不是 LLM 猜测：仅处理同时设置 `allow-implicit-invocation: true` 和非空 workspace-relative `paths` glob 的 Skill；prompt 中的 path token 必须匹配 glob，非 bundled 内容还必须已对完全相同 hash 取得信任、依赖必须满足。候选按匹配路径数降序和 canonical name 升序稳定排序，最多评估 `max_candidates`（1–128）个；超过 `max_instruction_chars`（256–65536，字符而非伪精确 tokenizer）的 instructions 不会自动注入。每个 prompt 至多激活一项，写入当前 runtime 的 session map、不切换模型、不持久化 activation，显式 `skills.run` 仍是需确认且可持久恢复的完整工作流。Plugin manifest 新增声明式 `skills` vector；贡献的 instructions/aliases/allowed-tools/model/dependencies 与 plugin id/version 一起进入 catalog，但不会让插件提供任意 filesystem root，也不能读取 package resource，且一律按 non-bundled content 走 trust confirmation。manifest validator 拒绝空/重复 lookup name、过长 instructions、blank dependency/allowed-tool/path 与不规范 model。watcher 与 request-driven discovery 共同保证 catalog 不会在活动边界长期停留在陈旧状态，同时保留 trust/activation 的显式安全边界。

### 13.4 Hybrid Tool Exposure：保留 gateway，但不再强迫高频工具绕路

当前分类规则：

- Direct：多数 `agena.*` 高频核心工具；
- Deferred：MCP、memory、settings、skills、agent、session、snapshot/repo、`web.crawl`、除 `cron.wakeup` 外的 cron 工具；
- Hidden：`schema_lab`；
- Internal：五个 `agena.tools` gateway。

Provider Protocol 会收到稳定五 gateway 加 Direct bindings；Prompt Envelope 仍只有五 gateway，以保持不支持 native function calling 的 Provider 兼容性。持久 transcript 新增 `provider_function_name`，所以 Direct tool call/result 可以按原 provider 名正确回放，而内部执行仍映射回 canonical execution-tool name。

本轮又修复了 sanitized provider function name collision：不再使用 `dedup_by` 静默丢弃。新算法按“可读短名 → sanitized full canonical name → 截断前缀 + 稳定 SHA-256 后缀”分配，并把五个 gateway 名称作为保留名；测试覆盖 compact collision、gateway collision 和超过 64 字符的名字。

`providers.<id>.adapters.<adapter>.models.<model>.agena_tools.direct` 现在是 Direct surface 的 route-level policy：`include`/`exclude` 用 canonical name（如 `agena.fs.read`）上的简单 `*` wildcard，exclude 优先；`max_tools` 可把 Direct declaration 数压为零而保留全部五个 gateway，`max_schema_tokens` 对序列化后的 Direct definition 以 `ceil(chars / 4)` 的确定性、provider-neutral 估算限制 schema。候选先按 canonical name 排序，再分配安全 provider function name 和累积预算，因此同一 route 的 Prompt cache shape 稳定。`prompt_envelope`/`disabled` route 明确拒绝该配置，防止无效的“看似生效”设置。配置解析、route registry、Session planner 和 ToolExecutor 已贯通；被过滤/预算截断的工具没有失去执行权限，仍经 gateway 搜索和调用。

### 13.5 MCP：协议、生命周期与标准 OAuth 已形成闭环，治理仍待细化

已完成的协议层：

- resources/prompts cursor pagination；
- resource templates；
- ToolDescriptor 的 title、input/output schema、annotations、execution、icons、meta；
- text/image/audio/embedded resource/resource link/unknown raw JSON；
- CallToolResult 的 `structuredContent`、`_meta`、`isError`；
- MCP server 反向投影同样保留上述内容。

已完成的连接与发现层：

- connect timeout 默认 20 秒、request timeout 默认 60 秒；
- 配置 spec 与活连接分离，失败后仍可在 status 中观察并手动 reconnect；
- `tools.search` 返回 schema、annotations 和稳定 fingerprint；
- `servers.status` 返回 connected/tool count/network target/last error；
- `servers.reconnect` 手动恢复；
- 自定义 rmcp client handler 公布 workspace file root，支持 `roots/list` 与 `roots/list_changed`；
- 保存 initialization `instructions`，以有界文本注入 MCP 工具 help；
- `notifications/tools/list_changed` 异步刷新工具缓存；
- resources/prompts list-changed 与 resource-updated 分别递增 generation；
- refresh error 与各 generation 在 `servers.status` 中可见。
- runtime config 的 `reconnect` policy 默认启用：initial/max delay/poll interval 可配；supervisor 对仍在配置中的 disconnected server 使用 capped exponential backoff，并只持有 `Weak<McpConnectionManager>`，不会阻止 runtime snapshot 回收；
- `servers.status` 返回 reconnect supervisor 是否运行。
- MCP status 现投影 connected/tool count/network target/last error、各 list generation、refresh error 和 supervisor 状态，排除 bearer/header/credential 内容；CLI `agena mcp status/list/get` 使用同一投影。MCP initialization instructions 只在 tool-definition help 以每 server 2,000 字符上限提供；`servers.status` 不重复返回它们。
- CLI 已提供 `mcp add/remove/enable/disable/reconnect`：通过既有 schema-validated settings service 更新 global 或 workspace 的 static `agena.mcp` plugin record，支持 dry-run 和 reload；HTTP add 拒绝 URL 内 credential 和 Authorization header。
- `mcp login/logout` 默认写入/删除 `agena.mcp` 系统 keyring service；keyring account key 是 server id 的 SHA-256 派生值，不暴露配置 server name。兼容文件 store 只有显式 `--store file` 或 config opt-in fallback 时才使用。
- HTTP `auth: { kind: "oauth", scopes: [...] }` 只保存 scopes，不保存 client id、access token 或 refresh token；`rmcp` 的 protected-resource/authorization-server discovery、S256 PKCE、dynamic registration 与 refresh 由 `McpOAuthLoginSession` / `AuthClient` 执行。
- `agena mcp login <server> --browser --url <endpoint> [--scope <scope>]` 建立 loopback callback；所有 OAuth credentials 使用独立的 `mcp-oauth-v1-<sha256(server)>` keyring key。callback 继续验证 CSRF state 和 RFC 9207 `iss`，`mcp logout <server> --oauth` 只清除 OAuth 记录，不影响手动 bearer。

OAuth health 已形成可观测闭环：`KeyringOAuthCredentialStore::health()` 只读取该 server 的独立 OAuth keyring record，产生三态 `missing/configured/unreadable`，并在可获得 `expires_in + token_received_at` 时投影 `valid/expiring/expired/unknown` 与 refresh-token 是否存在。30 秒的 `expiring` buffer 与 rmcp 的实际 refresh 触发线一致。`McpConnectionManager::statuses()` 将其只附加给 `auth.kind=oauth` 的 server；runtime status / CLI `agena mcp status`、以及模型可用的 `mcp.servers.status` 都仅输出字符串和布尔值。后者还给出 `run_mcp_login`、`clear_or_reauthenticate`、`reconnect_or_reauthenticate`、`reauthenticate_before_expiry` 或 `none` 的建议。

这不是 token introspection，也不是主动刷新：状态查询不接触 authorization server，不能因为一次健康检查产生网络副作用。它绝不返回或记录 client id、scope、access token、refresh token、keyring account key、header，甚至不返回原始 keyring/JSON 读取错误。存储不可读或记录损坏统一折叠为 `unreadable`；未登录折叠为 `missing`。`oauth_health_is_redacted_and_classifies_missing_unreadable_and_known_expiry` 覆盖 missing、malformed、valid、expiring、expired 和秘密不进入 Debug 投影。

可选 revocation 与 migration 也在本轮完成：`mcp logout <server> --oauth --revoke --url <MCP_ENDPOINT>` 明确要求 OAuth、明确 resource endpoint，重新执行标准 metadata discovery，只在 authorization server 公布 RFC 7009 `revocation_endpoint` 时发送 form-encoded request。请求禁止 redirect，响应 body 不写入日志或错误；只有 2xx 后才删除独立 OAuth keyring record。正常 `mcp logout --oauth` 保持本地-only、无网络副作用。若 remote revoke 或 local delete 失败，record 仍保留以供重试；不会从无关 config layer 猜测 authority。

manual bearer 和 OAuth 的 keyring key 从设计上独立。对 `auth: oauth` 但仍存 bearer record，或 `auth: bearer_from_store` 但仍存 OAuth record，`mcp status`、runtime status 与 `mcp.servers.status` 返回 redacted `credential_migration { state, recommendation }`；它不含 token、client ID、scope、keyring account 或底层错误。建议依次显式切换 config、验证连接、再删除旧 record；connection code 绝不会同时读取两类 credential、自动搬迁或自动清理。

本轮已补 server 级 `tools.include` / `tools.exclude`（CLI 对应
`--include-tool` / `--exclude-tool`，支持 `*`）。策略在 manager 中统一作用于
初始 discovery、list-changed refresh、search、manual refresh 和 invocation；exclude
优先，且拒绝发生在 MCP request 发出之前，不能被直接构造 `tools.call` 输入绕过。

MCP annotations 现有两层投影：`tools.search` 保留原始 annotations 并输出规范化
`risk_hint`；runtime snapshot 将同一 `McpConnectionManager` 以通用
`ExecutionPermissionInspector` port 注入 ToolExecutor。对 cached descriptor 的
`destructiveHint=true` 或 `openWorldHint=true`，inspector 会追加名为
`agena.mcp.high_risk`、qualifier 为 `<server>/<tool>` 的独立 Ask check。它进入既有
persisted rules、permission request、related/requested actions 与 trace，且取值不来自模型
输入中的 risk 字段。`readOnlyHint` 仅影响展示风险，绝不移除 bridge 的 `mutating` 检查。

### 13.6 长时执行：Tasks、Shell Monitor、Scheduler

`agena.tasks` 当前 9 个工具：

```text
run, create, list, get, output, cancel, message, followup, wait
```

`cancel` 会定位 `(parent_session_id, task_id)` 对应的真实 child session 并调用 execution cancellation；`message` 进入 child steer channel；`followup` 复用相同 task id 和持久 child session；`output` 使用持久 message id cursor；`wait` 使用 Notify，不做固定 20ms polling。子 Agent 的非递归限制覆盖 create/followup/message 和原有 run aliases。

`agena.tasks` 会把每个 asynchronous task handle（parent/task id、profile、description、original prompt、selection、timeout、`max_tokens`、`max_cost_microusd`、status、terminal response/error/budget_exceeded）写入 parent session 的 private plugin-storage namespace `async_tasks.v1`。因此 plugin/runtime reconstruction 后，`list/get/output/cancel/message/followup/wait` 先 hydrate handle；terminal result 直接恢复。若重启时记录仍为 running/cancelling，它会明确转成 `interrupted`，保留同一 child session 与 transcript，但**绝不自动重送原 prompt**：宕机时 child session 可能已经持久化该 prompt 而 task 仅未收到 acknowledgement，盲重放会制造重复副作用。模型可先 `tasks.output`，再通过明确的 `tasks.followup` 恢复；所有 handle 还按 parent session 做访问隔离，并有每 parent session 最多 4 个 nonterminal async task 的 admission boundary（global provider capacity 仍由 runtime/provider 管理）。parent session end 采用明确的 attached 默认：对每个仍在运行的 child 发起 cancellation，而不是留下无人拥有的 provider work；child transcript 继续保留，正常 completion path 会写最终 task record。

任务预算不只是 task plugin 的后处理比较。`max_tokens` 覆盖 Provider 上报的 input/output/reasoning/cache-read/cache-write 全部 token 类别；`max_cost_microusd` 使用整数 USD 微单位（例如 `250000` = `$0.25`），避免浮点预算边界。两者随 `RunSubtaskRequest → SessionSubtaskRequest` 进入 child 的 `run_until_stable` 生命周期，在每一个新的模型回合前用 child-run 的 baseline usage 检查；剩余 token 还会收紧该回合的 `max_output_tokens`。达到或超过任何 budget 会阻止下一个 model turn，返回 `budget_exceeded=true` 与实际 usage。单一已在飞的 Provider request 只能在 usage 返回后判断，因而可能有一次可审计超量；这属于 Provider usage reporting 的物理边界，不能伪称为硬中断。未计价/未上报 cost 的 route 仍会被 token budget 限制，但 cost ceiling 没有虚构精度。

Monitor 没有产生独立 `agena.monitor`。`shell.run.monitor` 可声明：success/failure literal 或 regex、include pattern、quiet period、timeout、persistent、buffered line 上限和 stderr capture。所有 monitored process 都返回统一 `process_id`，继续由 `shell.list/logs/stop` 管理，并记录 `success_pattern/failure_pattern/quiet_period/timeout/explicit_stop/natural_exit/runtime_failure` completion reason。

Scheduler 已有 SQLite 表 `agena_scheduler_jobs` 和 `SqliteJobStore`，runtime 不再使用纯内存 store。`Scheduler → SessionSink → SessionManager → ToolExecutor → Scheduler` 的强引用环已通过 `Weak<SessionManager>` 和后台 loop 的 `Weak<Scheduler>` 打断，Drop 会 notify 并 abort handle。

Scheduler 现在保留 terminal once/expired jobs 供审计，`cron.update/pause/resume/history` 使模型能完整管理其生命周期；每个 job 的 history 最多保留 50 条并随 job JSON 持久化。`cron.create`/`cron.update` 可以配置 `misfire_policy`（`skip`、`run_once_now`、`reschedule`）与 bounded exponential `retry_policy`（默认总共 3 次，15s → 30s，最长 300s）。

每次到期处理先把 `pending_delivery { delivery_key, scheduled_for, attempt, claimed_at }` 持久化，再调用 SessionSink；失败会留下相同 key 并记录失败 attempt，完成或耗尽 retries 才 advance schedule。该 key 被写入 user message metadata；sink 在重放前查询目标 session 的持久 message projection，已存在则以 `skipped` 结束而不重复 enqueue。因此它在单 runtime + SQLite restart 边界提供“durable claim + session-level dedup”的 at-least-once delivery 语义，不把无法证明的分布式 exactly-once 写成承诺。misfire/retry/claim/key 的单元测试已覆盖。剩余长时执行缺口是跨 job history 的集中 retention/export，而不是 task registry restart、parent end、depth/concurrency 或 task token/cost budget。

### 13.7 新增高价值工具的安全边界

#### 文件与图像查看

`agena.fs` 现在有 9 个工具。`write/replace` 支持 expected revision，`read_many` 有总预算，`stat` 返回 SHA-256；`view_image` 支持 low/high/original、50 MiB 上限、MIME/extension 校验、SHA-256 和 local-path image attachment。

Provider-native image generation 也不是 transient URL：OpenAI Responses 的 `image_generation_call` base64 result 会先由 session processor 解析为 image media，再复制到每 workspace/session 的 process-managed generated-image artifact。解码前按 base64 长度做 50 MiB lower-bound 拒绝，解码后再次检查实际长度；写入后计算 SHA-256，并将 local path、size、hash 投影回 `OperationBlock::Media` / `AttachmentItem`。这使后续 UI、export 和 `fs.view_image` 面对的是同一受管附件语义。它不是跨 Provider 的主动 `image.generate/edit` API，不能因此误写成后者已经完成。

#### Notebook

`notebook.edit_cell` 支持 replace/insert_before/insert_after/delete，code/markdown/raw，expected SHA-256，code output 保留或清空。写入采用同目录 temporary file、`sync_all`、rename；`.ipynb` 和 cells array 会先校验。

#### Context 与 Environment

`context.status` 的 Host API 链路贯通 plugin SDK、stdio transport、scoped host、runtime host client 和 SessionManager，返回 measured/projected/current/remaining tokens、limit、ratio、reserve、context window、compaction 前后与失败次数，不返回 prompt 内容。

`environment.wait` 支持 path、TCP、HTTP、expected status、body contains、最高 10 分钟 timeout；拒绝 URL credentials，只允许 HTTP(S)，HTTP body 上限 1 MiB，并对 path、URL 和 DNS 解析地址执行 permission check。

#### 浏览器

浏览器工具融入现有 `agena.web`：`browser_open/list/close/snapshot/click/type/wait/screenshot/download`。实现复用本地 Chromium launcher，使用 CDP WebSocket 创建/附着 target；`browser_list` 只返回 page target 的 id/title/URL/attached，`browser_close` 通过 CDP `Target.closeTarget` 回收指定 target；snapshot 返回 URL/title/readyState/可见正文/最多 200 个交互元素；screenshot 写到 `.agena/artifacts/browser/<session>.png` 并返回 image attachment。snapshot 的 `elements[].ref` 与同一交互元素枚举次序相同，可在 `browser_click`/`browser_type` 的 `ref` 参数中直接使用（selector/ref 必须二选一）；DOM 变化后模型应重新 snapshot。`browser_type` 使用 HTMLInput/HTMLTextArea/HTMLSelect 原生 property setter、将 React `_valueTracker` 回退到旧值，再 dispatch input/change 和可选 keydown/keypress/keyup Enter，避免简单 `el.value=` 被 React controlled component 忽略。CDP fixture 覆盖了 native setter、tracker 和 event 路径。

`browser_open` 与 `browser_download` 会在导航前发送 no-follow HEAD，逐个解析至多 10 个普通 HTTP `Location` hop，并对每个 URL 执行既有 host/DNS permission check；非 HTTP(S) redirect 直接拒绝。`browser_download` 用当前受管 Chromium page 的 profile 发起导航，临时把下载行为锁定到唯一 `.agena/artifacts/browser/downloads/<uuid>` 目录，序列化这段 browser-global CDP 设置，忽略 `.crdownload`，稳定两次轮询后返回 local attachment，并限制 100 MiB。它不允许模型指定任意写入路径。`browser_click`、`browser_type` 与 `browser_wait` 现在也会在动作完成后获取 committed page URL/snapshot 并再次运行相同的 host + DNS permission 检查，不能在交互后把未经审计的最终 URL 直接投影给模型。仍明确缺失的是 cookie/JavaScript/method-dependent 的**请求发出前** Fetch-domain 拦截：post-navigation recheck 不是“请求发出前”的严格 sandbox，不能如此宣传。

### 13.8 有意没有实现的“伪能力”

以下两点是刻意控制范围，不是遗漏：

1. **OS 级 sandbox**：当前不做。permission/effect/approval/audit 必须被准确描述，不能包装成隔离。
2. **空壳 image.generate/edit**：当前 Provider-native image generation 已能落 process-managed artifact，且统一 attachment 回填 local path、size 与 SHA-256，解码上限为 50 MiB；`imagegen` Skill 和 `fs.view_image` 已有。但 runtime 还没有跨 Provider 的直接 image generation Host API。只有在活动 route 真支持、输出能持久保存、edit 输入有统一 attachment 语义时才条件注册 `agena.image.*`；在此之前不增加“看起来存在但不能执行”的 Tool。

### 13.9 下一阶段优先级（按剩余风险排序）

1. 跨 job history 的集中 retention/export；task registry restart recovery、non-recursive policy、per-parent concurrency boundary、parent end attached/cancel 和 task token/cost budget 已完成。
2. 统一 Provider image generation/edit Host API 与条件注册工具；现有 provider-native artifact 已有大小/hash/attachment 边界，不能以此伪称主动 Tool API。
3. Browser 的 Fetch-domain/proxy 逐请求 redirect preflight，覆盖 cookie/JS/method-dependent navigation；当前普通 HTTP redirect、下载 artifact 与交互后的 final-URL permission recheck 已处理。
4. 给 capability manifest 增加 CI 文档快照/漂移校验。`agena inspect --json` 已直接输出确定性的编译期 manifest；当前已有守恒测试，但还没有将 Markdown reference 的生成或 hash snapshot 接入 CI。

### 13.10 验证证据

本轮 focused tests 覆盖：Skill discovery/activation/trust/dependency，以及 active activation 的 plugin reconstruction/hash-validated stale invalidation；MCP content fidelity/timeout/status/reconnect/roots；真实 shell monitor process；SQLite scheduler 与 Weak lifecycle；async task manifest/Notify；Notebook revision/atomic edit；Context/Environment manifest；Hybrid provider parsing/replay/validation；真实 Chrome/CDP open/evaluate/screenshot；capability manifest 计数守恒。

新增的 manifest 测试输出并验证：

```json
{
  "plugins": 22,
  "tools": 100,
  "direct_tools": 53,
  "deferred_tools": 40,
  "hidden_tools": 2,
  "internal_tools": 5,
  "bundled_skills": 14
}
```

历史验证记录：在 persisted exact-hash Skill trust 落地后，曾执行过 `cargo test --workspace --locked` 和 `cargo clippy --workspace --all-targets --locked`，均以 exit code 0 结束。Clippy 当时仍报告仓库既有的 unused re-export/dead-code，以及若干 `large_enum_variant`、`type_complexity`、`items_after_test_module`、`let_and_return` 警告；“exit code 0”不等于“零 warning”。

本次报告验收又在当前 MCP keyring/CLI 改动后实际执行了以下 focused 验证，均通过：

```text
cargo test -p agena-runtime-plugins bundled_manifest_is_complete_and_has_consistent_counts --locked -- --nocapture
cargo test -p agena-mcp-client --lib --locked -- --nocapture
cargo test -p agena-cli --lib --locked -- --nocapture
```

结果为 manifest 计数守恒、MCP 8/8 单元测试通过（包括 keyring key 派生、pagination、structured content、roots、reconnect）、CLI 9/9 单元测试通过（包括安全的 `mcp add`、keyring 默认凭据和静态 plugin-record 更新）。

本次交付已再对**当前**工作树执行完整验证：

```text
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked
```

两者均以 exit code 0 结束。Clippy 仍有既有的 unused import/re-export/dead-code，以及 `large_enum_variant`、`type_complexity`、`items_after_test_module`、`let_and_return` warnings；它们没有被掩盖或误写成“零 warning”，但不构成当前测试或 lint 的失败。

随后加入的 MCP OAuth implementation 也已完成当前工作树验证：`cargo test --workspace --offline --quiet` 与 `cargo clippy --workspace --all-targets --offline --quiet` 均以 exit code 0 结束；OAuth-focused 验证还覆盖了独立 hashed keyring record 的 save/load/clear、CLI `--auth oauth` / `--scope` parser、以及配置不会序列化 client id、access token 或 refresh token。真实 provider 的授权服务器仍应在接入时做一次手动 smoke test，以覆盖其自身的 metadata/registration policy。

本次 Scheduler durable-delivery 补强后，额外实际执行并通过：

```text
CARGO_INCREMENTAL=0 cargo test -p agena-scheduler -p agena-runtime-tools -p agena-runtime-contracts --lib --offline -- --nocapture
CARGO_INCREMENTAL=0 cargo check -p agena-runtime -p agena-application -p agena-tui-app --offline
CARGO_INCREMENTAL=0 cargo test -p agena-runtime-plugins --lib --offline -- --nocapture
```

前者覆盖 `skip/run_once_now` misfire、stable delivery key 的 bounded retry、SQLite store 与 scheduler Weak lifecycle；后者确认 delivery key 从 Scheduler 到 SessionSink、`SessionUserMessageRequest`、持久 `MessageMetadata`、Runtime presentation 和 API resource 的整个类型链可编译。编译输出仍含仓库既有 unused import/re-export/dead-code warning，不能误解成零 warning。

最后一个任务插件测试运行覆盖 storage-capability manifest、非递归任务 contract、Notify waiter 以及全部 22 plugins / 100 tools / 14 bundled Skills 的能力清单守恒。Skill 的 `active_skill_is_restored_after_reconstruction_and_discarded_when_changed` 还验证 session-private activation 的 reconstruction restore、allowlist 仍然在 tool hook 强制执行，以及修改 `SKILL.md` 后旧 record 被拒绝并清理；新增的 config/refresh 测试覆盖 disabled policy、workspace-root escape 拒绝和 deterministic catalog generation，plugin manifest 测试覆盖声明式 contribution 的 source/trust/resource boundary。跨真实 runtime restart 的最终端到端 smoke test 仍应在有可用 provider route 的集成环境执行；本地 unit test 覆盖的是不自动 replay、持久 handle/activation protocol 和类型/注册边界。

Capability manifest 现在还有零 runtime/database 依赖的 CLI consumer：

```text
agena inspect --json
```

它输出 `schema_version`、snapshot date、22 plugins/100 tools/14 bundled skills 计数，以及每个工具的 canonical name、exposure、tags、effects、Host capabilities、schema hash 和 definition identity，供 CI 将文档快照与真实注册表进行比较。该 CLI parser contract 已纳入 `agena-cli` 单元测试；`bundled_capability_manifest` 还会读取 reference overview，验证总数、exposure 与 plugin 行；仍待增加“生成完整 reference / diff reference”这一 CI job。

本次最终复核还实际执行并通过：

```text
CARGO_INCREMENTAL=0 cargo check -p agena-plugin-host -p agena-runtime-plugins -p agena-runtime -p agena-runtime-session --offline
CARGO_INCREMENTAL=0 cargo test -p agena-runtime-plugins -p agena-plugin-host --lib --offline -- --nocapture
target/debug/agena inspect --json
```

其中第二条命令通过 `agena-plugin-host` 的 11 项测试和 `agena-runtime-plugins` 的 51 项测试，后者包含 active Skill 的 reconstruction/hash invalidation、refresh/config、plugin contribution 和 reference-drift 回归；最后一条由 JSON parser 校验了计数、嵌套 plugin tool 总数和 14 个 bundled Skill。上述 compile/test 输出仍有仓库既有 unused import/re-export/dead-code warnings，不能将通过理解为零 warning。

本轮的 MCP credential-governance 和 Skill implicit-selection 补强后，实际执行并通过：

```text
CARGO_INCREMENTAL=0 cargo test \
  -p agena-mcp-client -p agena-runtime-plugins -p agena-runtime -p agena-cli \
  --lib --offline -- --nocapture
```

结果为 `agena-mcp-client` 12/12、`agena-runtime-plugins` 56/56、`agena-runtime` 54/54、`agena-cli` 11/11。新增测试覆盖 OAuth/bearer 双 record 的 redacted migration advisory，以及未 trust 的 filesystem Skill 不能因路径提及而自动注入、同 hash trust + `allow-implicit-invocation` + `paths` 命中才可确定性激活、不匹配路径不激活且 implicit route 不变更 model。RFC 7009 需要真实 authorization server 发布 metadata 才能作端到端 remote smoke；本地单元验证覆盖 parser、metadata/credential 边界和 no-secret status projection。编译输出仍存在仓库既有 unused import/re-export/dead-code warnings，不能称为零 warning。

为避免把 library test 的 feature 集误当成可执行 CLI，另实际执行了：

```text
CARGO_INCREMENTAL=0 cargo build -p agena --offline
target/debug/agena inspect --json
```

最终二进制输出与测试 manifest 一致：22 plugins / 100 tools（Direct 53、Deferred 40、Hidden 2、Internal 5）/ 14 bundled Skills；`agena.skills` 的 hook 列表包含 `user.prompt.submit`，证明路径门控选择进入的是实际 app binary，而不是只在 test target 存在。

图像 artifact hardening 与交互浏览器 final-URL recheck 后，另实际执行并通过：

```text
CARGO_INCREMENTAL=0 cargo test -p agena-runtime-session --lib --offline -- --nocapture
CARGO_INCREMENTAL=0 cargo test -p agena-runtime-plugins --lib --offline -- --nocapture
git diff --check
```

结果为 session 73/73、runtime-plugins 57/57；新增 media 测试验证只接受 base64 data URL、extension 的受控推导，实际实现还在 decode 前后实施 50 MiB 上限和 SHA-256 回填；Skill watcher 测试覆盖已有 root 的递归监听及缺失 root 最近父目录的非递归监听。浏览器单元与真实 CDP fixture 继续通过；Fetch-domain 逐请求 interception 尚未被伪称为已验证能力。

OAuth health/status 投影补强后，又在当前工作树实际执行并通过：

```text
cargo fmt
git diff --check
CARGO_INCREMENTAL=0 cargo test -p agena-mcp-client -p agena-runtime-plugins --lib --offline -- --nocapture
CARGO_INCREMENTAL=0 cargo check -p agena-runtime -p agena-cli --offline
```

结果为 `agena-mcp-client` 11/11、`agena-runtime-plugins` 54/54 单元测试通过；后两项静态检查通过。编译仍显示仓库既有的 unused import/re-export/dead-code warnings，未被删除、抑制或误写为零 warning。
