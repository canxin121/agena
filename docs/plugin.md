# Plugin 体系

Agena 的扩展能力统一通过 plugin host 接入。模型可见的 entries、MCP server 暴露出来的能力、LSP 查询、skills、workflow、memory、hooks、cron 等，都会在 runtime 中表达为 plugin 或 plugin entry，然后进入同一个 entry registry。

这意味着使用者和开发者应该把 plugin 当成主要抽象：

- 要新增一个模型可调用能力，写 plugin entry。
- 要接入 MCP server，配置 MCP server，然后由 `agena.mcp` static plugin 暴露 entries。
- 要扩展 prompt、provider、权限、shell env、事件、UI 状态栏等运行时行为，订阅 plugin hook。
- 要给 plugin 持久化状态或 secret，使用 plugin host callback。
- 要观察运行状态、日志、marketplace 安装状态，走 plugin CLI/API。

## 总览

Runtime build 会构建一个 `PluginHost`。这个 host 加载 runtime 注册的 static plugin，以及用户在 `[plugins.list.<id>]` 中声明的 plugin。

```text
config.toml
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
  loaded plugins             entry registry             hook dispatch
        |                          |                          |
        v                          v                          v
  status/log/quota          model-visible entries       runtime extension
```

`PluginHost` 负责：

- 按 config 加载 plugin transport。
- 调用 `init`，拿到 plugin manifest。
- 将 manifest 中的 entries 注册到 entry registry。
- 在 entry 调用、chat、权限、provider、session、event 等阶段分发 hooks。
- 为 plugin 提供 host callbacks。
- 维护 plugin status、logs、quota 和 inspect 信息。
- 在配置 reload 时复用配置完全一致的 plugin transport。

## Plugin 和 Entry

一个 plugin 是一组扩展能力。它可以只提供 hooks，也可以提供一个或多个 entries。

一个 entry 是模型可调用的能力单元。Entry 包含：

- `name`: plugin 内部 entry 名称。
- `description`: 展示给模型和 UI 的说明。
- `input_schema`: JSON Schema，用来描述调用入参。
- `behavior`: 行为类别，用于 plan mode、catalog 和权限推断。
- `input_paths`: 从调用入参中提取本地路径并做读写权限审计。
- `input_networks`: 从调用入参中提取网络目标并做网络权限审计。
- `network_access`: entry 固定访问的网络目标。
- `tags`: 权限策略没有命中精确 entry 规则时使用的标签。
- `search_terms`: entry search/catalog 的检索词。
- `load_priority`: entry 是否总是加载、标准加载或延迟加载。
- `concurrency_safe`: 是否允许并发执行。
- `requires_user_interaction`: 是否需要用户交互。
- `strict`: 是否启用更严格的 schema/调用约束。
- `plan_mode_policy`: plan mode 中的可用性策略。
- `streaming`: 是否支持流式 entry 输出。
- `host_capabilities`: 这个 entry 调用 host callback 时需要的能力声明。

Entry 的模型可见名称由 entry registry 决定：

- 如果名称没有冲突，暴露为 entry 原名。
- 如果和已有 entry 或内置名称冲突，暴露为 `plugin_id__entry_name`。
- 如果 manifest 设置了 `expose_as`，使用指定名称。

这个规则让 plugin 可以安全地提供通用名称，同时避免覆盖其他 plugin 的能力。

## First-Party Static Plugins

Runtime 会注册一组 first-party static plugins。这些 plugin 和外部 plugin 使用同一个 host、manifest、entry registry、hook dispatch 和 permission 路径。

Runtime build 注册：

| Plugin id | 作用 |
| --- | --- |
| `agena.fs` | 文件系统读写相关 entries |
| `agena.shell` | shell / powershell / monitor 相关 entries |
| `agena.web` | web search / web fetch entries |
| `agena.workflow` | plan mode、todo、worktree 等 workflow entries |
| `agena.skills_fs` | 扫描 `SKILL.md` 和 slash command，并动态注册 entries |
| `agena.lsp` | LSP server 观测和 LSP 查询 entries |
| `agena.cron` | cron 和 one-shot wakeup 调度 entries |
| `agena.memory` | memory 配置和项目记忆相关能力 |
| `agena.hooks` | 用户配置的 shell/HTTP hooks |
| `agena.mcp` | 已配置 MCP server 的 tool/resource/prompt entries |

`agena.mcp` 读取 MCP server snapshot，并把每个 MCP capability 包装成 plugin entries，例如：

```text
mcp:<server>:tool:<tool>
mcp:<server>:resources:list
mcp:<server>:resources:read
mcp:<server>:prompts:list
mcp:<server>:prompts:get
```

因此，MCP 对模型的可见面统一进入 plugin host 和 plugin entry registry。

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

Timeout 字段：

| 字段 | 默认用途 |
| --- | --- |
| `init` | `init` / manifest 初始化。 |
| `tool_hook` | tool before/after/failure hooks。 |
| `tool_invoke` | plugin entry invoke timeout。 |
| `permission_ask` | permission hook。 |
| `chat` | chat message/params/headers/system transform。 |
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

HTTP transport 的 auth 写在 plugin entry 内：

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
            .entry(
                PluginEntryDecl::new(
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
                .behavior(EntryBehavior::ReadOnly),
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

## Hooks

Plugin 可以通过 manifest 的 `hooks` 订阅 runtime 生命周期和调用链事件。Host 只会向订阅了对应 hook 的 plugin 分发事件。
Hook 名称遵循 plugin SDK 协议命名；`tool.*` hook 作用于 plugin entry 调用链。

主要 hook 组：

| 组 | Hooks |
| --- | --- |
| init/shutdown | `init`, `shutdown` |
| tool | `tool.execute.before`, `tool.execute.after`, `tool.execute.failure`, `tool.invoke`, `tool.invoke.stream`, `tool.definition` |
| chat | `chat.message`, `chat.messages.transform`, `chat.params`, `chat.headers`, `chat.system.transform` |
| provider/auth | `provider.list`, `auth` |
| permission | `permission.ask` |
| command/shell | `command.execute.before`, `command.execute.after`, `shell.env` |
| session | `session.start`, `session.end`, `session.compacting`, `session.compacted` |
| turn | `pre_turn`, `post_turn`, `user.prompt.submit`, `agent.stop` |
| event/notification | `event`, `notification` |
| config | `config` |

典型用途：

- 在 `provider.list` 中注入 plugin-provided provider。
- 在 `chat.headers` 中给 provider request 增加 header。
- 在 `chat.system.transform` 中改写 system prompt。
- 在 `permission.ask` 中为组织策略提供建议或决策。
- 在 `shell.env` 中注入 shell 环境变量。
- 在 `tool.execute.before/after` 中改写 entry 调用入参或输出。
- 在 `tool.definition` 中调整 entry definition。

## Host Capabilities

Plugin 调用 host callback 前必须在 manifest 中声明对应 capability。Host 会按 plugin 和 entry 做 capability 校验，避免 plugin 拿到未声明的宿主能力。

常用 capability：

| Capability | 能力 |
| --- | --- |
| `AskUser` | 向用户询问输入 |
| `SpawnSubtask` | 启动 subtask/subagent |
| `ListTools` | 列出可用 entries |
| `InvokeTool` | 调用其他 entry |
| `ReadConfig` | 读取配置 |
| `PublishEvent` / `SubscribeEvents` | 发布或订阅事件 |
| `Scheduler` / `CronScheduler` | 管理调度任务 |
| `PlanRegistry` | 访问 plan registry |
| `WorktreeRegistry` | 访问 worktree registry |
| `LspRegistry` | 访问 LSP registry |
| `McpRegistry` | 管理或查看 MCP server |
| `EntryRegistry` | 动态注册或注销 entries |
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

Capability 可以写在 entry 上，也可以写在 manifest 的 `plugin_capabilities` 上。Entry-level capability 更适合多 entry plugin，因为它可以把敏感能力限定到需要的 entry。

## 权限关系

Plugin entry 调用会经过同一套 permission system：

1. Entry manifest 声明 `input_paths`、`input_networks`、`network_access`。
2. Plugin 可以在运行时通过 `permission_paths` / `permission_networks` 补充动态审计项。
3. Permission runtime 检查 path/network/entry policy。
4. `permission.ask` hooks 可以给出建议；拥有 `PermissionDecision` capability 的 plugin 可以返回最终决策。
5. 需要用户确认时，session 状态和 UI/API 会产生 pending permission request。

Manifest 中的权限声明适合 entry 调用前就能知道的资源：

```rust
PluginEntryDecl::new("download", schema)
    .input_path(InputPathSpec {
        jsonpath: "$.output_path".to_string(),
        kind: PathKind::Write,
        optional: false,
    })
    .input_network(InputNetworkSpec {
        jsonpath: "$.url".to_string(),
        optional: false,
    })
    .network_access(NetworkAccessSpec {
        target: "https://api.example.com".to_string(),
    })
    .tag("network");
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

这些声明和动态返回项都会在 entry body 执行前进入同一套 path/network policy。

Plugin 内部发起的额外文件或网络操作不能由 host 做强沙箱隔离。需要在 plugin 内部配合权限系统时，manifest 要声明 `PermissionCheck` capability，然后通过 host callback 主动检查：

```rust
host.ensure_path_permission(HostPathPermissionCheckRequest::write(path)).await?;
host.ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url)).await?;
```

也可以使用 `check_path_permission` / `check_network_permission` 拿到 `allow`、`prompt`、`deny` 结果自行处理。Host 会按当前 session、agent、persisted rule、permission hook 和静态 policy 解析该检查。

Entry 权限配置分为 tag、entry name 和 entry-specific rules：

```toml
[permission.entries.tags]
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"
internet = "ask"
task = "ask"
shell = "ask"

[permission.entries.names]
bash = "ask"
apply_patch = "ask"
"my-plugin.echo" = "allow"
```

`names` 覆盖 first-party static plugin entries 和外部 plugin entries。

## MCP

MCP server 本身配置在 `agena.mcp` static plugin options，并通过 `agena.mcp` plugin entries 对模型暴露。

```toml
[plugins.list."agena.mcp"]
kind = "static"

[plugins.list."agena.mcp".options.servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
```

Runtime build 时：

1. 从 `plugins.list["agena.mcp"].options` 读取 MCP server config。
2. 构建 `McpConnectionManager`。
3. 注册 `agena.mcp` static plugin。
4. `agena.mcp` 从 MCP manager 读取 tool/resource/prompt capabilities。
5. 每个 MCP capability 进入 plugin entry registry。

因此，MCP 的权限、catalog、调用、hook、status 都落在 plugin 体系中。

## Plugin Storage 和 Secrets

Plugin storage 是 plugin-scoped 的 key/value 存储。一个 plugin 不能读取另一个 plugin 的 storage。

默认目录：

```text
~/.agena/plugin-storage
```

覆盖目录：

```bash
export AGENA_PLUGIN_STORAGE_DIR=/var/lib/agena/plugin-storage
```

Storage 按 `plugin_id / namespace / key` 组织，底层是 JSON 文件。目录和文件会尽量使用受限权限。

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
- `--force`: 覆盖已有同名 plugin entry。
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
| `GET` | `/api/v1/plugins/{plugin_id}` | inspect plugin status、manifest、authority |
| `GET` | `/api/v1/plugins/{plugin_id}/logs` | retained logs |
| `POST` | `/api/v1/plugins/marketplace/search` | 搜索 registry |
| `POST` | `/api/v1/plugins/marketplace/sync` | 同步 registry |
| `GET` | `/api/v1/plugins/marketplace/installed` | 已安装 marketplace plugins |
| `GET` | `/api/v1/plugins/marketplace/outdated` | 可升级 plugins |
| `POST` | `/api/v1/plugins/marketplace/install` | 安装 plugin |
| `POST` | `/api/v1/plugins/marketplace/uninstall` | 卸载 plugin |
| `POST` | `/api/v1/plugins/marketplace/upgrade` | 升级 plugin |
| `POST` | `/plugin-rpc/{plugin_id}` | plugin UI/assets 或外部 plugin 管理面调用 plugin JSON-RPC |

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
6. 在 `config.toml` 的 `[plugins.list.<id>]` 中配置。
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

Runtime reload 会重建 runtime snapshot 和 plugin host。对配置完全一致的 plugin entry，host 会复用已有 transport，所以未变更的 stdio subprocess 或 HTTP plugin 可以在 reload 后继续存活。

发生以下变化时通常会重新加载对应 plugin：

- plugin id 变化。
- kind 变化。
- path/command/url/options/timeouts/env/restart 等 entry config 变化。
- trusted key、signature、hash 等校验信息变化。

加载失败的 plugin 不会阻止整个 host 构建。Host 会记录 failed status 和 error log，其他 plugin 仍可继续运行。

## 实现索引

关键实现文件：

- Plugin config schema: `crates/agena-plugin-host/src/config.rs`
- Plugin host/load/reload/status/logs: `crates/agena-plugin-host/src/host.rs`
- Entry registry and name collision handling: `crates/agena-plugin-host/src/registry.rs`
- Plugin manifest and hooks: `crates/agena-plugin-sdk/src/manifest.rs`
- Plugin trait and SDK runtime surface: `crates/agena-plugin-sdk/src/plugin.rs`
- Host callbacks: `crates/agena-plugin-sdk/src/host_api.rs`
- Static plugin registration: `crates/agena/src/config/registry.rs`
- First-party plugin ids and bridge: `crates/agena/src/entry/mod.rs`
- Bundled plugins: `crates/agena/src/plugins/bundled/`
- MCP plugin bridge: `crates/agena/src/plugins/bundled/mcp.rs`
- Plugin storage/secrets: `crates/agena/src/plugins/storage.rs`
- Marketplace manifest/install/cache: `crates/agena-plugin-marketplace/`
- CLI plugin commands: `crates/agena/src/cli.rs`
- Backend plugin APIs: `crates/agena-api-server/src/lib.rs` and `crates/agena-api-server/src/rest.rs`
