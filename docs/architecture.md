# 架构说明

本文描述 Agena 仓库的主要模块、运行时边界、数据流和扩展点。它面向需要修改代码、接入 API、开发插件或排查 runtime 行为的开发者。

## 总览

Agena 的核心是 `crates/agena`。它负责加载配置、构建 runtime snapshot、管理 provider、插件、agent、会话、事件、权限、数据库和 tool 执行。不同用户界面和传输层都围绕同一个核心 runtime 工作：

```text
CLI / TUI / Studio Web / Desktop / API clients
                  |
                  v
        agena-api-server / CLI command handlers
                  |
                  v
             agena::runtime
                  |
      +-----------+-----------+-----------+
      |           |           |           |
  session     providers    plugins     event/db
 manager      registry      host        store
```

几个重要原则：

- `crates/agena-api` 只定义 wire types，不绑定 HTTP、WS、SSE 或 stdio。
- `crates/agena-api-server` 把这些 type 挂到 HTTP/REST、SSE、WebSocket、IPC、JSON-RPC app-server。
- Studio Web 通过 REST 和 session SSE 与 Studio server 通信。
- TUI 和 CLI 直接使用 `agena` core，不需要 HTTP server。
- Desktop app 是 Tauri shell，启动并连接 packaged `agena-studio` sidecar。
- 插件通过 `agena-plugin-host` 接入，既可以贡献 entries，也可以通过 hook 影响 prompt、provider、权限、shell env、事件、状态栏等。

## Workspace 结构

```text
apps/
  agena-cli/
  agena-tui/
  agena-studio-server/
  agena-studio-desktop/

crates/
  agena/
  agena-api/
  agena-api-server/
  agena-client/
  agena-keyring-store/
  agena-lsp/
  agena-mcp-client/
  agena-mcp-server/
  agena-marketplace-server/
  agena-otel/
  agena-plugin-host/
  agena-plugin-marketplace/
  agena-plugin-sdk/
  agena-rollout/
  agena-scheduler/
  agena-skills/

packages/
  agena-studio-web/

ops/
  agena-studio/

examples/
  echo_plugin/
  echo_plugin_stdio/
```

## Apps

### `apps/agena-cli`

生成 `agena` 二进制。主入口在 `apps/agena-cli/src/main.rs`，CLI 类型和命令实现主要在 `crates/agena/src/cli.rs`。

行为：

- 直接运行 `agena` 时启动 TUI。
- `agena tui` 显式启动 TUI。
- `agena exec`、`review`、`resume`、`continue` 等命令直接调用 core session runtime。
- `agena config`、`provider`、`auth`、`plugin`、`sessions` 等命令操作配置、provider、凭据、插件和会话。
- `agena app-server --transport stdio` 启动 JSON-RPC app-server，用于 IDE/外部进程集成。
- `agena mcp-server` 以 MCP server 形态暴露能力。

### `apps/agena-tui`

终端 UI。它直接构建 runtime 和 session manager，并使用 Ratatui/Crossterm 渲染交互界面。TUI 支持工作区、会话、消息、权限请求、用户输入请求、外部编辑器、剪贴板、多语言文本等。

### `apps/agena-studio-server`

Studio 后端应用，二进制名 `agena-studio`。它做三件事：

- 构建 `AgenaRuntime` 和数据库连接。
- 挂载公共 Studio 路由：`/health`、`/auth/session`。
- 挂载 `agena-api-server::router(...)` 到同一个 Axum app，并在需要时服务 `packages/agena-studio-web/dist` 静态资源。

如果未传 `--ui-dir`，它运行在 API-only 模式。

### `apps/agena-studio-desktop`

Tauri 桌面封装。包含标准 WebView2/Wry variant 和实验 CEF variant。桌面 app 不依赖外部代理目标，而是启动 packaged `agena-studio` backend sidecar。

### `packages/agena-studio-web`

Vue 前端。主要通过 `packages/agena-studio-web/src/agena/lib/agenaApi.ts` 调用 REST API，通过 session event stream 获取会话事件。基础 fetch 封装在 `packages/agena-studio-web/src/lib/api.ts`，会自动处理 UI auth token、cookie fallback 和 401 auth required 事件。

## Core crate: `crates/agena`

`crates/agena/src/lib.rs` 暴露 core 模块：

- `config`: TOML schema、env overlay、CLI override、provider auth/adapters normalization、provider registry build。
- `runtime`: runtime builder、snapshot、reload、background tasks。
- `session`: session manager、processor、history、store、cache、append-only prompt assembly。
- `event`: event bus、publisher、store、filter、envelope。
- `db`: SeaORM entities、migration、CRUD。
- `provider`: model providers、credential、auth、runtime retry、streaming。
- `permission`: path/network/tool permission、persisted rule、runtime request/reply。
- `agent` / `agents`: default agent policy、disk/runtime subagent registry。
- `tool` / `plugins/provided`: provided tools and runtime-provided static plugins.
- `memory`: project instructions and memory plugin.
- `storage`: database URL/path resolution.
- `tracing` / `metrics`: logging, DB tracing, process/provider/tool/session counters.

## Runtime lifecycle

Runtime construction starts with `AgenaRuntime::builder()` in `crates/agena/src/runtime/builder.rs`:

1. Resolve workspace root, defaulting to current directory.
2. Load config through `ConfigLoader`.
3. Connect database if a database URL/connection is supplied.
4. Run schema migration when auto-migrate is enabled.
5. Build initial `RuntimeSnapshot` generation 1.
6. Install runtime-backed host client into plugin host.
7. Apply tracing filter.
8. Spawn background reload and janitor tasks.

`RuntimeSnapshot` is the immutable service bundle for one config generation:

- `ConfigResolution`
- `ProviderRegistry`
- `PluginHost`
- `SubagentRegistry`
- optional `SessionManager`
- optional `McpConnectionManager`
- optional `LspRegistry`
- event bridge guard
- plugin shutdown guard
- maintenance policy and watch paths

Reload builds a fresh runtime snapshot and preserves byte-identical plugin transports where possible, so a config reload does not always restart long-running plugin subprocesses.

## Config flow

```text
LoadConfigRequest
  config_path + CLI overrides
        |
        v
ConfigLoader
  defaults -> file -> env overlay -> CLI --set
        |
        v
RawConfig
        |
        v
ResolvedConfig
  canonical provider.auth + provider.adapters
        |
        v
RuntimeSnapshot services
```

The resolved config builds:

- provider registry.
- plugin host.
- MCP/LSP static plugin options and runtime registries.
- web static plugin options.
- session manager config.
- agent permission defaults.

配置细节见 [配置说明](configuration.md)。

## Session architecture

`SessionManager` 是会话层的主入口。它持有：

- `SessionStore`: 数据库存储和 history/event projection。
- `EventPublisher`: 发布 domain events。
- `EventBus`: live broadcast。
- `SessionProcessor`: provider 调用、tool call loop、append-only prompt window。
- `ToolExecutor`: provided/plugin tool 执行。
- `RunRegistry`: active run control/cancel。
- session cache。

常见写操作：

```text
submit_user_message
continue_session
reply_permission
reply_user_input
fork_session
rewind_session (creates a fork)
cancel_active_run
export_session_jsonl
import_session_jsonl
```

一次 run 的高层流程：

```text
user parts
  |
  v
SessionManager reserves/appends message
  |
  v
SessionProcessor builds append-only provider prompt
  |
  v
ProviderRegistry resolves model/provider
  |
  v
provider complete/complete_stream
  |
  +--> text deltas/messages
  |
  +--> tool calls
         |
         v
      ToolExecutor
         |
         +--> provided plugin tools
         +--> configured plugins
         +--> permission runtime
         +--> path/network/tool policies
  |
  v
store history events + publish domain events
```

Session storage is database-backed when runtime is built with database config. Without database, `session_manager()` is absent, and API routes that need sessions return service unavailable/internal errors depending on call path.

## Event architecture

Events provide both persistence and live UI updates:

- `EventPublisher` writes persistent events to `EventStore` and broadcasts to `EventBus`.
- `EventBus` supports filtered live subscriptions by scope and kind.
- REST `/api/v1/events` reads persisted events.
- REST `/api/v1/sessions/{session_id}/events` reads session events.
- REST `/api/v1/sessions/{session_id}/events/stream` streams session events as SSE.
- `/api/v1/events/stream` streams global/workspace/session events as SSE notification frames.
- `/api/v1/ws` multiplexes commands, queries and subscriptions over WebSocket.
- Plugin event bridge forwards core events into plugin hooks when relevant.

Event filters use:

- scope: `global`、`workspace`、`session`
- optional kind set
- optional `since_seq_global`

## Provider architecture

Provider config resolves into `ProviderRegistry` through `crates/agena/src/config/registry.rs`.

Supported runtime provider families include:

- Ollama
- OpenAI
- OpenAI-compatible
- SAP AI Core
- Anthropic
- Gemini
- Codex
- GitLab
- Copilot
- Amazon Bedrock
- Google Vertex
- Cloudflare AI Gateway
- plugin-provided providers

Provider registry responsibilities:

- resolve default model and explicit model references.
- list models.
- expose model metadata/capabilities.
- apply runtime retry policy.
- handle streaming replay behavior.
- use managed credentials from env/provider-local auth/provider-specific auth.

Plugins can patch provider list through the `provider.list` hook, allowing plugin-provided model backends to appear in runtime status and selection flows.

## Plugin architecture

Plugin config is parsed by `agena-plugin-host`. `PluginHost` owns loaded plugins, tool registry, status registry, log store, transport runtime and host callback handle. Model-visible capabilities flow through plugin tools; filesystem/shell/web/workflow/skills/LSP/MCP are all represented inside the plugin host. For the full plugin surface and configuration details, see [Plugin 体系](plugin.md).

Transport kinds:

- `static`: in-process plugin registered by core runtime.
- `cdylib`: dynamic library.
- `stdio`: child process JSON-RPC over stdin/stdout.
- `http`: JSON-RPC over HTTP POST.
- `wasm`: WebAssembly module.

Plugin manifest defines:

- schema/version/name/authors.
- supported transports.
- hook subscriptions.
- tools.
- input path/network declarations.
- tags/search terms.
- loading, plan-mode, and streaming policy.
- host capabilities.
- UI contributions split into TUI (`ui.tui`) and Studio (`ui.studio`) surfaces.

Core registers runtime-provided static plugins during runtime build, including:

- filesystem entries.
- shell entries.
- web entries.
- workflow entries.
- skills filesystem.
- LSP.
- cron/scheduler.
- memory.
- MCP, when configured.
- settings config editor.

Plugin host can invoke hooks for:

- tool before/after/failure.
- tool invoke and streaming tool invoke.
- chat message/params/headers/system transform.
- auth and provider list.
- `permission.ask_permission` hook.
- command before/after and shell env.
- config notification.
- session start/end. Runtime provider prompts are kept append-only for cache stability.
- user prompt submit.
- agent stop.
- pre_run/post_run.
- notification.

Host callbacks allow plugins to ask user input, spawn subagents, list/invoke entries, read and reload config, publish/subscribe events, use scheduler, manage worktrees, access LSP/MCP registries, store plugin data/secrets, register entries/agents/hooks/themes/statusline segments, and more. Static UI contributions live in the manifest, while dynamic statusline/theme updates still flow through host callbacks and are merged into the same UI catalog.

## Permission architecture

Permission policy has three major surfaces:

- path access: workspace/external defaults and path pattern rules.
- network access: internet/private/loopback defaults and target rules.
- tool access: tags, tool names, and tool-specific rules.

Static config produces base policy. During runtime:

1. ToolExecutor derives the requested action.
2. Permission runtime checks persisted permission rules.
3. Plugin `permission.ask_permission` hooks can decide or advise.
4. Static policy allows, denies, or creates a pending permission request.
5. Pending request appears in session execution state and UI/API.
6. User reply can apply once or persist a session/workspace/global rule.

Persisted rules live in the database and can be managed through `/api/v1/permission-rules` or CLI permission commands.

## API architecture

`crates/agena-api` defines shared protocol types:

- `commands`: side-effectful operations.
- `queries`: read-only operations.
- `resource`: REST/WS resource projections.
- `notifications`: server-to-client push messages.
- `subscribe`: subscription filters and IDs.
- `ws`: client/server WebSocket frames.
- `error`: stable API error envelope.

`crates/agena-api-server` wires those types to transports:

- REST JSON endpoints under `/api/v1`.
- session-specific SSE under `/api/v1/sessions/{session_id}/events/stream`.
- global notification SSE under `/api/v1/events/stream`.
- WebSocket multiplexing under `/api/v1/ws`.
- optional IPC when enabled.
- JSON-RPC app-server under the `jsonrpc` feature.

`dispatch.rs` maps typed commands/queries to core services. REST handlers sometimes call `ApiService` directly to preserve the JSON shapes already used by Studio Web.

详见 [后端 API](backend-api.md)。

## Studio architecture

Studio has three layers:

```text
packages/agena-studio-web
        |
        | HTTP REST + session SSE
        v
apps/agena-studio-server
        |
        | agena-api-server router + runtime state
        v
crates/agena runtime/session/event/db
```

`agena-studio-server` builds `AgenaRuntime` with:

- config request from `--config` and `--set`.
- workspace root from `--workspace-root` or current directory.
- database URL/path from CLI/env/default.
- optional UI password.
- optional UI static directory.
- optional CORS allowlist.

Studio Web reads plugin UI from the runtime/plugin API rather than from a separate front-end registry. `GET /api/v1/runtime` includes `operator.ui` for bootstrap, `GET /api/v1/plugins/ui` returns the same unified catalog on demand, and plugin command/control actions are executed through `/api/v1/plugins/{plugin_id}/ui/actions/{action_id}`.

UI auth is a Studio-level middleware:

- If `AGENA_STUDIO_UI_PASSWORD` is absent/empty, auth is disabled.
- If enabled, `/auth/session` issues a 12-hour session token.
- Token is accepted via `Authorization: Bearer <token>` or cookie.
- Unsafe cookie-auth requests enforce Origin checks when cross-origin cookie use is enabled.

## Desktop architecture

Desktop packaging lives in `apps/agena-studio-desktop`:

- `src-tauri`: standard WebView2/Wry build.
- `src-tauri-cef`: experimental CEF build.

Build scripts in `ops/agena-studio/desktop/` prepare the `agena-studio` backend sidecar and Studio Web assets. Desktop runtime connects the webview to the packaged backend.

## Database architecture

Database access uses SeaORM with SQLite by default.

Key areas:

- migrations: `crates/agena/src/db/migrate/`
- entities: `crates/agena/src/db/entities/`
- CRUD helpers: `crates/agena/src/db/crud/`
- event store: `crates/agena/src/db/sea_event_store.rs`
- session store/history: `crates/agena/src/session/store.rs` and `crates/agena/src/session/history/`

Default storage resolution:

```text
AGENA_DATABASE_URL
AGENA_DATABASE_PATH
~/.agena/agena.db
```

`StorageConfig::ensure_parent` creates parent directories for file-backed SQLite URLs.

## Background tasks

Runtime starts two long-running tasks:

- reload task: watches config-related paths and rebuilds snapshot when modified.
- janitor task: periodic maintenance for runtime/session resources.

Reload policy is controlled by:

```toml
[runtime.reload]
enabled = true
poll_interval_secs = 2
```

Janitor policy:

```toml
[runtime.janitor]
enabled = true
interval_secs = 30
```

`runtime.shutdown()` signals task control and broadcasts session end events to plugins when possible.

## Extension points

Use these extension points depending on what you need:

- New model backend: add provider implementation under `crates/agena/src/provider/` and materialize it in `config/registry.rs`, or provide a plugin provider through `provider.list`.
- New provided tool: implement plugin tool under `crates/agena/src/plugins/provided/` or an internal tool module and register it in plugin host build.
- External plugin tool/plugin: use `agena-plugin-sdk` and configure `[plugins.list.<id>]`.
- New API operation: add type to `crates/agena-api`, map it in `crates/agena-api-server/src/dispatch.rs`, and expose REST route if Studio/Web needs direct HTTP.
- New Studio UI feature: add API wrapper in `packages/agena-studio-web/src/agena/lib/agenaApi.ts`, then page/state/component code.
- New config field: add raw type, merge behavior, env/override if needed, resolved type, validation, and example config coverage across the integration/e2e suite.

## Implementation index

- CLI app entrypoint: `apps/agena-cli/src/main.rs`
- CLI command definitions: `crates/agena/src/cli.rs`
- Runtime builder/snapshot/reload: `crates/agena/src/runtime/`
- Config loader/schema: `crates/agena/src/config/`
- Session manager/processor/store: `crates/agena/src/session/`
- Event bus/store/publisher: `crates/agena/src/event/`
- Provider registry/providers: `crates/agena/src/provider/`
- Permission system: `crates/agena/src/permission/` and `crates/agena/src/agent/mod.rs`
- Plugin host/config/transport: `crates/agena-plugin-host/`
- Plugin SDK/manifest/hooks: `crates/agena-plugin-sdk/`
- API protocol: `crates/agena-api/`
- API transports: `crates/agena-api-server/`
- Rust client SDK: `crates/agena-client/`
- Studio server: `apps/agena-studio-server/`
- Studio Web client wrapper: `packages/agena-studio-web/src/agena/lib/agenaApi.ts`
- Desktop packaging: `apps/agena-studio-desktop/`
