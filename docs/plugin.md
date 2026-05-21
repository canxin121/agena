# Plugin 体系

Agena 的扩展能力统一通过 plugin host 接入。模型可见的 tools、MCP server 暴露出来的能力、LSP 查询、skills、workflow、memory、cron 等，都会在 runtime 中表达为 plugin 或 plugin tool，然后进入同一个 tool registry。

这意味着使用者和开发者应该把 plugin 当成主要抽象：

- 要新增一个模型可调用能力，写 plugin tool。
- 要接入 MCP server，配置 MCP server，然后由 `agena.mcp` static plugin 暴露 tools。
- 要扩展 prompt、provider、权限、shell env、事件等运行时行为，订阅 plugin hook。
- 要扩展 TUI 或 Studio 前端界面，在 manifest 的 `ui.tui` / `ui.studio` 中声明 UI contributions。
- 要给 plugin 持久化状态或 secret，使用 plugin host callback。
- 要观察运行状态、日志、marketplace 安装状态，走 plugin CLI/API。

## 总览

Runtime build 会构建一个 `PluginHost`。这个 host 加载 runtime 注册的 static plugin，以及用户在 `[plugins.list.<id>]` 中声明的 plugin。

```text
config.json
  |
  +-- [plugins] -------------------+
  |                                |
  +-- [plugins.list."agena.*"]     |
       |                           |
       v                           v
RuntimeConfigRegistry        PluginHostBuilder
       |                           |
       +-- register static plugins |
       +-- configure agena.* ------+
                                   |
                                   v
                             PluginHost
                                   |
        +--------------------------+--------------------------+
        |                          |                          |
  loaded plugins             tool registry             hook dispatch
        |                          |                          |
        v                          v                          v
  status/log/quota          model-visible tools       runtime extension
```

`PluginHost` 负责：

- 按 config 加载 plugin transport。
- 调用 `init`，拿到 plugin manifest。
- 将 manifest 中声明的 tools 注册到 tool registry。
- 在 tool 调用、chat、权限、provider、session、event 等阶段分发 hooks。
- 为 plugin 提供 host callbacks。
- 聚合 plugin manifest 和动态 host callback 提供的 UI contributions。
- 维护 plugin status、logs、quota 和 inspect 信息。
- 在配置 reload 时复用配置完全一致的 plugin transport。

## Plugin 和 Tool

一个 plugin 是一组扩展能力。它可以只提供 hooks，也可以提供一个或多个 tools。

一个 tool 是模型可调用的能力单元。Tool 包含：

- `name`: plugin 内部 tool 名称。
- `description`: 展示给模型和 UI 的说明。
- `summary`: 简短说明。开启 help 模式时，模型只看到这个简短说明和 `tools help` 引导。
- `help`: 详细帮助文本。不会直接进入 provider tool definition；需要时通过 `tools` 的 `help` action 获取。
- `description_mode`: tool 自己建议的模型可见说明模式，`detailed` 或 `help`。运行时配置可以覆盖它。
- `input_schema`: JSON Schema，用来描述调用入参。
- `input_paths`: 从调用入参中提取本地路径并做读写权限审计。
- `input_networks`: 从调用入参中提取网络目标并做网络权限审计。
- `path_access`: tool 固定会访问的本地路径，带 `read/write` 语义。
- `network_access`: tool 固定访问的网络目标。
- `tags`: tool 的规范化语义标签。权限 fallback、catalog/profile 过滤、tool search，以及运行时的一些判断都基于 tags、name 和 description。
- `load_priority`: tool 是否总是加载、标准加载或延迟加载。
- `concurrency_safe`: 是否允许并发执行。现在是显式字段，不再从 tags 自动推导。
- `strict`: 是否启用更严格的 schema/调用约束。
- `streaming`: 是否支持流式 tool 输出。
- `host_capabilities`: 这个 tool 调用 host callback 时需要的能力声明。

Tool 的模型可见名称由 tool registry 决定：

- 如果名称没有冲突，暴露为 tool 原名。
- 如果和已有 tool 名称冲突，暴露为 `plugin_id/tool_name`。

这个规则让 plugin 可以安全地提供通用名称，同时避免覆盖其他 plugin 的能力。

## Tool 说明模式和 Help

Tool 的模型可见说明有两种模式：

- `detailed`: 默认行为。Provider tool definition 使用 tool 的 `description`。
- `help`: Provider tool definition 只使用短说明，并提示模型调用 `tools` 的 `help` action 获取完整帮助。

详细帮助不随 provider 请求一起发送，避免把大量 tool、MCP server 或 skill 的长说明塞进每次模型调用。需要完整用法时，模型可以调用：

```json
{"action": "help", "tool": "fs", "include_schema": true}
```

其中 `tool` 是模型可见 tool 名称；如果有重名冲突，使用 registry 暴露出来的 `plugin_id/tool_name` 名称。`include_schema` 默认为 `true`，会把注册后的 input schema 一并返回。

配置优先级从高到低：

1. `[plugins.tool_presentation.tools]` 中的具体 tool 覆盖。
2. `[plugins.tool_presentation.plugins]` 中的 plugin 覆盖。
3. manifest 里的 `description_mode`。
4. `[plugins.tool_presentation].default_mode`。

示例：

```toml
[plugins.tool_presentation]
default_mode = "help"

[plugins.tool_presentation.plugins]
"agena.skills" = "help"
"agena.mcp" = "help"

[plugins.tool_presentation.tools]
fs = "detailed"
"agena.workflow/tools" = "detailed"
```

`tools` 的 `search` action 用于发现和加载 deferred tools；`help` action 用于拿到任意已注册 tool 的详细说明。Plugin 作者可以在 manifest 中设置 `summary` 和 `help`，也可以通过 `tool.definition` hook 改写 `description`、`summary`、`help`、`description_mode` 和 `input_schema`。

## Provided Static Plugins

Runtime 会注册一组 runtime-provided static plugins。这些 plugin 和用户配置 plugin 使用同一个 host、manifest、tool registry、hook dispatch 和 permission 路径。

Runtime build 注册：

| Plugin id | 作用 |
| --- | --- |
| `agena.fs` | 文件系统读写相关 tools |
| `agena.shell` | shell exec / monitor 相关 tools |
| `agena.web` | web search / web fetch tools |
| `agena.workflow` | plan mode、todo、worktree 等 workflow tools |
| `agena.skills` | 扫描 `SKILL.md` 和 slash command，并动态注册 tools |
| `agena.lsp` | LSP server 观测和 LSP 查询 tools |
| `agena.cron` | cron 和 one-shot wakeup 调度 tools |
| `agena.memory` | memory 配置和项目记忆相关能力 |
| `agena.mcp` | 已配置 MCP server 的 tool/resource/prompt tools |
| `agena.settings` | 读取、列出、校验和编辑当前 `config.json` 的 settings tools |

内置 static plugin 使用少量领域级入口承载 action，避免把每个动作都展开成独立 tool。常见入口：

| Tool | Action |
| --- | --- |
| `fs` | `read`, `glob`, `grep`, `apply_patch`, `notebook_edit` |
| `shell` | `exec`, `monitor_start`, `monitor_list`, `monitor_read`, `monitor_stop` |
| `web` | `fetch`, `search` |
| `lsp` | `servers`, `definition`, `references`, `hover`, `diagnostics` |
| `settings` | `get`, `list`, `validate`, `set`, `delete`, `patch` |
| `schedule` | `list`, `create`, `delete`, `wakeup` |
| `workflow` | `init`, `review`, `security_review` |
| `tools` | `search`, `help` |
| `task` | `run` |
| `agent` | `switch`, `restore` |
| `todo` | `write` |
| `session` | `get`, `rename` |
| `goal` | `get`, `create`, `clear`, `complete` |
| `user` | `request_input` |
| `plan` | `enter`, `exit` |
| `worktree` | `enter`, `exit` |

这些入口现在统一使用扁平 `{"action": "...", ...}` 形状；旧的 `{"command": "...", "args": {...}}` 仍会保留一段兼容期。当前内置 static plugin 倾向把同域动作尽量收进一个顶层 tool，再通过 action 区分读、写、调度或交互行为。

`agena.mcp` 读取 MCP server snapshot，但不再把每个 MCP capability 展开成一个模型可见 tool：

| Tool | 作用 |
| --- | --- |
| `mcp` | resource/prompt 读取和 MCP tool 调用：`list_resources`, `read_resource`, `list_prompts`, `get_prompt`, `call` |

因此，MCP 对模型的可见面统一进入 plugin host 和 plugin tool registry，同时不会随 server/tool 数量线性膨胀。MCP 的网络权限按调用里的 `server` 动态审计。

`agena.settings` 使用当前 runtime 的 active config path。对 effective 读操作，推荐显式传 `scope = config|meta` 和相对 `path`，避免依赖 `config.` / `meta.` 前缀魔法。`settings` 里的写入类 action 默认先校验再写入，并在有实际变更时通过 `host/config.reload` reload runtime；`dry_run=true` 会返回差异但不落盘、不 reload。

## Transport

`[plugins.list.<id>]` 的 `kind` 选择 plugin transport。

| kind | 场景 | 关键字段 |
| --- | --- | --- |
| `static` | runtime 内注册的 in-process plugin | `options`, `timeouts` |
| `cdylib` | 本地动态库 plugin | `path`, `options`, `timeouts`, `sha256`, `signature` |
| `stdio` | 子进程 JSON-RPC plugin | `command`, `args`, `env`, `cwd`, `restart`, `options`, `timeouts`, `sha256` |
| `http` | 远端 JSON-RPC plugin | `url`, `auth`, `options`, `timeouts` |
| `wasm` | WebAssembly plugin | `path`, `options`, `timeouts`, `sha256` |

选择建议：

- 需要最低开销并且随 runtime 一起发布：用 `static`。
- Rust 本地 plugin、追求低延迟：用 `cdylib`。
- Node/Python/Go/任意语言子进程：用 `stdio`。
- 组织内已有服务或远端策略服务：用 `http`。
- 需要更强隔离和可移植 artifact：用 `wasm`。

## 配置

最小配置：

```toml
[plugins]
enabled = true

[plugins.list.echo]
kind = "stdio"
command = "node"
args = ["./plugins/echo/index.js"]
options = { uppercase = true }
```

完整一些的 stdio 配置：

```toml
[plugins]
enabled = true
timeouts = { init = "10s", tool_invoke = "60s", permission_ask = "10s" }

[plugins.list.lint]
kind = "stdio"
command = "node"
args = ["./plugins/lint/index.js"]
env = { LOG_LEVEL = "info" }
cwd = "."
restart = { policy = "on-failure", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { project = "rust" }
timeouts = { tool_invoke = "30s" }
```

HTTP plugin：

```toml
[plugins.list.cloud-policy]
kind = "http"
url = "https://policy.example.com/agena/rpc"
auth = { kind = "bearer", token_env = "AGENA_POLICY_TOKEN" }
options = { org_id = "acme" }
```

cdylib plugin：

```toml
[plugins.list.echo]
kind = "cdylib"
path = "examples/echo_plugin/target/debug/libagena_echo_plugin.so"
options = { uppercase = true }
```

Wasm plugin：

```toml
[plugins.list.sandboxed]
kind = "wasm"
path = "./plugins/sandboxed/plugin.wasm"
sha256 = "..."
```

Static plugin options override：

```toml
[plugins.list."agena.web"]
kind = "static"
options = { }
timeouts = { tool_invoke = "45s" }
```

`PluginHostBuilder::register_static` 会把已注册 static plugin 加入 load list。需要给 static plugin 设置 `options` 或 `timeouts` 时，显式写对应 `[plugins.list.<id>]`。

## `[plugins]` 字段

顶层 `[plugins]` 支持：

| 字段 | 含义 |
| --- | --- |
| `enabled` | 是否启用 plugin host。设为 `false` 时返回空 plugin host。 |
| `timeouts` | 全局 plugin timeout overlay。 |
| `list` | plugin 声明表，key 是 plugin id。 |
| `default_quota` | 没有单独 quota 的 plugin 使用的默认 host callback quota。 |
| `quotas` | 按 plugin id 设置 host callback quota。 |
| `trusted_keys` | cdylib signature 校验使用的 ed25519 public key。 |
| `tool_presentation` | 控制 tool 说明进入模型请求时使用 `detailed` 还是 `help` 模式。 |

Timeout 字段：

| 字段 | 默认用途 |
| --- | --- |
| `init` | `init` / manifest 初始化。 |
| `tool_hook` | tool before/after/failure hooks。 |
| `tool_invoke` | plugin tool invoke timeout。 |
| `permission_ask` | permission hook。 |
| `chat` | chat params/headers。Mutating chat prompt hooks are kept for compatibility but are not applied to provider prompts. |
| `fast` | shell env、command hooks、config hooks 等快速路径。 |

Duration 支持 `ms`、`s`、`m`、`h`：

```toml
[plugins]
timeouts = { init = "10s", tool_invoke = "2m", fast = "500ms" }
```

Quota 用于限制 plugin 调用 host callback 的频率和并发数。默认不限制。

```toml
[plugins.default_quota]
rate_per_sec = 20
burst = 40
max_concurrent = 8

[plugins.quotas."cloud-policy"]
rate_per_sec = 5
burst = 10
max_concurrent = 2
```

Supply-chain 校验可以使用 artifact hash 和 trusted key：

```toml
[plugins.trusted_keys]
acme = "0123456789abcdef..."

[plugins.list.secure-plugin]
kind = "cdylib"
path = "./plugins/secure/libsecure.so"
sha256 = "..."
signature = { key_id = "acme", signature = "..." }
```

## Stdio Restart

`stdio` plugin 支持 restart policy：

```toml
[plugins.list.worker]
kind = "stdio"
command = "./plugins/worker"
restart = { policy = "always", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
```

可用 `policy`：

- `never`
- `on-failure`
- `always`

默认值：

- `policy = "on-failure"`
- `min_backoff = "1s"`
- `max_backoff = "30s"`
- `max_retries = 5`

Runtime status 会记录 stdio plugin 的 pid、restart count、last exit code、last restart time 和 last error。其他 transport 通常以 running state 呈现，不带 pid/restart 字段。

## HTTP Auth

HTTP transport 的 auth 写在 plugin tool 内：

```toml
[plugins.list.policy]
kind = "http"
url = "https://policy.example.com/agena/rpc"
auth = { kind = "none" }
```

Bearer token：

```toml
auth = { kind = "bearer", token_env = "PLUGIN_TOKEN" }
```

Basic auth：

```toml
auth = { kind = "basic", username = "user", password_env = "PLUGIN_PASSWORD" }
```

HTTP plugin 初始化时，host 会把 callback URL 和 bearer token 放进 `InitContext`。Plugin 通过 callback URL 调用 host API 时需要携带这个 bearer token。

## Manifest

Plugin manifest 是 plugin 和 host 之间的契约。它可以由 plugin 在 `meta/manifest` JSON-RPC 方法中返回，也可以由 SDK 的 `Plugin::manifest()` 构造。

Manifest 包含：

- `schema_version`
- `name`
- `version`
- `description`
- `authors`
- `transports`
- `hooks`
- `entries`
- `plugin_capabilities`
- `ui`
- `options_schema`

Rust SDK 中的最小 plugin：

```rust
use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct EchoPlugin;

#[async_trait]
impl Plugin for EchoPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("echo", env!("CARGO_PKG_VERSION"))
            .description("Echo text.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(
                PluginToolDecl::new(
                    "echo",
                    json!({
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" }
                        },
                        "required": ["text"]
                    }),
                )
                .description("Echo the supplied text.")
                .tag(ToolTag::ReadOnly),
            )
            .build()
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let text = input
            .input
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        Ok(ToolInvokeOutput::text(text.to_string()))
    }
}
```

导出为 cdylib：

```rust
agena_plugin_sdk::export_cdylib!(EchoPlugin);
```

导出为 stdio：

```rust
#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::io::Result<()> {
    agena_plugin_sdk::drivers::stdio::serve_stdio(EchoPlugin::default()).await
}
```

仓库内示例：

- `examples/echo_plugin`: cdylib plugin。
- `examples/echo_plugin_stdio`: stdio plugin。

## Plugin UI

Plugin UI 是 manifest 的一部分，由 plugin host 统一聚合后提供给 TUI、Studio Web 和 API。它不再是 Studio 前端单独维护的一套 plugin UI registry；前端只消费 runtime 当前 plugin host 给出的 catalog。

Manifest 的 UI surface 明确拆成两个宿主面：

| 字段 | 面向宿主 | 内容 |
| --- | --- | --- |
| `ui.tui.statusline_segments` | TUI | 静态状态栏片段。也可以继续通过 `host/ui.statusline.*` 动态贡献和删除。 |
| `ui.tui.themes` | TUI | TUI theme palette。也可以通过 theme host callback 动态注册。 |
| `ui.tui.content_blocks` | TUI | TUI 文本块，当前默认位置是 `composer_footer`。 |
| `ui.studio.commands` | Studio Web/Desktop | Command Palette 和 slash command 可发现的 plugin commands。 |
| `ui.studio.controls` | Studio Web/Desktop | Plugin detail panel 等位置上的按钮、选择器、开关、文本和数字 controls。 |
| `ui.studio.views` | Studio Web/Desktop | Plugin detail 或其他 Studio 位置可渲染的 markdown/link/control 组合视图。 |

TUI 和 Studio 的 UI contribution 类型不同，是有意的边界：TUI 只处理可在终端稳定渲染的状态栏、主题和文本块；Studio 可以渲染 command palette、controls、views，并把用户操作映射为统一的 `PluginUiAction`。

Studio control 的 `kind` 当前支持：

| kind | 渲染和提交 |
| --- | --- |
| `button` | 普通按钮，点击后执行 action。 |
| `select` | 使用 `options` 渲染下拉框，执行时把当前值作为 `{ "value": "<option>" }` 传给 action。 |
| `checkbox` / `toggle` / `switch` | 渲染布尔开关，执行时把当前值作为 `{ "value": true|false }` 传给 action。 |
| `text` 或其他未知 kind | 渲染文本输入，执行时把当前文本作为 `{ "value": "..." }` 传给 action。 |
| `number` | 渲染数字输入，执行时把当前数字作为 `{ "value": 123 }` 传给 action；空值传 `null`。 |

`PluginUiAction` 支持：

| Action | 行为 |
| --- | --- |
| `none` | 只展示，不执行行为。 |
| `invoke_tool` | 调用当前 plugin 的某个 tool；`input` 是默认 JSON object，前端或 API 请求里的 input 会覆盖同名字段；`submit_output_as_prompt` 为 true 时，Studio 会把 tool 输出放回聊天输入/下一轮 prompt。 |
| `open_route` | Studio 前端打开内部 route。 |
| `open_url` | Studio 前端打开外部 URL。 |
| `submit_prompt` | Studio 把固定 prompt 放入聊天流程。 |

Rust SDK 示例：

```rust
use agena_plugin_sdk::prelude::*;
use serde_json::json;

fn manifest() -> PluginManifest {
    PluginManifest::builder("project-helper", env!("CARGO_PKG_VERSION"))
        .tool(
            PluginToolDecl::new(
                "summarize",
                json!({
                    "type": "object",
                    "properties": {
                        "scope": { "type": "string" }
                    }
                }),
            )
            .description("Summarize the requested project scope.")
            .tag(ToolTag::ReadOnly),
        )
        .tui_content_block(PluginTuiContentBlock {
            id: "workspace-hint".into(),
            title: "Project".into(),
            body: "Custom project helper is active.".into(),
            location: "composer_footer".into(),
            priority: 10,
            color: None,
        })
        .studio_command(PluginStudioCommand {
            id: "summarize-workspace".into(),
            title: "Summarize workspace".into(),
            description: "Run the project helper summary tool.".into(),
            category: "Project".into(),
            slash: Some("/project-summary".into()),
            aliases: vec!["workspace summary".into()],
            usage: Some("/project-summary".into()),
            location: "command_palette".into(),
            action: PluginUiAction::InvokeTool {
                tool: "summarize".into(),
                input: Some(json!({ "scope": "workspace" })),
                submit_output_as_prompt: true,
            },
        })
        .build()
}
```

Plugin host 公开的统一 catalog 形状是：

```json
{
  "tui": {
    "statusline_segments": [],
    "themes": [],
    "content_blocks": []
  },
  "studio": {
    "commands": [],
    "controls": [],
    "views": []
  }
}
```

Catalog item 会带上 `plugin_id`，便于 Studio 在执行 command/control/view control 时把 action 解析回 owning plugin。对于 statusline 和 theme，静态 manifest contribution 会和动态 host callback contribution 合并；同一 plugin 的动态 statusline segment 会覆盖同 id 的静态 segment，动态 theme 会覆盖同 id 的静态 theme。

Studio 使用两条执行路径：

- runtime skill/command 或 plugin tool 的直接调用走 `POST /api/v1/plugins/ui/invoke-tool`。
- manifest 中的 Studio command/control/view control action 走 `POST /api/v1/plugins/{plugin_id}/ui/actions/{action_id}`。

这两个 REST 入口都会经过 plugin host 的 tool registry 和 permission 检查。当前 direct UI invocation 没有交互式 permission confirmation 流；如果调用需要 ask/deny，API 会返回 409，Studio 需要把结果作为无法直接执行的操作处理。

## Hooks

Plugin 可以通过 manifest 的 `hooks` 订阅 runtime 生命周期和调用链事件。Host 只会向订阅了对应 hook 的 plugin 分发事件。
Hook 名称遵循 plugin SDK 协议命名；`tool.*` hook 作用于 plugin tool 调用链。

主要 hook 组：

| 组 | Hooks |
| --- | --- |
| init/shutdown | `init`, `shutdown` |
| tool | `tool.execute.before`, `tool.execute.after`, `tool.execute.failure`, `tool.invoke`, `tool.invoke.stream`, `tool.definition` |
| chat | `chat.message`, `chat.messages.transform`, `chat.params`, `chat.headers`, `chat.system.transform` |
| provider/auth | `provider.list`, `auth` |
| permission | `permission.ask` |
| command/shell | `command.execute.before`, `command.execute.after`, `shell.env` |
| session | `session.start`, `session.end` |
| turn | `pre_turn`, `post_turn`, `user.prompt.submit`, `agent.stop` |
| event/notification | `event`, `notification` |
| config | `config` |

Provider-visible prompt content is append-only within a session so upstream prompt caches can match stable prefixes. For that reason `chat.message`, `chat.messages.transform`, and `chat.system.transform` are not applied on the provider request path: they can rewrite, drop, or reorder system/messages. `chat.params` and `chat.headers` still apply because they do not mutate the transcript prefix.

典型用途：

- 在 `provider.list` 中注入 plugin-provided provider。
- 在 `chat.headers` 中给 provider request 增加 header。
- 在 `chat.params` 中调整 temperature、max output tokens 等 request 参数。
- 在 `permission.ask` 中为组织策略提供建议或决策。
- 在 `shell.env` 中注入 shell 环境变量。
- 在 `tool.execute.before/after` 中改写 tool 调用入参或输出。
- 在 `tool.definition` 中调整 tool definition。

## Host Capabilities

Plugin 调用 host callback 前必须在 manifest 中声明对应 capability。Host 会按 plugin 和 tool 做 capability 校验，避免 plugin 拿到未声明的宿主能力。

常用 capability：

| Capability | 能力 |
| --- | --- |
| `AskUser` | 向用户询问输入 |
| `SpawnSubtask` | 启动 subtask/subagent |
| `ListTools` | 列出可用 tools |
| `InvokeTool` | 调用其他 tool |
| `ReadConfig` | 读取配置 |
| `PublishEvent` / `SubscribeEvents` | 发布或订阅事件 |
| `Scheduler` / `CronScheduler` | 管理调度任务 |
| `PlanRegistry` | 访问 plan registry |
| `WorktreeRegistry` | 访问 worktree registry |
| `LspRegistry` | 访问 LSP registry |
| `McpRegistry` | 管理或查看 MCP server |
| `EntryRegistry` | 动态注册或注销 tools |
| `AgentRegistry` | 注册或读取 agent profiles |
| `HookRegistry` | 动态注册 hook |
| `PluginStorage` | 使用 plugin-scoped storage |
| `PluginSecrets` | 使用 plugin-scoped secret store |
| `PluginStatus` | 查看 plugin status |
| `Statusline` | 贡献 UI statusline segment |
| `Theme` | 注册 UI theme palette |
| `PermissionUi` | 接管 permission UI render |
| `PermissionDecision` | 在 permission hook 中返回最终决策 |
| `PermissionCheck` | 让 plugin 主动请求 host 按当前 path/network policy 做权限判断 |

Capability 可以写在 tool 上，也可以写在 manifest 的 `plugin_capabilities` 上。Tool-level capability 更适合多 tool plugin，因为它可以把敏感能力限定到需要的 tool。

## 权限关系

Plugin tool 调用会经过同一套 permission system：

1. Tool manifest 声明 `input_paths`、`input_networks`、`network_access`。
2. Plugin 可以在运行时通过 `permission_paths` / `permission_networks` 补充动态审计项。
3. Permission runtime 检查 path/network/tool policy。
4. `permission.ask` hooks 可以给出建议；拥有 `PermissionDecision` capability 的 plugin 可以返回最终决策。
5. 需要用户确认时，session 状态和 UI/API 会产生 pending permission request。

Manifest 中的权限声明适合 tool 调用前就能知道的资源：

```rust
PluginToolDecl::new("download", schema)
    .input_path(InputPathSpec {
        jsonpath: "$.output_path".to_string(),
        kind: PathKind::Write,
        optional: false,
    })
    .path_access(PathAccessSpec {
        path: ".agena/cache".to_string(),
        kind: PathKind::Write,
    })
    .input_network(InputNetworkSpec {
        jsonpath: "$.url".to_string(),
        optional: false,
    })
    .network_access(NetworkAccessSpec {
        target: "https://api.example.com".to_string(),
    })
    .tag(ToolTag::Network);
```

如果路径或网络目标要先解析输入、读取 plugin 状态、展开 workspace 信息后才能知道，plugin 可以实现：

```rust
async fn permission_paths(
    &self,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<Vec<PathRequest>>;

async fn permission_networks(
    &self,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<Vec<NetworkRequest>>;
```

这些声明和动态返回项都会在 tool body 执行前进入同一套 path/network policy。

Plugin 内部发起的额外文件或网络操作不能由 host 做强沙箱隔离。需要在 plugin 内部配合权限系统时，manifest 要声明 `PermissionCheck` capability，然后通过 host callback 主动检查：

```rust
host.ensure_path_permission(HostPathPermissionCheckRequest::write(path)).await?;
host.ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url)).await?;
```

也可以使用 `check_path_permission` / `check_network_permission` 拿到 `allow`、`prompt`、`deny` 结果自行处理。Host 会按当前 session、agent、persisted rule、permission hook 和静态 policy 解析该检查。

Tool 权限配置分为 tag、tool name 和 tool-specific rules：

```toml
[permission.entries.tags]
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"
internet = "ask"
task = "ask"
shell = "ask"

[permission.entries.names]
shell = "ask"
fs = "ask"
"my-plugin.echo" = "allow"
```

`names` 覆盖 runtime-provided 和 user-configured plugin tools。

## MCP

MCP server 本身配置在 `agena.mcp` static plugin options，并通过 `agena.mcp` plugin tools 对模型暴露。

```toml
[plugins.list."agena.mcp"]
kind = "static"

[plugins.list."agena.mcp".options.servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
```

Remote HTTP:

```toml
[plugins.list."agena.mcp".options.servers.remote]
transport = "http"
url = "https://mcp.example.com"
mode = "streamable_http"
headers = { }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
```

Remote WebSocket:

```toml
[plugins.list."agena.mcp".options.servers.browser]
transport = "ws"
url = "wss://mcp.example.com/socket"
headers = { }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
```

Runtime build 时：

1. 从 `plugins.list["agena.mcp"].options` 读取 MCP server config。
2. 构建 `McpConnectionManager`。
3. 注册 `agena.mcp` static plugin。
4. `agena.mcp` 从 MCP manager 读取 tool/resource/prompt capabilities。
5. 每个 MCP capability 进入 plugin tool registry。

因此，MCP 的权限、catalog、调用、hook、status 都落在 plugin 体系中。

## Plugin Storage 和 Secrets

Plugin storage 是 host 提供的统一 key/value 存储接口。它现在按两个维度组织：

- `scope`: `session`、`workspace`、`global`
- `visibility`: `private`、`shared`

组合起来就是：

- `session + private`: 当前 plugin 在当前会话下的私有数据
- `session + shared`: 当前会话下多个 plugin 共享的数据
- `workspace + private`: 当前 plugin 在当前 workspace 下的私有数据
- `workspace + shared`: 当前 workspace 下多个 plugin 共享的数据
- `global + private`: 当前 plugin 在整个 runtime 下的私有数据
- `global + shared`: 整个 runtime 下多个 plugin 共享的数据

兼容旧插件：如果请求里没有显式传 `scope` / `visibility`，host 默认按
`global + private` 处理，也就是旧的 plugin-scoped storage 语义。

默认目录：

```text
~/.agena/plugin-storage
```

覆盖目录：

```bash
export AGENA_PLUGIN_STORAGE_DIR=/var/lib/agena/plugin-storage
```

Storage 按 `scope / visibility / namespace / key` 组织；`private` bucket 会再自动带上
`plugin_id`。当前默认文件布局大致是：

```text
global/private/<plugin_id>/<namespace>.json
global/shared/<namespace>.json
workspace/<workspace-hash>/private/<plugin_id>/<namespace>.json
workspace/<workspace-hash>/shared/<namespace>.json
session/<session_id>/private/<plugin_id>/<namespace>.json
session/<session_id>/shared/<namespace>.json
```

目录和文件会尽量使用受限权限。

Secrets 使用独立 keyring service：

```text
agena.plugin
```

当系统 keyring 不可用时，可以 fallback 到 plugin storage 目录下的文件实现。Plugin 必须声明 `PluginStorage` 或 `PluginSecrets` capability 才能使用对应 callback。

## Marketplace

Marketplace 是 plugin 的安装和升级分发层。Registry index 是 JSON，包含 plugin id、名称、描述、homepage 和 versions。每个 version 描述：

- `version`
- `kind`
- `platform`
- `url`
- `sha256`
- `signature`
- `command`
- `args`
- `env`
- `options`
- `min_agena_version`
- `archive`
- `dependencies`

安装时，marketplace client 会解析 registry、选择版本、下载 artifact、校验 hash/signature、写入 active config 的 `[plugins.list.<id>]`，并记录安装元数据。

CLI：

```bash
agena plugin sync https://example.com/marketplace.json
agena plugin search lint https://example.com/marketplace.json
agena plugin install lint@1.2.3 --registry https://example.com/marketplace.json
agena plugin list-installed
agena plugin outdated
agena plugin upgrade lint
agena plugin upgrade --all
agena plugin uninstall lint
```

常用安装参数：

- `--config <path>`: 写入指定 config。
- `--force`: 覆盖已有同名 plugin tool。
- `--dry-run`: 计算结果但不写文件。
- `--allow-unverified`: 允许没有 sha256 的 artifact。
- `--require-signature`: 要求 registry record 带 signature。
- `--refresh`: 安装前刷新 registry index。

Marketplace cache 默认目录：

```text
~/.agena/marketplace
```

覆盖目录：

```bash
export AGENA_MARKETPLACE_DIR=/var/lib/agena/marketplace
```

## 诊断和运维

CLI：

```bash
agena plugin status
agena plugin inspect <plugin-id>
agena plugin logs <plugin-id>
agena plugin logs <plugin-id> --after-seq 100 --limit 100
agena plugin status --format json
agena plugin inspect <plugin-id> --format toml
```

Studio/backend API：

| Method | Path | 用途 |
| --- | --- | --- |
| `GET` | `/api/v1/plugins` | plugin runtime status list |
| `GET` | `/api/v1/plugins/ui` | 当前 runtime 的统一 plugin UI catalog |
| `POST` | `/api/v1/plugins/ui/invoke-tool` | 从 Studio/TUI 支持面直接调用 plugin tool |
| `GET` | `/api/v1/plugins/{plugin_id}` | inspect plugin status、manifest、authority |
| `GET` | `/api/v1/plugins/{plugin_id}/logs` | retained logs |
| `POST` | `/api/v1/plugins/{plugin_id}/ui/actions/{action_id}` | 执行 manifest Studio UI action |
| `POST` | `/api/v1/plugins/marketplace/search` | 搜索 registry |
| `POST` | `/api/v1/plugins/marketplace/sync` | 同步 registry |
| `GET` | `/api/v1/plugins/marketplace/installed` | 已安装 marketplace plugins |
| `GET` | `/api/v1/plugins/marketplace/outdated` | 可升级 plugins |
| `POST` | `/api/v1/plugins/marketplace/install` | 安装 plugin |
| `POST` | `/api/v1/plugins/marketplace/uninstall` | 卸载 plugin |
| `POST` | `/api/v1/plugins/marketplace/upgrade` | 升级 plugin |
| `POST` | `/plugin-rpc/{plugin_id}` | plugin UI/assets 或 user-configured plugin 管理面调用 plugin JSON-RPC |

`plugin inspect` 会包含：

- runtime status。
- manifest。
- authority summary。

`plugin logs` 来自 host retained log store，包含 seq、timestamp、level、source、message 和 fields。

## 开发流程

开发一个 plugin 的基本步骤：

1. 选择 transport。
2. 使用 `agena-plugin-sdk` 实现 `Plugin` trait。
3. 在 `manifest()` 中声明 hooks、entries、capabilities 和 options schema。
4. 在 `tool_invoke` 或相关 hook 方法中实现行为。
5. 按 transport 导出 plugin。
6. 在 `config.json` 的 `[plugins.list.<id>]` 中配置。
7. 用 `agena config validate` 验证配置。
8. 用 `agena plugin status` 和 `agena plugin inspect <id>` 验证加载结果。

cdylib 构建：

```bash
cargo build -p agena-echo-plugin
```

stdio 构建：

```bash
cargo build -p agena-echo-plugin-stdio
```

配置后检查：

```bash
agena config validate
agena plugin status
agena plugin inspect echo
```

## Reload 行为

Runtime reload 会重建 runtime snapshot 和 plugin host。对配置完全一致的 plugin tool，host 会复用已有 transport，所以未变更的 stdio subprocess 或 HTTP plugin 可以在 reload 后继续存活。

发生以下变化时通常会重新加载对应 plugin：

- plugin id 变化。
- kind 变化。
- path/command/url/options/timeouts/env/restart 等 tool config 变化。
- trusted key、signature、hash 等校验信息变化。

加载失败的 plugin 不会阻止整个 host 构建。Host 会记录 failed status 和 error log，其他 plugin 仍可继续运行。

## 实现索引

关键实现文件：

- Plugin config schema: `crates/agena-plugin-host/src/config.rs`
- Plugin host/load/reload/status/logs: `crates/agena-plugin-host/src/host.rs`
- Tool registry and name collision handling: `crates/agena-plugin-host/src/registry.rs`
- Plugin manifest and hooks: `crates/agena-plugin-sdk/src/manifest.rs`
- Plugin trait and SDK runtime surface: `crates/agena-plugin-sdk/src/plugin.rs`
- Host callbacks: `crates/agena-plugin-sdk/src/host_api.rs`
- Static plugin registration: `crates/agena/src/config/registry.rs`
- Provided tool ids and bridge: `crates/agena/src/entry/mod.rs`
- Provided plugins: `crates/agena/src/plugins/provided/`
- MCP plugin bridge: `crates/agena/src/plugins/provided/mcp.rs`
- Plugin storage/secrets: `crates/agena/src/plugins/storage.rs`
- Marketplace manifest/install/cache: `crates/agena-plugin-marketplace/`
- CLI plugin commands: `crates/agena/src/cli.rs`
- Backend plugin APIs: `crates/agena-api-server/src/lib.rs` and `crates/agena-api-server/src/rest.rs`
