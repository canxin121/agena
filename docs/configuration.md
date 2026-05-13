# 配置说明

本文说明 Agena 的运行时配置、环境变量、CLI 覆盖、provider、权限、插件和相关服务参数。配置实现主要在 `crates/agena/src/config/`，示例文件为仓库根目录的 `config.example.toml` 和 `config.full.toml`。

## 配置文件

Agena 使用 TOML 配置文件。最小可用配置见仓库根目录的 `config.example.toml`，完整功能示例见 `config.full.toml`。

建议从最小配置开始：

```bash
mkdir -p ~/.agena
cp config.example.toml ~/.agena/config.toml
agena config validate
```

`config.example.toml` 展示了最小启动面：

- `[tracing]`: 日志过滤。
- `[auth]`: 凭据存储方式。
- `[providers.<id>]`: 至少配置一个模型 provider。
- `[runtime]`: 默认 agent。
- `[agents.<name>]`: 自定义 agent。
- `[permission]`: 路径、网络、entry 权限。

`config.full.toml` 展示了更完整的功能面：

- telemetry。
- provider HTTP timeout、retry、stream replay。
- runtime reload、janitor、session cache。
- permission path/network/entry rules。
- `agena.memory` project instructions。
- `agena.hooks` shell/HTTP hooks。
- plugin transport、restart、storage、marketplace 安装后的配置形态。
- provider model metadata 和 model variants。

这两个示例文件有解析测试，测试位置为 `crates/agena/tests/config_examples.rs`。

## 加载路径与优先级

配置加载入口是 `ConfigLoader`。实际默认路径如下：

1. 如果显式传入 `--config <path>`，使用该路径。
2. 否则如果设置 `AGENA_CONFIG`，使用该路径。
3. 否则使用 `~/.agena/config.toml`。

缺失配置文件不是错误。没有文件时，Agena 仍会使用内置默认值、环境变量和 CLI 覆盖解析出配置。

合并优先级从低到高：

1. 内置默认值。
2. TOML 配置文件。
3. 环境变量 overlay。
4. CLI 全局 `--set key=value` 覆盖。

配置始终解析为单个生效快照。

## 查看与验证配置

解析并输出最终配置：

```bash
agena config resolve --format toml
agena config resolve --format json
```

只验证配置是否可加载：

```bash
agena config validate
```

诊断命令会输出配置路径、是否找到配置文件、应用层级、provider 数量、plugin 数量和相关环境变量是否设置：

```bash
agena diagnostics
```

## CLI 覆盖

`agena` 主 CLI 支持全局 `--set key=value`，解析逻辑在 `crates/agena/src/config/overrides.rs`。

通用覆盖：

```text
auth.store_path
auth.store_backend
tracing.filter
tracing.database_level
ui.locale
runtime.provider_http.timeout_secs
runtime.provider_http.connect_timeout_secs
runtime.request_retry.max_retries
runtime.request_retry.base_delay_ms
runtime.request_retry.max_delay_ms
runtime.stream_replay.max_retries_after_output
runtime.stream_replay.max_tracked_events
```

Provider 覆盖：

```text
providers.<id>.default_model
providers.<id>.base_url
providers.<id>.api_key
providers.<id>.api_key_env
providers.<id>.enabled
```

示例：

```bash
agena \
  --set tracing.filter=debug \
  --set providers.openai.default_model=gpt-4.1-mini \
  config resolve
```

## Merge 规则

配置层之间不是简单替换整个文件，而是按类型合并：

- 顶层可选 struct 通常按字段合并。
- map 通常按 key 合并。
- provider config 按字段合并，`extra_headers`、`ai_gateway_headers`、`feature_flags`、`models` 会按 key 扩展或覆盖。
- `plugins` 的 `enabled` 和 `timeouts` 会被 overlay 替换；非空 plugin list 会替换嵌套 plugin entries。
- MCP、LSP、web、memory 和 hooks 都作为 first-party static plugin 的 `options` 解析。
- static plugin options 的合并语义跟随对应 plugin 的配置结构，例如 server map 按名称合并，web options 整体替换。

这些规则由 `crates/agena/src/config/raw.rs` 中的 `Merge` 实现定义。

## 环境变量

### 配置加载与核心 overlay

```text
AGENA_CONFIG
AGENA_AUTH_FILE
AGENA_LOG
AGENA_DATABASE_LOG
AGENA_TELEMETRY_ENABLED
AGENA_OTEL_SERVICE_NAME
AGENA_OTEL_ENDPOINT
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT
AGENA_LOCALE
AGENA_PLUGIN_ENABLED
AGENA_PROVIDER_HTTP_TIMEOUT_SECS
AGENA_PROVIDER_CONNECT_TIMEOUT_SECS
AGENA_PROVIDER_REQUEST_MAX_RETRIES
AGENA_PROVIDER_RETRY_BASE_DELAY_MS
AGENA_PROVIDER_RETRY_MAX_DELAY_MS
AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES
AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS
```

插件通过 `[plugins.list.<id>]` 显式配置，插件存储和 marketplace cache 可以通过上面的环境变量改写。

### Provider-specific overlay

Provider 环境变量使用双下划线分段：

```text
AGENA_PROVIDER__<PROVIDER_ID>__<FIELD>=...
```

`<PROVIDER_ID>` 会规范化为小写，并把 `_` 转为 `-`。例如：

```bash
export AGENA_PROVIDER__OPENAI__DEFAULT_MODEL=gpt-4.1-mini
export AGENA_PROVIDER__GOOGLE_VERTEX__KIND=google_vertex
```

支持字段：

```text
ENABLED
KIND
DEFAULT_MODEL
BASE_URL
API_KEY
API_KEY_ENV
AUTH_HEADER
AUTH_SCHEME
STREAM_MODE
API_MODE
REALTIME_WS_URL
AUTH_PROVIDER_ID
INSTANCE_URL
AI_GATEWAY_URL
MODELS_URL
REGION
PROFILE
ACCESS_TOKEN
ACCESS_TOKEN_ENV
ACCESS_KEY_ID
SECRET_ACCESS_KEY
SESSION_TOKEN
```

### 数据库、Studio、TUI

数据库路径由 `StorageConfig` 从 CLI/env 读取：

```text
AGENA_DATABASE_URL
AGENA_DATABASE_PATH
```

默认数据库路径为 `~/.agena/agena.db`，最终 SQLite URL 形如：

```text
sqlite://~/.agena/agena.db?mode=rwc
```

Studio server 参数：

```text
AGENA_STUDIO_HOST
AGENA_STUDIO_PORT
AGENA_STUDIO_UI_PASSWORD
AGENA_WORKSPACE_ROOT
AGENA_DATABASE_URL
AGENA_DATABASE_PATH
AGENA_STUDIO_UI_DIR
AGENA_STUDIO_CORS_ORIGINS
AGENA_STUDIO_CORS_ALLOW_ALL
AGENA_STUDIO_UI_COOKIE_SAMESITE
```

TUI 参数：

```text
AGENA_TUI_LOG_FILE
AGENA_TUI_LOG_STDERR
AGENA_TUI_CONFIG
```

### 插件、marketplace、provider preset

```text
AGENA_PLUGIN_STORAGE_DIR
AGENA_MARKETPLACE_DIR
AGENA_PROVIDER_PRESETS_PATH
```

`AGENA_PLUGIN_STORAGE_DIR` 覆盖插件存储根目录。默认是 `~/.agena/plugin-storage`。

`AGENA_MARKETPLACE_DIR` 覆盖 marketplace cache。默认是 `~/.agena/marketplace`。

`AGENA_PROVIDER_PRESETS_PATH` 覆盖 provider preset cache 文件路径。`kind = "preset"` 会使用这个 cache 或从 models.dev 获取 provider 元数据，并额外注入一些内置 preset。

## Tracing

```toml
[tracing]
filter = "info"
database_level = "error"
```

默认值：

- `filter = "info"`
- `database_level = "error"`

`database_level` 可选：

```text
off
error
warn
info
debug
trace
```

它会独立应用到 `sqlx`、`sea_orm` 和 `sea_orm_migration`，避免数据库日志淹没主应用日志。

## Telemetry

```toml
[telemetry]
enabled = false
service_name = "agena"
otlp_endpoint = "http://127.0.0.1:4318/v1/traces"
headers = { }
```

`enabled` 默认 false。endpoint 也可通过 `AGENA_OTEL_ENDPOINT` 或 `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` 提供。

## Auth

```toml
[auth]
store_backend = "auto"
store_path = "~/.agena/auth.json"
```

`store_backend` 可选：

- `auto`: 优先 OS keyring，不可用时 fallback 到 file。
- `keyring`: 使用 OS keyring，不做 file fallback。
- `file`: 只使用 file store。

默认 auth 文件路径由 `AGENA_AUTH_FILE` 或 `~/.agena/auth.json` 决定。auth store 可保存 API key、OAuth token 和 well-known credential。

## Runtime

```toml
[runtime]
default_agent = "build"

[runtime.provider_http]
timeout_secs = 120
connect_timeout_secs = 15

[runtime.request_retry]
max_retries = 5
base_delay_ms = 250
max_delay_ms = 2000

[runtime.stream_replay]
max_retries_after_output = 5
max_tracked_events = 2048

[runtime.reload]
enabled = true
poll_interval_secs = 2

[runtime.janitor]
enabled = true
interval_secs = 30

[runtime.session_cache]
max_sessions = 128
ttl_secs = 900
max_bytes = 67108864
```

默认 agent 是 `build`。即使省略 `runtime.default_agent`，解析后的默认值也是 `build`。

校验规则：

- provider HTTP timeout 和 connect timeout 必须大于 0。
- reload poll interval 必须大于 0。
- janitor interval 必须大于 0。
- session cache TTL、max sessions、max bytes 必须大于 0。
- `runtime.request_retry.max_delay_ms` 会至少等于 `base_delay_ms`。

Runtime 会根据配置构建 snapshot。手动 reload 或配置文件变更触发 reload 时，新的 snapshot 会重新构建 provider registry、plugin host、agent registry、MCP/LSP registry 等服务。

## Providers

Provider 定义在 `[providers.<id>]`。

示例：

```toml
[providers.anthropic]
kind = "anthropic"
base_url = "https://api.anthropic.com/v1"
default_model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
```

Provider `kind`：

```text
preset
ollama
openai
openai_compatible
sap_ai_core
anthropic
gemini
codex
gitlab
copilot
amazon_bedrock
google_vertex
cloudflare_ai_gateway
```

常用字段：

```toml
enabled = true
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key = "..."
api_key_env = "OPENAI_API_KEY"
extra_headers = { }
api_mode = "responses"
stream_mode = "sse"
realtime_ws_url = "wss://..."
```

OpenAI 的 `api_mode`：

```text
responses
chat
auto
```

Stream transport：

```text
sse
realtime_websocket
```

Credential 解析顺序要按 provider 具体实现理解，但通常是：

1. 配置中的直接 secret，例如 `api_key`、`access_token`。
2. 配置中命名的 env，例如 `api_key_env`、`access_token_env`。
3. Auth store 中的 credential，例如 `agena login openai --browser` 写入的 OAuth token。
4. provider 特有 fallback，例如 Google Vertex ADC、Amazon Bedrock SigV4。

不要把真实 API key 提交到仓库。优先使用 `api_key_env` 或登录命令。

### Preset provider

`kind = "preset"` 会按 provider id 加载 provider preset。preset 数据来自 `AGENA_PROVIDER_PRESETS_PATH` 指定的 cache 或 models.dev；代码还内置了一些常见 preset，例如 `ollama`、`lmstudio`、`openrouter`、`deepseek`、`xai`、`groq`、`mistral`。

示例：

```toml
[providers.ollama]
kind = "preset"
default_model = "qwen3:14b"

[providers.openrouter]
kind = "preset"
```

### Model metadata 和 variants

模型元数据必须放在 provider 的 `models."<model-id>"` 下：

```toml
[providers.openai.models."gpt-4.1-mini"]
input = { unsupported = ["image"] }
features = ["tool_calling"]
display_name = "GPT-4.1 Mini"
family = "gpt"
lifecycle = "active"
context_window_tokens = 200000
max_output_tokens = 16384

[providers.openai.models."gpt-4.1-mini".variants.light]
display_name = "Light"
thinking = { type = "effort", effort = "low" }
```

每个 model id 可以定义自己的 variants。

## Agents

Agent 可通过 TOML 配置，也可通过 `.agena/agents/*.md` 和 `~/.agena/agents/*.md` 发现。

TOML 示例：

```toml
[agents.plan]
description = "Read-only planning agent"
prompt = "You are a planning agent..."
allowed_entries = ["read", "view_file", "glob", "grep", "bash", "todo_write"]
mode = "all"
model = "anthropic/claude-sonnet-4-6"
aliases = ["planner"]
```

字段：

- `description`
- `prompt`
- `mode`: `primary`、`subagent`、`all`
- `hidden`
- `color`
- `temperature`
- `max_output_tokens`
- `steps`
- `allowed_entries`
- `permission`
- `model`
- `aliases`
- `disabled`

`allowed_entries` 会收窄 agent 能调用的 entries 集合，同时保留已有 bash pattern 规则。

## Permissions

权限 mode 固定为：

```text
allow
ask
deny
```

顶层权限 schema：

```toml
[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "ask" }

[permission.network]
internet = "ask"
private = "deny"
loopback = "deny"

[permission.entries.tags]
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"
shell = "ask"
```

Agent 也可以有自己的权限：

```toml
[agents.plan.permission.path]
workspace = { read = "allow", write = "deny" }
external = { read = "ask", write = "ask" }

[agents.plan.permission.entries.names]
enter_plan_mode = "allow"
exit_plan_mode = "allow"
todo_write = "allow"
```

Agent permission 默认继承顶层 permission。可用 `inherit` 控制：

```toml
[agents.plan.permission.inherit]
path = true
network = true
entries = true
```

也可以写成 `inherit = false` 完全不继承。

### Path permission

```toml
[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "deny" }

[permission.path.rules]
"<cwd>/.env*" = { read = "ask", write = "deny" }
"<cwd>/secrets/**" = "deny"
"/tmp/allowed/**" = "read_write"
```

Path rule 支持 `{ read, write }` 或 shorthand：

```text
allow
ask
deny
none
read
ro
write
write_only
wo
read_write
rw
```

常用 path alias：

```text
<cwd>
<workspace>
<home>
<tmp>
```

Path rules 按插入顺序保存，权限匹配时后写规则优先。

### Network permission

```toml
[permission.network]
internet = "ask"
private = "deny"
loopback = "deny"

[permission.network.rules]
"github.com:443" = "allow"
"*.corp.local:443" = "ask"
"10.0.0.0/8:*" = "deny"
```

规则可以匹配 host、通配 host、CIDR 和端口。后写规则优先。

### Entry permission

```toml
[permission.entries.tags]
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"

[permission.entries.names]
bash = "ask"
apply_patch = "ask"
"my-plugin.echo" = "ask"

[permission.entries.rules.bash]
"git status" = "allow"
"git push *" = "deny"
"*" = "ask"
```

`tags` 用于 entry 没有精确规则时的默认策略。`names` 按 entry 名匹配；first-party static plugin entries 和外部 plugin entries 使用同一个名字表。`rules.bash` 支持命令 pattern 覆盖。

## Memory

```toml
[plugins.list."agena.memory"]
kind = "static"

[plugins.list."agena.memory".options.project_instructions]
enabled = true
include_global = true
```

默认两项都为 true。该配置会影响项目指令/记忆是否进入上下文。Memory 配置属于 `agena.memory` static plugin options。

## Hooks

Hooks 用 `agena.hooks` static plugin options 配置。每个 hook 可以运行本地 command 或调用 HTTP URL：

```toml
[plugins.list."agena.hooks"]
kind = "static"

[[plugins.list."agena.hooks".options.hooks]]
event = "user_prompt_submit"
command = "python3 .agena/hooks/enrich_prompt.py"
timeout_ms = 3000

[[plugins.list."agena.hooks".options.hooks]]
event = "post_tool_use"
url = "http://127.0.0.1:8080/agena-hook"
timeout_ms = 2000
```

支持事件：

```text
user_prompt_submit
pre_tool_use
post_tool_use
post_tool_use_failure
stop
session_start
session_end
notification
```

Hook command 会收到事件相关环境变量，例如：

```text
AGENA_HOOK_EVENT
AGENA_SESSION_ID
AGENA_PROMPT
AGENA_TOOL_NAME
AGENA_TOOL_INPUT
AGENA_ERROR
AGENA_NOTIFICATION_KIND
AGENA_NOTIFICATION_TITLE
AGENA_NOTIFICATION_MESSAGE
AGENA_VERSION
AGENA_CWD
```

如果同一个 hook 同时配置 `url` 和 `command`，实现会优先走 HTTP URL。

## Plugins

Plugin 是 Agena 的统一能力入口。模型可见 entries、MCP 暴露能力、LSP、skills、memory、hooks 等都会通过 plugin 或 plugin entry 接入 runtime。完整体系说明见 [Plugin 体系](plugin.md)。

```toml
[plugins]
enabled = true
timeouts = { tool_invoke = "60s", permission_ask = "10s" }

[plugins.list.echo]
kind = "stdio"
command = "node"
args = ["./plugins/echo/index.js"]
env = { LOG_LEVEL = "info" }
restart = { policy = "on-failure", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { uppercase = true }
```

顶层 `[plugins]` 字段：

- `enabled`
- `timeouts`
- `list`
- `default_quota`
- `quotas`
- `trusted_keys`

Plugin transport kind：

- `static`: 编译期注册的 first-party/static 插件。
- `cdylib`: 本地动态库。
- `stdio`: 子进程 JSON-RPC over stdin/stdout。
- `http`: 远端 JSON-RPC over POST。
- `wasm`: WebAssembly module。

Timeout 字符串支持：

```text
ms
s
m
h
```

Stdio restart policy：

- `never`
- `on-failure`
- `always`

默认是 `on-failure`、最小 backoff 1s、最大 backoff 30s、最多 5 次 retry。

HTTP plugin auth 支持：

```toml
auth = { kind = "none" }
auth = { kind = "bearer", token_env = "PLUGIN_TOKEN" }
auth = { kind = "basic", username = "user", password_env = "PLUGIN_PASSWORD" }
```

First-party static plugins 由 runtime 注册，包括文件系统、shell、web、workflow、skills、LSP、cron、memory、hooks、MCP 等。它们和外部 plugin 一样进入 plugin host 与 entry registry。

插件存储默认目录是 `~/.agena/plugin-storage`，可通过 `AGENA_PLUGIN_STORAGE_DIR` 覆盖。插件 secret 默认使用 `agena.plugin` keyring service，并可 fallback 到文件。

## MCP

MCP server 配置在 `agena.mcp` static plugin options。
Runtime 会把配置后的 MCP servers 通过 `agena.mcp` static plugin 暴露成 plugin entries，并统一进入 plugin host 和 entry registry。

Stdio:

```toml
[plugins.list."agena.mcp"]
kind = "static"

[plugins.list."agena.mcp".options.servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
env = { }
cwd = "."
```

HTTP:

```toml
[plugins.list."agena.mcp".options.servers.remote]
transport = "http"
url = "https://mcp.example.com"
mode = "streamable_http"
headers = { }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
```

HTTP mode:

- `streamable_http`
- `sse`

HTTP auth:

- `bearer`
- `bearer_from_env`
- `bearer_from_store`
- `custom`

配置了 MCP server 时，runtime 会构建 `McpConnectionManager`，并注册 MCP static plugin。

## LSP

LSP server 配置在 `agena.lsp` static plugin options：

```toml
[plugins.list."agena.lsp"]
kind = "static"

[plugins.list."agena.lsp".options.servers.rust]
command = "rust-analyzer"
args = []
env = {}
file_extensions = ["rs"]
root_markers = ["Cargo.toml"]
initialization_options = {}
```

LSP registry 是 lazy-spawn 的。相关 entry 首次触及匹配文件时才会启动对应 server。

## Web Plugin

```toml
[plugins.list."agena.web"]
kind = "static"

[plugins.list."agena.web".options]
fetch_enabled = true

[plugins.list."agena.web".options.search]
backend = "duck_duck_go_html"
tavily_api_key = "..."
exa_api_key = "..."
brave_api_key = "..."
```

Search backend：

- `tavily`
- `exa`
- `brave`
- `duck_duck_go_html`

API key 可写在配置里，也可由环境变量提供：

```text
TAVILY_API_KEY
EXA_API_KEY
BRAVE_API_KEY
```

## Studio 服务配置

Studio server 是 `agena-studio` 二进制，参数定义在 `apps/agena-studio-server/src/main.rs`。

常用启动：

```bash
agena-studio \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace-root "$PWD" \
  --config ~/.agena/config.toml
```

服务参数：

```text
--config / AGENA_CONFIG
--set key=value
--host / AGENA_STUDIO_HOST
--port / AGENA_STUDIO_PORT
--ui-password / AGENA_STUDIO_UI_PASSWORD
--workspace-root / AGENA_WORKSPACE_ROOT
--database-url / AGENA_DATABASE_URL
--database-path / AGENA_DATABASE_PATH
--ui-dir / AGENA_STUDIO_UI_DIR
--cors-origin / AGENA_STUDIO_CORS_ORIGINS
--cors-allow-all / AGENA_STUDIO_CORS_ALLOW_ALL
--ui-cookie-samesite / AGENA_STUDIO_UI_COOKIE_SAMESITE
```

`ui-cookie-samesite` 可选：

```text
auto
strict
lax
none
```

`auto` 默认 same-origin 使用 `Strict`，配置跨域 CORS 时切到 `None`。

## 实现索引

- Loader、默认路径、优先级: `crates/agena/src/config/loader.rs`
- Raw schema、env overlay、merge、validation: `crates/agena/src/config/raw.rs`
- Resolved schema 和默认值: `crates/agena/src/config/types.rs`
- CLI override parser: `crates/agena/src/config/overrides.rs`
- Provider preset: `crates/agena/src/config/provider_presets.rs`
- Provider registry materialization: `crates/agena/src/config/registry.rs`
- Auth store: `crates/agena/src/provider/auth/store.rs`
- Runtime builder/snapshot/reload: `crates/agena/src/runtime/`
- Permission schema/policy: `crates/agena/src/agent/mod.rs`、`crates/agena/src/permission/`
- Plugin config: `crates/agena-plugin-host/src/config.rs`
- Plugin storage: `crates/agena/src/plugins/storage.rs`
- Studio args: `apps/agena-studio-server/src/main.rs`
