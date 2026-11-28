# 统一项目会话执行

项目工作由 Session Kernel 承载。AI Bot 和用户操作同一份项目会话：创建、发送消息、继续、读取结果、停止执行。内部并行分工使用现有 Subagent，状态、工具输出、权限卡片和消息顺序都属于原生会话。

## 外部 Agent 会话

Agena 的「ACP」页面连接服务器上的内置外部 Agent（Antigravity、Claude Code）或管理员在全局配置中登记的任意标准 ACP（stdio）Agent。外部 Agent 使用自己的登录和模型设置，并保存原生对话历史。Agena 在独立的内存会话中展示实时消息、附件、工具结果和权限交互。Agena 通过 ACP 传递用户输入，在同一次执行中通过本地 MCP 返回 Agena 工具的结果。

1. 在运行 `agena server` 的机器上，用同一个系统账号安装所需的 Agent。Antigravity 使用支持 ACP 的 `agy`，直接使用当前系统账号默认的 `~/.gemini` 配置和历史目录。可以在 Agena 的 ACP 登录界面完成原生登录，或在该机器的交互终端运行 `agy acp --login`。[Antigravity ACP 文档](https://antigravity.google/docs/ide/extensions/acp)
2. 使用 Claude Code 时，安装 `claude-agent-acp`（Node.js 22+ 可执行 `npm install -g @agentclientprotocol/claude-agent-acp`），并用官方 `claude auth login` 登录。将适配器加入 Agena Server 的 `PATH`。适配器使用官方 Claude Agent SDK 与 Claude Code。[Claude ACP 适配器](https://github.com/agentclientprotocol/claude-agent-acp)
3. 接入其他标准 ACP Agent（或覆盖、停用内置的 `claude-code` / `antigravity`）时，在**实例级全局** `agena.json` 的 `acp.agents` 中登记（工作区 `.agena/config.json` 不允许配置 `acp`，以防不受信任的仓库注入可执行命令）：

```json
{
  "acp": {
    "agents": {
      "codex": {
        "name": "Codex CLI",
        "command": "codex-acp",
        "args": ["--stdio"],
        "env": {
          "CODEX_PROFILE": "work"
        },
        "client_tools": true,
        "writable_paths": ["/home/user/.codex"]
      },
      "antigravity": {
        "disabled": true
      }
    }
  }
}
```

- Agent ID 须为 1–64 个小写 ASCII 字母、数字、`-` 或 `_`。
- `name`（可选）：Web / TUI / CLI 中显示的名称，默认使用内置名称或 Agent ID。
- `command` / `args`：启动 ACP stdio 进程的程序名（按 `PATH` 解析）或绝对路径及固定参数；自定义 Agent 的 `command` 必填，内置 Agent 省略时沿用默认命令。
- `env`（可选）：注入到该 Agent 子进程的额外环境变量（禁止 `AGENA_*` 前缀）。`GET /api/v1/acp/agents` 不会返回 `env` 与 `writable_paths`。
- `client_tools`（可选）：是否在 ACP v1 握手中声明客户端 `fs` / `terminal` 委托能力（自定义 Agent 默认为 `true`，ACP v2 忽略此项）。
- `writable_paths`（可选）：启用 `AGENA_ACP_SANDBOX_FILE` 进程隔离时允许该 Agent 读写的原生状态绝对路径。
- `disabled`（可选）：设为 `true` 时隐藏并禁止启动该 Agent（可用于停用内置 Agent）。

4. 在 Agena Web 左侧打开「ACP」，选择项目和 Agent，点击「新建会话」。发送第一条消息时，Agent 开始原生会话。也可以在终端查看当前实例可用的 ACP Agent 目录并创建会话，再用返回的 `session.id` 打开 TUI。

```sh
agena sessions agent-sessions agents
agena sessions create --workspace /path/to/project --external-agent antigravity
agena sessions create --workspace /path/to/project --external-agent claude-code
agena sessions create --workspace /path/to/project --external-agent codex
agena tui --session <session-id>
```

REST 客户端可通过 `GET /api/v1/acp/agents` 查询当前启用的 ACP Agent 目录（仅返回 `id`、`name`、`builtin`、`command`、`args`、`available`，不暴露 `env`），并向 `POST /api/v1/acp/sessions` 传入 `workspace_id`、`title` 和 `external_agent`（如 `"antigravity"`、`"claude-code"` 或全局配置中的自定义 ID）。可用 `GET /api/v1/acp/sessions` 列出当前内存中的临时 ACP 视图，用 `PUT /api/v1/acp/sessions/{session_id}` 更新视图标题、提示词或 Profile，用 `/api/v1/acp/sessions/{session_id}/messages` 发送消息。普通会话列表和项目会话历史仍由 `/api/v1/sessions` 提供。 Web 的 ACP 侧边栏会展示当前项目已打开的临时视图列表，便于在尚未发送消息的视图之间切换。

在 Web 点输入框下方的外部 Agent 名称，或在 TUI 运行 `/model`，可以查看当前 Agent 提供的模型、思考强度、模式和其他选项。Agena 向 Agent 核对所选值，在当前临时会话中应用。Agent 在会话中主动变更选项时，Web 会同步显示最新值。选用「遵循外部 Agent 设置」时，会话采用该 Agent 的原生设置。

Agent 声明支持 `session/options` 时，Agena 按公布顺序在新建、恢复、加载和原生分叉时传入已选的 `configOptions`，并根据返回值核对结果。需要补设的值通过 `session/set_config_option` 应用。旧版 Agent 继续通过会话设置接口接收选项。[ACP 会话选项草案](https://agentclientprotocol.com/rfds/session-options)

```sh
agena sessions agent-options <session-id>
agena sessions agent-options <session-id> --preview
agena sessions agent-options <session-id> --model <advertised-model-id>
agena sessions agent-options <session-id> --thinking <advertised-value>
agena sessions agent-options <session-id> --mode <advertised-value>
agena sessions agent-options <session-id> --option '<advertised-kind>=<value>'
```

REST 可向 `POST /api/v1/acp/sessions/{session_id}/agent-options` 提交 `{}` 查看选项，或用 `{"change":{"kind":"model","value":"<advertised-model-id>"}}` 更改选择。思考强度使用 `kind: "thought_level"`，`value: null` 恢复由外部 Agent 决定。`{"preview":true}` 通过 `session/options` 查询新会话的预览。

### ACP 供应商路由

Agent 声明支持 ACP 供应商路由时，可用 `agena sessions agent-providers <session-id>`、`GET /api/v1/acp/sessions/{session_id}/agent-providers`、Web 选项面板中的「模型提供商路由」或 TUI「管理 ACP 会话」中的「模型提供商路由」查看新连接使用的供应商 ID、协议和地址。返回的地址会去掉用户信息、查询参数和片段。 [ACP Configurable LLM Providers 草案](https://agentclientprotocol.com/rfds/custom-llm-endpoint)

使用 CLI 的原生路由时，启动 Agena Server 可省略 `AGENA_ACP_PROVIDERS_FILE`。如需显式设置路由，将该变量指向服务器管理员维护的 JSON 文件，顶层键为对应的 ACP Agent ID（内置 ID 或 `acp.agents` 中的自定义 ID）。下例使用 Claude Code 公布的 `main` 供应商，`env` 引用的是 Agena Server 进程环境中的请求头值。

```json
{
  "claude-code": [
    {
      "action": "set",
      "providerId": "main",
      "apiType": "anthropic",
      "baseUrl": "https://api.anthropic.com",
      "headers": { "x-api-key": { "env": "AGENA_CLAUDE_PROVIDER_KEY" } }
    }
  ]
}
```

`set` 替换该供应商的完整路由配置，也可携带 Agent 定义的 `_meta`。要执行 `providers/disable`，使用 `{"action":"disable","providerId":"main"}`，所选供应商必须声明为可停用。Agena 会先通过 `providers/list` 核对能力，再把配置交给新连接上的 Agent。后续新建、恢复、回放或分叉会话时读取文件里的当前配置。

### 登录与原生命令

Web 的「外部 Agent 账号」面板显示 Agent 支持的账号状态、登录和退出选项。CLI 也可查看账号状态、选择 Agent 提供的协议登录方式和退出。

```sh
agena sessions agent-auth <session-id>
agena sessions agent-auth <session-id> --live
agena sessions agent-auth <session-id> --login <method-id>
agena sessions agent-auth <session-id> --logout
```

如果 Agent 提供终端登录方式，请在 Agena 服务进程所在的运行环境中用同一系统账号打开终端，并连接服务的回环地址。可用 `--server http://127.0.0.1:<port>` 指定地址。用 `--terminal` 查看可用方式，再用返回的方法 ID 登录。

```sh
agena sessions agent-auth <session-id> --terminal
agena sessions agent-auth <session-id> --terminal --login <terminal-method-id>
```

支持 `auth/status_update` 时，当前连接的账号状态会显示在 Web 账号面板和 TUI 的外部 Agent 活动中。`--live` 和 `GET /api/v1/sessions/{session_id}/agent-auth` 读取这份状态。没有收到报告时返回 `null`，连接关闭后这份状态会被清除。

Web 和 TUI 的 `/` 命令菜单会显示当前会话的外部 Agent 发布的命令、说明和参数提示。外部命令以 `/agent:` 开头，发送时还原为 `/<命令>` 通过标准 ACP `session/prompt` 直接交给外部 Agent。在 Claude Code 或 Antigravity 的 ACP 会话中，用 `/agent:compact` 调用对方原生的 `/compact` 压缩当前会话。直接输入 `/compact` 会调用 Agena 的压缩操作，该操作适用于 Agena 原生会话。选中带参数提示的外部命令会先把命令填入输入框。

ACP v2 Agent 通过 `session/options` 提供预览时，第一次打开命令菜单即可查看它公布的部分原生命令。原生会话打开后，菜单根据实际返回的命令和后续更新刷新。旧版 Agent 通常在第一条消息后发布列表。已经知道命令名称时，可以直接提交 `/agent:<命令>`。

支持 ACP v2 结构化技能的 Agent 会在同一个菜单发布技能。选中技能后可以补写任务，例如 `/agent:summarize 请概括这个文件`。Agent 声明支持在任意位置插入技能时，也可从 Web 的技能选择器或 TUI 的 `Ctrl+A` 插入菜单添加多个技能引用。原生历史保留每个技能的名称和所在位置。[ACP Agent Skills 草案](https://agentclientprotocol.com/rfds/agent-skills)

### 原生历史与会话管理

在 TUI 的命令面板打开「ACP 原生会话」，可选择工作区和 Agent，翻页浏览原生历史，打开已有会话或新建会话。打开会话后，命令面板的「管理 ACP 会话」显示该适配器提供的会话操作及可用的历史位置。「ACP 账户与登录」提供适配器声明的登录方式，并给出服务器主机的终端登录命令。

在 ACP 侧边栏选定项目和 Agent 后，历史列表从该 Agent 请求。点击某条记录的「打开」时，Agena 按原生会话 ID 回放 Agent 当前提供的消息、思考内容、附件和工具事件，后续消息可以接着原生上下文发送。再次打开空闲会话会重新回放。如果该会话正在运行、有排队消息或等待用户答复，Agena 会接入当前执行。

会话投影、执行状态和聊天附件在 Agena 服务内存中使用。同一次服务运行期间，重开空闲会话会保留 Agena 的 Profile、提示词、会话选择、权限和额外目录。服务重启后，再次打开原生历史会分配新的临时 ID。旧版已经写进 `agena.db` 的 ACP 记录保留原数据，普通会话列表会过滤这些记录。工具在项目里执行的文件修改按项目权限处理。

回放失败时，原有临时视图可继续使用。

```sh
agena sessions agent-sessions --workspace /path/to/project list claude-code
agena sessions agent-sessions --workspace /path/to/project list claude-code --cursor '<next-cursor>'
agena sessions agent-sessions --workspace /path/to/project open claude-code '<native-id>'
agena sessions agent-sessions --workspace /path/to/project preview-history claude-code '<native-id>' [--before-turn <n>]
```

Agent 在 ACP v1 或 v2 声明公开归档能力时，原生历史列表支持筛选、归档和取消归档。Agena 对当前项目里的会话调用 `session/archive` 或 `session/unarchive`。早期的 Claude Code v1 适配器可以通过 AIR 索引提供归档。AIR 还提供批量查询和变更通知，Web 会按通知刷新当前项目的历史。

```sh
agena sessions agent-sessions --workspace /path/to/project list claude-code --archived all
agena sessions agent-sessions --workspace /path/to/project batch-get claude-code '<native-id>'
agena sessions agent-sessions --workspace /path/to/project archive claude-code '<native-id>'
agena sessions agent-sessions --workspace /path/to/project unarchive claude-code '<native-id>'
agena sessions agent-sessions --workspace /path/to/project watch claude-code --wait-seconds 10
```

对应的 REST 操作是 `index_list`、`batch_get`、`set_archived` 和 `watch`。`index_list` 的 `archived` 参数可设为 `unarchived`、`archived` 或 `all`。索引查询和变更都以当前项目目录为范围。

打开长会话时，Agena 滚动接收回放，优先载入最近的完整轮次。超出视图容量时，历史里会标出省略的轮次或内容。主视图载入的附件合计最多 8 MiB，每条记录最多 16 个附件。从 Web 的原生会话列表选「预览」，或在 TUI 的原生会话菜单选「预览历史」，可按页查看更早的用户和助手文字。预览是只读操作，打开的会话视图保持可用。每页至多八个轮次，每个角色每轮至多 32 KiB 的文字，完整记录由原生 Agent 保存。

预览沿用 `POST /api/v1/acp/workspaces/{workspace_id}/sessions`。使用 `{"action":"preview_history","agent":"claude-code","native_id":"<native-id>"}` 获取最新页。可以把响应中的 `preview.next_before_turn` 作为 `before_turn` 请求上一页。Claude Code 当前的 ACP v2 适配器用 `session/resume` 从头回放，ACP v1 使用 `session/load`。Agena 在本地选页，翻页时 Agent 仍会重发历史，所以长会话的加载时间取决于原生回放。预览要求原生 Agent 提供历史回放能力。[ACP v1 加载会话](https://agentclientprotocol.com/protocol/session-setup#loading-sessions) · [Claude Code ACP v2 适配器](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/src/v2/agent.ts)

在 ACP 侧边栏点击「原生会话」，可查看关联信息和 Agent 提供的重命名、设置额外目录、关闭、删除或分叉操作。修改操作需要当前会话空闲。额外目录会按项目读取权限检查，下一次打开原生会话时生效。

Agena 合成全局、工作区、匹配 ACP 路由的模型层、Profile 和会话提示词，并按配置加入项目指令和预加载技能。内置指引让 Agent 优先通过 `agena` MCP 处理项目文件、Shell 和网络操作，缺少合适的 MCP 工具时使用原生工具。单次请求的 `system` 可以替换配置提示词层，项目指令、预加载技能和内置指引仍会追加。ACP 的提示词预览显示将追加的正文。Agent 在握手中声明相应能力时，Agena 在新建、恢复或分叉时追加系统提示词。Claude Code 直接使用标准 ACP 的 `systemPrompt`（保留原生系统提示词快照与缓存），Antigravity 在 ACP v1 使用 `_meta.antigravity.systemPrompt.append`。[ACP 客户端系统提示词](https://agentclientprotocol.com/rfds/client-system-prompt)

ACP 会话跳过 Agena 的会话、消息、命令、Shell 和工具 hooks，包括由这些 hooks 提供的自动记忆召回。Agena 按原始输入发送用户文本、附件和技能引用，并按用户输入路由 `/agent:` 命令。重新打开会话时，历史取自原生 Agent。

```sh
agena sessions agent-session <session-id> status
agena sessions agent-session <session-id> rename 'Project conversation'
agena sessions agent-session <session-id> set-directories ./shared /path/to/another/project
agena sessions agent-session <session-id> fork --title 'Agena branch title'
agena sessions agent-session <session-id> rewind --before-message-id <message-id>
agena sessions agent-session <session-id> close
agena sessions agent-session <session-id> delete
```

`fork` 在 Agent 中创建原生分支，并打开该分支的临时视图。`close` 释放原生会话资源，后续仍可恢复。`delete` 删除 Agent 保存的历史并关闭 Agena 的临时视图。`set-directories` 省略路径时清空目录列表。

Agent 提供原生游标时，可在 Web 和 TUI 选择一条用户消息，从该消息之前分叉。分支保留该位置之前的 Agena 记录和原生上下文。CLI 可从 `status` 返回的 `fork_points` 选取 `message_id`，传给 `fork --before-message-id <message-id>`。支持 AIR `sessionRewind` 的 Claude Code v1 连接还会发布 `rewind_points`，用于在同一原生会话中回退。从原生历史重新打开会话可读取这种连接通过回放发布的位置。REST 的 `fork` 和 `rewind` 操作都使用 `before_message_id`。

会话列表和打开操作使用 `POST /api/v1/acp/workspaces/{workspace_id}/sessions`。例如 `{"action":"list","agent":"claude-code"}` 或 `{"action":"open","agent":"claude-code","native_id":"<native-id>"}`。会话管理使用 `POST /api/v1/acp/sessions/{session_id}/agent-session`，接受 `{"action":"status"}`、`{"action":"set_title","title":"Project conversation"}` 或 `{"action":"fork"}`。已有的 `import` 命令和兼容 REST 入口也使用临时视图。

### 运行中交互与协议版本

外部 Agent 正在运行时，在 Web 输入文本后点击「插话」，或在 Web、TUI 按 `Alt+Enter`，可把文本交给当前运行。Agena 在 Agent 确认接收后记录这条追加指令。原生接口为 `POST /api/v1/sessions/{session_id}/agent-steer`，请求体为 `{"text":"请同时检查测试"}`。

```sh
agena sessions agent-steer <session-id> '请同时检查测试'
```

支持 ACP v2 的 Agent 如果为某个工具调用、终端或子 Agent 发布取消能力，可以在 Web 的「动态」面板或 TUI 命令面板的「取消外部 Agent 任务」中单独发送取消请求。Claude Code v1 发布的可停止异步任务也出现在这里。目标的最终状态由 Agent 后续更新。

```sh
agena sessions agent-work <session-id>
agena sessions agent-work <session-id> --cancel <cancel-id>
```

REST 可用 `GET /api/v1/sessions/{session_id}/agent-work` 查看当前可取消项，用同一路径 `POST` `{"cancel_id":"<cancel-id>"}` 请求取消。列表中的 `cancel_id` 对应该连接的当前能力。目标完成、能力撤回或重连后需重新查询。

ACP 请求超时或调用方撤回正在等待的插话与单项操作时，Agena 用 `$/cancel_request` 通知 Agent 取消对应的请求 ID。停止当前执行时走 `session/cancel`，随后结算 Agena 的本地执行和工具授权。

Agent 从 ACP 或 MCP 撤回单个 Agena 工具请求时，对应的审批和仍在执行的工具会取消，同一轮的其他工具可以继续。ACP 断线时，本轮签发的工具授权会撤回，运行中的本地工具完成清理后才结算断线错误。

提示词发出后，如果 Agent 持续 180 秒没有报告当前执行的进展，Agena 会发送 `session/cancel`、撤销工具授权并报告超时。等待用户答复、浏览器授权或 Agena 工具完成期间会延后计时。前台完成后，Agena 会继续观察 Agent 已报告的独立后台任务。后台清空后会短暂等待可能随之开始的前台工作。服务器可通过 `AGENA_ACP_PROMPT_IDLE_TIMEOUT_SECS` 配置无进展时限，单位为秒，有效范围为 1 至 86400。[ACP 后台工作](https://agentclientprotocol.com/protocol/v2/prompt-lifecycle#background-work)

Agena 连接外部 Agent 时自动协商 ACP 版本。支持 v2 的 Agent 使用 v2，旧版 Agent 会通过新的 v1 握手连接。当前 Claude Code 适配器会选择 **ACP v2 Preview**，Antigravity 会选择 **ACP v1**。在 v2 会话里，Agent 的确认响应表示已接收消息，Agena 继续跟踪运行状态和已报告的后台任务。

Agena 默认优先使用 Streamable HTTP。也可以在服务器环境中用 `AGENA_ACP_MCP_TRANSPORT` 指定 `http`、`acp`、`sse` 或 `stdio`，Agena 根据 Agent 声明的能力选择可用的传输。stdio 是 ACP v1 的基础传输，旧式 SSE 只用于声明支持它的 v1 Agent。没有共同 MCP 传输时，普通 v2 连接仍可使用原生会话，聊天记录会提示当前轮的 Agena 工具不可用。MCP 工具使用当前轮次的权限，工具出口随轮次结束撤销。

- Claude Code 和 Antigravity 保留原生工具、钩子、权限设置和斜杠命令。Agent 执行原生工具时应用自身的权限规则，Agena 可展示 Agent 发来的审批。通过 `agena` MCP 调用的插件工具应用当前 Agena Profile 的工具权限和审批。授权后产生的图片附件可以作为 MCP 图片内容返回，例如通过 `fs.read_media` 查看本地截图。
- `agena` MCP 只声明 `plugins_list`、`tools_list`、`tools_help` 和 `tools_call`。Agent 用目录函数发现插件和工具，通过 `tools_help` 查参数，然后把工具名称与参数传给 `tools_call`，例如 `{"tool":"fs.read","input":{"file_path":"README.md"}}`。
- Antigravity 和 Claude Code 都直接使用各自默认的原生配置与历史目录（分别对应 `~/.gemini` 与 `CLAUDE_CONFIG_DIR` / `~/.claude`）以及工作区的原生设置。[Antigravity 钩子](https://antigravity.google/docs/hooks?tab=cli) · [Antigravity MCP 配置](https://antigravity.google/docs/mcp)
- 旧版 Agena 创建的 Antigravity 会话可能带有 `agy` 保存的受限工具目录，续聊时可继续使用 Agena MCP。新建 ACP 会话会使用当前的原生工具目录。
- `agy 1.5.18` 的原生分叉和删除可在官方终端里用 `/fork` 和 `/resume` 会话选择器操作。Agena 的原生管理入口随 ACP 适配器公布的分叉、归档和删除能力开放。[Antigravity 分叉](https://antigravity.google/docs/cli-conversations) · [Antigravity 删除会话](https://antigravity.google/docs/cli/commands/resume/#deleting-a-conversation)
- 支持客户端委托的 ACP v1 Agent 可以通过文件和终端回调使用 Agena 的文件和 Shell 工具，终端句柄绑定到当前轮次。Antigravity 使用原生文件与终端工具，并可通过 MCP 调用相应的 Agena 工具。
- 一轮成功后，服务可以保留 ACP 读流以接收标题、命令列表、通知和当前原生会话发来的交互。空闲时的表单、授权链接和原生权限仍使用 Agena 的交互卡。下一轮恢复会话时会撤回旧连接的待答请求。Agena 的 MCP 工具、文件和终端回调需要活跃轮次。ACP v2 恢复连接时，正在打开的原生会话可以使用该轮授予的 MCP 工具，重新发出的原生权限请求会在同一轮继续等待用户答复。
- ACP 表单请求可以在 Web 和 TUI 中作答。表单支持文本、整数、数字、布尔值、单选和多选。可选字段可以跳过，回答在发给 Agent 前按表单校验。
- 协议登录如果发来绑定到该次登录的交互请求，也会在对应 Agena 会话中显示。通过 CLI 发起此类登录时，可以在同一会话的 Web 或 TUI 中回答。TUI 使用 `Ctrl+R` 打开待答输入。
- Agent 请求打开授权页面时，Web 和 TUI 会先展示完整地址。确认后由客户端打开页面，Agent 报告授权完成时会在对话中显示。对于原生工具权限，Agena 会展示 Agent 提供的全部选项，并回传用户选择的选项 ID。
- 等待表单或权限答复时仍可接收 Agent 的流式更新。Agent 撤回某个请求后，Agena 会关闭对应卡片，当前轮和其他待答请求可以继续。
- ACP v2 的助手消息和思考内容按原生消息 ID 更新。对于已记录的消息，Agent 可在运行中、空闲时或恢复连接时发送完整快照，在原位置替换文字、链接和附件。此前创建的分叉继续读取分叉时的版本。
- Agent 发布的清单和 Markdown 计划会显示在 Web 和 TUI 的计划查看器里。计划按 Agent 给出的 ID 更新和移除，既有的 ACP 清单也能显示。新建原生对话或清空上下文时会清除旧的进度。
- Agent 提供上下文用量时，Agena 显示它报告的已用 Token 和窗口容量。恢复同一个原生会话时可以查看上次的读数。Claude Code v1 的轮次响应和 v2 的前台完成状态会把所报告的单轮 Token 写入会话用量。Web 输入框下方的外部 Agent 活动面板和 TUI 的外部 Agent 活动命令还可以查看 Agent 报告的费用、通知、压缩进度、子 Agent 状态和只读终端输出。
- 原生工具行可以显示工具报告的文本、文件差异和终端输出，并在工具完成后附上它返回的媒体。外部 Agent 提供后续提示建议时，可以在 Web 活动面板或 TUI 命令菜单里选一条填入空白输入框，再由用户发送。
- 外部 Agent 声明支持时，可以在提示词里附带图片、音频或嵌入资源。附件经过项目读取权限校验，并以提交时的内容发送。外部 Agent 会收到当前消息选中的项目文件、目录和 Agena 技能引用，可以通过可用工具读取。其他文件可用项目路径引用。
- 连续对话优先使用 `session/resume`，也可通过 `session/load` 恢复。恢复遇到临时错误时，执行会报告错误，重试使用原生会话 ID。Agent 明确报告原生会话已不存在时，Agena 会提示并建立当前消息的新上下文。清空上下文或使用普通 Agena 分叉也会开始新上下文。Agent 到达单轮请求上限时，Agena 会提示并保留当前进度。
- 消息发送后断线或取消时，下一条新消息会续接同一个原生会话。v2 尚未确认的普通消息使用原提交键重试，并通过原生回放核对收到的消息 ID。Agent 明确在接受消息前拒绝请求，且 Agena 尚无该轮工具调用时，可以在同一会话重试。已确认的消息、交付不明的 v1 消息及可能只在当前连接生效的原生命令，适合先重开原生历史核对，再发送下一条消息。[ACP v2 提交重试](https://agentclientprotocol.com/protocol/v2/prompt-lifecycle#retrying-a-prompt)
- 外部 Agent 会话面向用户主动发起的项目对话。计划任务、渠道和集成事件的自动推理仍由 Agena 原生会话处理。
- Antigravity 的 ACP 可用性与个人账号、套餐、地区、版本及当时的厂商政策有关；请在部署前核对最新官方说明和适用条款。本项目不保证任意账号或套餐均可使用，也不构成厂商合规意见。[Google ACP 用法讨论](https://github.com/google-antigravity/antigravity-cli/issues/1493) · [Google Antigravity 条款](https://antigravity.google/terms) · [Anthropic Claude Code 条款](https://code.claude.com/docs/en/legal-and-compliance)。Claude Code 也使用用户自己的登录与用量，具体使用条件以 Anthropic 当前条款为准。

### ACP 进程隔离

安装并启用 [MXC 沙盒](sandbox.md) 后，可在服务进程设置 `AGENA_ACP_SANDBOX_FILE`，为 `claude-code`、`antigravity` 或 `acp.agents` 中配置的自定义 Agent ID 设置进程策略。策略将工作区及已授权的额外目录提供为只读，Agent 的原生配置和历史目录（以及自定义 Agent 的 `writable_paths`）可写。外部 CLI 按策略直连其模型与登录端点。Agena MCP 通过 ACP 连接或经认证的本机 stdio relay 提供。Linux 和 macOS 使用 Unix socket，Windows 使用本机命名管道。

```json
{
  "claude-code": {
    "network": {
      "version": 2,
      "default": "deny",
      "inbound": "deny",
      "hostLoopback": "deny",
      "allow": [{ "to": "203.0.113.10", "protocol": "tcp", "ports": [443] }]
    },
    "runtime_paths": []
  }
}
```

将示例 IP 替换成部署允许的 Agent 服务地址，并按需要允许 DNS 和登录端点。`runtime_paths` 可列出需只读挂载的运行依赖，例如安装适配器的 Node.js 和 npm 包目录。Claude Code 使用 `CLAUDE_CONFIG_DIR` 或服务账号的 `.claude`，Antigravity 使用服务账号的 `.gemini` 保存原生状态。某个 Agent 配置了此策略时，沙盒不可用会中止该 Agent 的连接。

Linux 和 Windows 可按目的 IP、协议和端口允许出站。macOS Seatbelt 支持允许或拒绝出站两种默认动作。macOS 上需要厂商联网时，可以显式设置 `default` 为 `allow`，工作区仍由 Seatbelt 设为只读。[各平台网络能力](sandbox.md)

在 Ubuntu 上，安装器为最终安装路径配置应用专用 AppArmor user namespace 授权。部署时可从该路径运行 `agena --agena-sandbox-check`，检查离线隔离和原生联网是否可用。[MXC 安装与自检](sandbox.md)

## Bot 与项目作用域

Bot 是实例级全局会话，项目关联和默认工作目录都为空；普通项目会话关联真实的已登记项目。全局会话的文件与命令操作必须明确路径基准，权限仍按实例与会话规则审查，不继承当前项目的规则、提示词或记忆。Bot 管理某个项目时只选择目标，不把自己的作用域改成那个项目。

## 组件职责

| 组件 | 职责 |
|---|---|
| `agena.bot` | `bot.profile` 修改当前 Bot 的资料，`bot.sessions` 操作项目与普通会话，`bot.connections` 管理接入草稿 |
| Session Kernel | 原生执行身份、执行代次、消息队列、幂等回执、取消与恢复 |
| Subagent | 项目会话内的委派、进度、结果和取消 |
| `agena-assistant` / `AssistantRuntime` | 记忆设置、提醒、接入授权与渠道收发 |
| 现有 SQLite 实例 | 保存会话、助手配置、规则、原生回执和事件审计 |

Bot 投递使用 `bot_instruction`；授权事件使用 `integration_instruction`；提醒和事件通知分别使用 `assistant_notice`、`integration_notice`。这些来源都是有明确身份的系统续接，不伪造新的用户输入。涉及用户授权的修改继续检查真实当前用户消息。

事件规则的 `session` 模式把执行投递到规则选定的普通项目会话，或明确授权的渠道服务主体会话。原生执行回执同时记录会话、执行 ID 和代次。规则暂停、到期、撤权或超时只取消对应执行，用户随后发起的人工工作继续保留。

## 数据库基线

SQLite 实例在首次启动时创建完整架构。已有数据必须匹配当前基线。使用空的 `--data-dir` 启动新基线时，原目录可以继续用于保存旧实例。

## 用量与费用

`/api/v1/usage` 汇总实例内的全部持久化用量，按页读取所有工作区和全局（Bot）会话。Web 与 TUI 使用同一份报表。`/api/v1/sessions/{id}/cost` 提供单个会话的模型、Token 与费用明细；Web 输入框上方的上下文进度可点击展开，显示当前上下文占用与该会话的历史 Token 与费用。

单条损坏或版本较新的 round 状态只跳过该条记录，不会让整个报表失败。

会话作用域设置（模型配置、权限、提示词层）使用同一套页面：作用域选择「当前会话」后，页面顶部的会话选择器默认最近使用的会话，也可切换并写入 `?sessionId=`。会话模型写入 `/api/v1/sessions/{id}/selection`，是会话自身的持久选择。

## 提示词前缀与缓存命中率

提供商通过逐字节相同的提示词前缀复用缓存。Companion 提供固定的提醒指引，模型通过 `companion.manage` 的 `profile` 或 `reminders` 动作读取实时钟、策略版本和提醒状态。具名 Bot 的附加系统指令由 Bot 插件加入对应会话的系统提示。

系统提示只承担模型每轮都需要的规则与稳定事实。工具的输入 schema、插件内部标识与路径、以及任何能通过工具重新读到的数据都不写进系统提示：执行契约由 `tools_help` 提供（`tools_call` 本来就必须先读到它），实时状态由返回它的工具提供。Agent roster 同理只保留身份与成员，不写会逐轮变化的执行状态。自动召回的记忆插在当前请求之前，而不是消息列表开头，这样本轮召回变化不会改写此前已缓存的整段对话。

当工具脚本 `tool_script.run` 出现在当前会话的工具目录时，内置系统提示会引导模型用它处理需要循环、分支、筛选或汇总的重复调用和依赖调用。模型先通过 `tools_help` 读取契约，再调用脚本。脚本里的 `tools.call` 沿用会话权限。单个简单操作，以及需要用户交互、流式输出或启动后台任务的工具，使用直接调用。

诊断方法：同一会话两轮请求的 `instructions` 应当完全一致；若不同，先比较差异位置，落在时钟或日程字段上即为前缀失效来源。

## 清空会话

`/clear`（别名 `/reset`）把一段对话重新开始，并提供两种模式。两者都不删除任何已存储的数据：条目、内容块与 segment 行都保留，可在需要时回读。

| 模式 | 对话记录 | 模型上下文 |
|---|---|---|
| 清空历史（默认） | 不再显示旧轮次 | 不再包含旧轮次 |
| 仅清空上下文 | 仍可阅读旧轮次 | 不再包含旧轮次 |

「清空历史」和 `truncate` 一样把 segment 退役到 `agena_retired_session_segments`，因此快照仍能寻址旧历史，只有在没有快照引用时才由回收流程释放。「仅清空上下文」不改动 membership，而是在当前尾部安装一个空检查点，把 prompt window 推过去；这条检查点不调用模型，也不会写入摘要文本。

两种模式都会先结算排队与运行中的执行，避免仍在运行的连接把输出写到新边界之后。Web 端 `/clear` 与侧栏会话菜单都会弹出模式选择框，TUI 同样先选模式再确认。

## 回归检查

```sh
python3 scripts/ci/check-task-retirement.py
python3 scripts/ci/check-global-bot-scope.py
cargo test -p agena-storage-sqlite --test global_scope --offline --locked -- --test-threads=1
cargo test -p agena --test system --offline --locked -- --test-threads=2
cargo test -p agena --test system scenarios::assistant_ui --offline --locked -- --ignored --test-threads=1
cargo test -p agena --test system scenarios::bot_operator_ui --offline --locked -- --ignored --test-threads=1
cargo test -p agena --test system scenarios::bot_profiles_ui --offline --locked -- --ignored --test-threads=1
```

静态检查覆盖退役目录、生产类型、工具名、接口和生成清单。存储测试覆盖无项目会话的创建与继承、全局 Bot 分支、删除祖先后的权限边界、事务回滚与重启。数据库基线测试验证不兼容文件会被拒绝，并保留原始数据库、WAL 和恢复日志。实际产品测试覆盖 Bot、原生会话、Subagent、事件接入及 Web/TUI。
