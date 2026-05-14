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
- `[providers.<id>]`: 至少配置一个逻辑 provider，通常由 provider-local `auth` + 一个或多个 `adapters` 组成。
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

当前 CLI provider 覆盖只接受 canonical 路径。

```text
providers.<id>.default_model
providers.<id>.auth.base_url
providers.<id>.auth.api_key
providers.<id>.auth.api_key_env
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
- provider config 按字段合并，`auth` 按字段合并，`adapters`、`extra_headers`、`ai_gateway_headers`、`feature_flags` 以及 provider/adapter 的 `models` map 会按 key 扩展或覆盖。
- `plugins` 的 `enabled` 和 `timeouts` 会被 overlay 替换；非空 plugin list 会替换嵌套 plugin entries。
- MCP、LSP、web、memory 和 hooks 都作为 first-party static plugin 的 `options` 解析。
- static plugin options 的合并语义跟随对应 plugin 的配置结构，例如 server map 按名称合并，web options 整体替换。

这些规则由 `crates/agena/src/config/raw.rs` 中的 `Merge` 实现定义。

## 环境变量

### 配置加载与核心 overlay

```text
AGENA_CONFIG
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

环境变量和 CLI 覆盖中的布尔值支持：

```text
true
1
yes
false
0
no
```

### Provider-specific overlay

`AGENA_PROVIDER__...` 这一整套 provider 环境变量覆盖已经移除。

现在只支持两种方式：

- 在配置文件里显式写 canonical 结构。
- 通过 `--set providers.<id>.default_model=...` 或 `--set providers.<id>.auth.api_key_env=...` 这类 canonical override 设置。

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

### 插件、marketplace

```text
AGENA_PLUGIN_STORAGE_DIR
AGENA_MARKETPLACE_DIR
```

`AGENA_PLUGIN_STORAGE_DIR` 覆盖插件存储根目录。默认是 `~/.agena/plugin-storage`。

`AGENA_MARKETPLACE_DIR` 覆盖 marketplace cache。默认是 `~/.agena/marketplace`。

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

`enabled` 默认 false。`headers` 是发送到 OTLP endpoint 的 header map。Endpoint 也可通过 `AGENA_OTEL_ENDPOINT` 或 `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` 提供。

## Provider Auth

provider 凭据的 canonical 位置是 `[providers.<id>.auth]`。常见来源有：

- `api_key`
- `api_key_env`
- `credential`

其中 `credential` 主要用于 provider-local OAuth token。CLI 登录、REST 登录、token refresh 都会直接回写当前 provider 的 `auth`，不会再经过独立的全局 auth store，也不会跨 provider 共享认证状态。

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

Provider 定义在 `[providers.<id>]`。新的 canonical 结构是“逻辑 provider + 共享 auth + 一个或多个 adapters”：

```toml
[providers.anthropic]
enabled = true
default_model = "claude-sonnet-4-6"

[providers.anthropic.auth]
mode = "api"
base_url = "https://api.anthropic.com/v1"
api_key_env = "ANTHROPIC_API_KEY"

[providers.anthropic.adapters.anthropic]
default_model = "claude-sonnet-4-6"
```

语义拆分如下：

- `providers.<id>`: 逻辑 provider id。CLI、HTTP API、Studio 和 model ref 都引用它。
- `providers.<id>.default_model`: 这个 provider 对外暴露的默认模型。单 adapter provider 省略时会回退到首个 adapter 的 `default_model`；多 adapter provider 应该把它设成已经声明路由的可见 model id。
- `providers.<id>.auth`: provider 级认证配置，供这个 provider 下的全部 adapters 共享。
- `providers.<id>.adapters.<adapter_id>`: 一个真实后端 adapter，负责协议类型和 provider-specific 选项。
- `providers.<id>.adapters.<adapter_id>.default_model`: 该 adapter 的上游默认模型。

adapter key 现在就是 kind，本身不再需要额外的 `kind = "..."` 字段。一个 provider 下同 kind adapter 只保留一个；如果你需要另一套 auth/base_url，就应该拆成另一个 provider，而不是在同一个 provider 里堆两个同 kind adapter。

一旦声明了 `adapters`，就不要再把 `kind`、`base_url`、`api_key_env` 这类字段留在 provider 根节点；canonical 结构只有 `provider + auth + adapters`。

Adapter `kind`：

```text
ollama
openai
openai_compatible
anthropic
gemini
gitlab
amazon_bedrock
```

顶层常用字段：

```toml
enabled = true
default_model = "gpt-4.1-mini"
```

`provider.auth` 的 canonical 字段：

```toml
[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "..."
api_key_env = "OPENAI_API_KEY"
```

`mode` 可选值：

```text
none
api
credential
bedrock_sigv4
google_adc
sap_ai_core
```

字段说明：

- `base_url`: provider 级 HTTP endpoint。OpenAI、Anthropic、Gemini、OpenRouter、Vertex static token 这类 API auth provider 都放在这里。
- `api_key` / `api_key_env`: `api` 模式使用的静态 token 来源。
- `issuer`: `credential` 模式使用，声明这份 provider-local auth 属于谁，例如 `openai_chatgpt`、`github_copilot`、`gitlab`。
- `credential`: provider-local `AuthData`。主要用于 OAuth token 以及 provider 自己管理的 refresh metadata。
- `profile`、`access_key_id`、`secret_access_key`、`session_token`、`base_url`、`region`: `bedrock_sigv4` 模式使用。
- `service_key_env`: `sap_ai_core` 模式使用，默认 `AICORE_SERVICE_KEY`。

`auth` block 只接受 canonical 字段；旧别名已经移除。

除 `kind` 和 `default_model` 之外，不同 adapter 还有这些字段：

| kind | `providers.<id>.adapters.<adapter_id>` 额外字段 |
| --- | --- |
| `ollama` | `base_url` |
| `openai` | `backend`、`extra_headers`、`api_mode`、`stream_mode`、`realtime_ws_url`、`models_url`、`auth_header`、`auth_scheme`、`capability_family` |
| `openai_compatible` | `extra_headers`、`auth_header`、`auth_scheme`、`stream_mode`、`realtime_ws_url` |
| `anthropic` | `extra_headers`、`auth_header`、`auth_scheme` |
| `gemini` | `extra_headers` |
| `gitlab` | `instance_url`、`ai_gateway_url`、`ai_gateway_headers`、`feature_flags` |
| `amazon_bedrock` | 无 |

OpenAI 的 `backend`：

```text
api
chatgpt_codex
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

1. `api` 模式下，配置中的 `api_key`。
2. `api` 模式下，配置中命名的 `api_key_env`。
3. `credential` 模式下，配置中的 inline `credential`。
4. provider 特有 fallback，例如 Google Vertex ADC、Amazon Bedrock SigV4、SAP AI Core service key。

几个重要特殊规则：

- `ollama` 使用 `auth.mode = "none"`，不走 credential。
- Vertex/OpenAI-style Gemini provider 如果没有静态 token，可用 `auth.mode = "google_adc"`。
- Amazon Bedrock 的 OpenAI-compatible token endpoint 应该配置成 `openai_compatible` adapter + `auth.mode = "api"`；如果走 AWS 原生签名，再用 `amazon_bedrock` adapter + `auth.mode = "bedrock_sigv4"`。`access_key_id` 和 `secret_access_key` 必须成对出现。
- `github_copilot` credential + `openai` / `anthropic` adapter，以及 OpenAI `backend = "chatgpt_codex"`，都只支持 provider-local OAuth credential，不支持直接 `api_key` 或 `api_key_env`。
- GitLab 既支持 `auth.mode = "api"` 直接 token，也支持 `auth.mode = "credential"` 的 provider-local OAuth。
- `capability_family` 用来把官方 `openai` adapter 的能力/模型元数据判定切到其他家族；目前 canonical 用法是 `gemini`。如果上游本身是 OpenAI-compatible chat-completions 协议，请直接使用 `openai_compatible` adapter，而不是 `openai + capability_family = "openai_compatible"`。
更细的整体结构、auth 模式和 adapter 约束见 [Provider / Auth / Adapter 架构](provider-auth-adapters.md)。

`openai` adapter 默认使用 `backend = "api"`，此时 endpoint 通常来自 `provider.auth.base_url`，也可以在 adapter 上额外覆写 `base_url` / `models_url`。如果当前 provider 配置的是 `github_copilot` credential，运行时会自动切到 Copilot endpoint / header 语义；当你没有显式设置 `api_mode` 时，会继续按模型自动选择 `chat` 或 `responses` 路径，这条路径适合 Copilot 上的 GPT / Codex / Gemini 等 OpenAI-style 模型。如果 `backend = "chatgpt_codex"`，endpoint 固定为 `https://chatgpt.com/backend-api/codex`，并要求当前 provider 配置 provider-local OpenAI OAuth credential。

如果当前 provider 配置的是 `github_copilot` credential，`anthropic` adapter 也可以复用同一份 provider-local OAuth credential。此时运行时会保留 Copilot 的 bearer token / enterprise host / request headers 语义，但消息协议走 Anthropic `/v1/messages`。你也可以在 adapter 上显式写 `auth_header = "authorization"`、`auth_scheme = "Bearer"`、`messages_url`、`models_url`、`extra_beta_header` 等字段；适合 Copilot 上的 Claude 类模型，或者任何需要 Anthropic wire format 的自定义网关。

如果要走 ChatGPT Codex backend，配置成：

```toml
[providers.openai_chatgpt]
default_model = "gpt-5.3-codex"

[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai_chatgpt.adapters.openai]
backend = "chatgpt_codex"
default_model = "gpt-5.3-codex"
```

`chatgpt_codex` backend 默认使用 `https://chatgpt.com/backend-api/codex`，并要求当前 provider 配置 OpenAI OAuth credential；它不支持直接 `api_key`、`api_key_env`、`api_mode = "chat"`、`stream_mode = "realtime_websocket"` 或 `realtime_ws_url`。

如果要走 GitHub Copilot OAuth，也使用 `openai` adapter：

```toml
[providers."github-copilot"]
default_model = "gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000 }

[providers."github-copilot".adapters.openai]
default_model = "gpt-4o-mini"
# models_url = "https://api.githubcopilot.com/models"

[providers."github-copilot".adapters.openai.models."gpt-4o-mini"]
target_model = "gpt-4o-mini"
```

```toml
[providers."github-copilot".adapters.anthropic]
default_model = "claude-sonnet-4"

[providers."github-copilot".adapters.anthropic.models.claude]
target_model = "claude-sonnet-4"
```

不要把真实 API key 提交到仓库。优先使用 `api_key_env` 或登录命令。

Provider-specific 示例：

```toml
[providers.openrouter]
default_model = "openai/gpt-4.1-mini"

[providers.openrouter.auth]
mode = "api"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[providers.openrouter.adapters.openai_compatible]
default_model = "openai/gpt-4.1-mini"
auth_header = "authorization"
auth_scheme = "Bearer"

[providers.shared]
default_model = "fast"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.adapters.openai]
default_model = "gpt-4.1"

[providers.shared.adapters.openai.models.fast]
target_model = "gpt-4.1-mini"

[providers.shared.adapters.anthropic]
default_model = "claude-sonnet-4"

[providers.shared.adapters.anthropic.models.coder]
target_model = "claude-sonnet-4"

[providers.gitlab]
default_model = "claude-sonnet-4-5"

[providers.gitlab.auth]
mode = "credential"
issuer = "gitlab"
credential = { type = "oauth", issuer = "gitlab", refresh = "gitlab-refresh", access = "gitlab-access", expires_at_ms = 4102444800000 }

[providers.gitlab.adapters.gitlab]
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
ai_gateway_headers = { "X-GitLab-Feature" = "agena" }
feature_flags = { use_ai_gateway = true }

[providers.bedrock]
default_model = "anthropic.claude-3-5-sonnet-20240620-v1:0"

[providers.bedrock.auth]
mode = "bedrock_sigv4"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com"
region = "us-east-1"
profile = "prod"

[providers.bedrock.adapters.amazon_bedrock]
default_model = "anthropic.claude-3-5-sonnet-20240620-v1:0"

[providers.bedrock-openai]
default_model = "amazon.nova-pro-v1:0"

[providers.bedrock-openai.auth]
mode = "api"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"
api_key_env = "BEDROCK_API_KEY"

[providers.bedrock-openai.adapters.openai_compatible]
default_model = "amazon.nova-pro-v1:0"

[providers.sap]
default_model = "anthropic/claude-sonnet-4"

[providers.sap.auth]
mode = "sap_ai_core"
base_url = "https://api.example.com/v2"
service_key_env = "AICORE_SERVICE_KEY"

[providers.sap.adapters.openai_compatible]
default_model = "anthropic/claude-sonnet-4"

[providers.vertex]
default_model = "gemini-1.5-pro"

[providers.vertex.auth]
mode = "api"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google"
api_key_env = "GOOGLE_VERTEX_ACCESS_TOKEN"

[providers.vertex.adapters.openai]
default_model = "gemini-1.5-pro"
capability_family = "gemini"
```

多 adapter provider 的约束：

- 每个 adapter 都可以有自己的 `models` 路由表。
- 只要一个 provider 下有多个 adapters，就必须为每个 adapter 显式声明 `models`。
- 同一个可见 model id 不能在多个 adapters 下重复声明。
- 多 adapter provider 的 `default_model` 必须是已经声明过的可见 model id。

### Model metadata 和 variants

新的 canonical 路径是 `providers.<id>.adapters.<kind>.models."<visible-model-id>"`。其中：

- `<visible-model-id>` 是 Agena 对外暴露的模型名。
- `target_model` 是真实上游模型名；省略时默认和 `<visible-model-id>` 相同。
- metadata、capabilities 和 `variants` 都挂在这个 routed model 节点上。

示例：

```toml
[providers.openai.adapters.openai.models.fast]
target_model = "gpt-4.1-mini"
display_name = "Fast"
family = "gpt"
lifecycle = "active"
context_window_tokens = 200000
max_output_tokens = 16384
description = "Fast general-purpose model."
input = { supported = ["text", "image"], unsupported = ["audio"] }
features = { supported = ["tool_calling", "streaming"], unsupported = ["temperature"] }

[providers.openai.adapters.openai.models.fast.variants.light]
display_name = "Light"
thinking = { type = "effort", effort = "low" }

[providers.openai.adapters.openai.models.fast.variants.deep]
display_name = "Deep"
thinking = { type = "effort", effort = "high" }
```

`input` 和 `features` 都支持 compact array：

```toml
input = ["text", "image"]
features = ["tool_calling", "streaming"]
```

也可以显式区分 `supported` 和 `unsupported`：

```toml
input = { supported = ["text", "document"], unsupported = ["audio", "video"] }
features = { supported = ["reasoning"], unsupported = ["temperature"] }
```

`supported` 和 `unsupported` 都可以只写其中一个。同一个值不能同时出现在两边。

`input` 可选值：

```text
text
image
document
audio
video
file
```

`features` 可选值：

```text
tool_calling
streaming
reasoning
structured_output
temperature
```

`family` 可选值：

```text
gpt
codex
claude
gemini
llama
mistral
deepseek
qwen
nova
grok
phi
command
```

`lifecycle` 可选值：

```text
active
preview
beta
alpha
experimental
deprecated
```

每个 model id 可以定义自己的 variants。Variant 字段包括 `display_name`、`description`、`thinking`、`disabled`。

`thinking` 写法：

```toml
thinking = { type = "budget", budget_tokens = 4096 }
thinking = { type = "effort", effort = "medium" }
thinking = { type = "disabled" }
```

`effort` 可选：

```text
minimal
low
medium
high
xhigh
max
```

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

Markdown frontmatter 示例：

```markdown
---
description: "Read-only planning agent"
mode: "all"
allowed_entries: ["read", "view_file", "glob", "grep", "bash", "todo_write"]
model: "anthropic/claude-sonnet-4-6"
aliases: ["planner"]
permission:
  path:
    workspace:
      read: allow
      write: deny
  entries:
    names:
      bash: ask
---
You are a planning agent...
```

TOML agent 的 `prompt` 字段是 system prompt；Markdown agent 的正文是 system prompt。Markdown frontmatter 支持的字段和 TOML agent 基本一致，但不使用 `prompt` 和 `disabled`。

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

`mode` 可选：

```text
primary
subagent
all
```

`allowed_entries` 会收窄 agent 能调用的 entries 集合，同时保留已有 bash pattern 规则。省略或写空数组表示不额外收窄。

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
[agents.plan.permission]
inherit = false
```

`inherit` 也可以直接写成 inline table：

```toml
[agents.plan.permission]
inherit = { path = true, network = false, entries = true }
```

也可以按 section 控制：

```toml
[agents.plan.permission.inherit]
path = true
network = true
entries = true
```

`inherit = true` 表示继承所有 section，`inherit = false` 表示不继承。按 section 写表时，未写的 section 默认继承。

### Path permission

```toml
[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "deny" }

[permission.path.rules]
"<cwd>/.env*" = { read = "ask", write = "deny" }
"<cwd>/secrets/**" = "deny"
"/tmp/allowed/**" = "read_write"
"<home>/Downloads/*.txt" = "read"
```

`workspace`、`external` 和每条 path rule 都可以写 `{ read, write }`，其中 `read`、`write` 可以只写其中一个；path rule 还支持 shorthand：

```text
allow
ask
deny
none
read
read_only
ro
write
write_only
wo
read_write
rw
```

这些 shorthand 只用于 path rule。`read` / `read_only` / `ro` 表示读允许、写拒绝；`write` / `write_only` / `wo` 表示读拒绝、写允许；`read_write` / `rw` 表示读写都允许。Shorthand 大小写不敏感，`-` 会按 `_` 处理，例如 `read-write` 等价于 `read_write`。

常用 path alias：

```text
<cwd>
<workspace>
<home>
<tmp>
```

Path key 可以写 workspace 相对路径、绝对路径、alias 路径和 glob。Path rules 按插入顺序保存，权限匹配时后写规则优先。

### Network permission

```toml
[permission.network]
internet = "ask"
private = "deny"
loopback = "deny"

[permission.network.rules]
"github.com:443" = "allow"
"api.github.com" = "allow"
"*.corp.local" = "ask"
"*.corp.local:8443" = "allow"
"10.0.0.0/8" = "deny"
"172.16.0.0/12:*" = "ask"
"fd00::/8" = "deny"
"[::1]:3000" = "ask"
```

Network rule key 可以写成：

```text
*
*:port
host
host:port
host:*
host-with-*
*.domain
*.domain:port
*.domain:*
IPv4
IPv4:port
IPv4:*
CIDR
CIDR:port
CIDR:*
IPv6
IPv6 CIDR
[IPv6]
[IPv6]:port
[IPv6]:*
[IPv6 CIDR]
[IPv6 CIDR]:port
[IPv6 CIDR]:*
```

不写端口表示匹配任意端口，写 `:*` 也是匹配任意端口；写具体端口时只匹配该端口。Host pattern 支持 `*` 和 `?`，所以 `*.corp.local`、`api.*`、`db-??.internal` 都是有效写法。IPv6 地址需要匹配具体端口时使用 bracket 形式，例如 `[::1]:3000`；IPv6 地址或 CIDR 不写端口时直接写 `::1`、`fd00::/8`。URL 目标会解析出默认端口，例如 `https://github.com` 会按 `github.com:443` 匹配。后写规则优先。

Network target 会先分到三类默认策略：`loopback` 匹配 `localhost`、`*.localhost` 和 loopback IP；`private` 匹配私有 IP、link-local IP、单段主机名，以及 `.local`、`.lan`、`.internal`、`.corp`、`.home.arpa`；其余走 `internet`。`rules` 命中时优先于这些默认策略。

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

[permission.entries.rules]
"my-plugin.echo" = "ask"

[permission.entries.rules."my-plugin.echo"]
"*" = "ask"

[permission.entries.rules.bash]
"git status" = "allow"
"git push *" = "deny"
"*" = "ask"
```

`tags` 用于 entry 没有精确规则时的默认策略。Entry 由 plugin manifest 声明自己的 tags，常见 tags 如 `filesystem_read`、`filesystem_write`、`network`、`internet`、`task`、`shell`。`names` 按 entry 名匹配；first-party static plugin entries 和外部 plugin entries 使用同一个名字表。

`rules.<entry>` 可以直接写 mode，也可以写 pattern table。`bash` 的 pattern table 按命令 pattern 覆盖，`"*"` 是 fallback。其他 entry 使用直接 mode；需要 fallback 时也可以写 `rules.<entry>."*" = "ask"`。

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
event = "pre_tool_use"
command = "python3 .agena/hooks/check_tool.py"
matcher = { tool = "bash" }
timeout_ms = 5000

[[plugins.list."agena.hooks".options.hooks]]
event = "post_tool_use"
url = "http://127.0.0.1:8080/agena-hook"
timeout_ms = 2000
```

Hook 字段：

- `event`: hook 事件名。
- `command`: 本地 shell command。
- `url`: HTTP endpoint，hook input 会以 JSON POST 过去。
- `matcher.tool`: 只匹配指定 entry/tool 名，支持 glob。
- `timeout_ms`: 单次调用超时；省略时使用 30000。

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

以下事件名也可以写在 `event` 中：

| 等价写法 | 对应事件 |
| --- | --- |
| `tool_before` | `pre_tool_use` |
| `tool_after` | `post_tool_use` |
| `tool_failure` | `post_tool_use_failure` |
| `agent_stop` | `stop` |

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
timeouts = { init = "10s", tool_invoke = "60s", permission_ask = "10s", fast = "500ms" }

[plugins.default_quota]
rate_per_sec = 20
burst = 40
max_concurrent = 8

[plugins.quotas.echo]
rate_per_sec = 5
burst = 10
max_concurrent = 2

[plugins.trusted_keys]
acme = "0123456789abcdef..."

[plugins.list.echo]
kind = "stdio"
command = "node"
args = ["./plugins/echo/index.js"]
env = { LOG_LEVEL = "info" }
cwd = "."
sha256 = "..."
restart = { policy = "on-failure", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { uppercase = true }
timeouts = { tool_invoke = "30s" }
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

每种 transport 的字段：

```toml
[plugins.list."agena.memory"]
kind = "static"
timeouts = { init = "5s" }

[plugins.list."agena.memory".options.project_instructions]
enabled = true
include_global = true

[plugins.list.native]
kind = "cdylib"
path = "./plugins/native/libnative.so"
sha256 = "..."
signature = { key_id = "acme", signature = "..." }
options = { mode = "strict" }
timeouts = { tool_invoke = "20s" }

[plugins.list.worker]
kind = "stdio"
command = "node"
args = ["./plugins/worker/index.js"]
env = { LOG_LEVEL = "info" }
cwd = "."
sha256 = "..."
restart = { policy = "always", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { project = "rust" }
timeouts = { tool_invoke = "45s" }

[plugins.list.policy]
kind = "http"
url = "https://policy.example.com/agena/rpc"
auth = { kind = "bearer", token_env = "AGENA_POLICY_TOKEN" }
options = { org_id = "acme" }
timeouts = { fast = "2s" }

[plugins.list.sandboxed]
kind = "wasm"
path = "./plugins/sandboxed/plugin.wasm"
sha256 = "..."
options = { }
timeouts = { init = "20s" }
```

`options` 是传给 plugin 的自由 JSON/TOML 配置；first-party static plugin 也通过 `options` 接收自己的配置。

Timeout 字段：

```text
init
tool_hook
tool_invoke
permission_ask
chat
fast
```

Timeout 字符串支持：

```text
ms
s
m
h
```

Timeout 值在 TOML 中写字符串。不写单位时按秒解析，例如 `"30"` 等价于 `"30s"`。

Quota 字段：

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

`rate_per_sec = 0` 表示不限制速率；`burst = 0` 表示使用 `rate_per_sec`；`rate_per_sec` 和 `burst` 都为 0 时关闭 token bucket。`max_concurrent = 0` 表示不限制并发。

Stdio restart policy：

- `never`
- `on-failure`
- `always`

默认是 `on-failure`、最小 backoff 1s、最大 backoff 30s、最多 5 次 retry。

HTTP plugin auth 支持：

```toml
auth = { kind = "none" }
auth = { kind = "bearer", token = "..." }
auth = { kind = "bearer", token_env = "PLUGIN_TOKEN" }
auth = { kind = "basic", username = "user", password = "..." }
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

MCP server transport：

```text
stdio
http
```

`stdio` 字段：

```text
command
args
env
cwd
```

`http` 字段：

```text
url
mode
headers
auth
```

HTTP mode:

- `streamable_http`
- `sse`

`mode` 省略时使用 `streamable_http`。`headers` 是普通 header map，`auth` 可以省略。

HTTP auth:

```toml
auth = { kind = "bearer", token = "..." }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
auth = { kind = "bearer_from_store" }
auth = { kind = "custom", headers = { "X-Token" = "..." } }
```

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

LSP 字段：

```text
command
args
env
file_extensions
root_markers
initialization_options
```

`file_extensions` 不带前导 `.`；写空数组表示该 server 匹配所有文件。`root_markers` 是用于识别项目根目录的文件名列表。`initialization_options` 是传给 language server 的 JSON/TOML object。

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

`fetch_enabled` 控制 `web_fetch` entry。Search backend 省略时使用 `duck_duck_go_html`。

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

`duck_duck_go_html` 不需要 API key；`tavily` 使用 `tavily_api_key` 或 `TAVILY_API_KEY`，`exa` 使用 `exa_api_key` 或 `EXA_API_KEY`，`brave` 使用 `brave_api_key` 或 `BRAVE_API_KEY`。

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
- Provider registry materialization: `crates/agena/src/config/registry.rs`
- Auth store: `crates/agena/src/provider/auth/store.rs`
- Runtime builder/snapshot/reload: `crates/agena/src/runtime/`
- Permission schema/policy: `crates/agena/src/agent/mod.rs`、`crates/agena/src/permission/`
- Plugin config: `crates/agena-plugin-host/src/config.rs`
- Plugin storage: `crates/agena/src/plugins/storage.rs`
- Studio args: `apps/agena-studio-server/src/main.rs`
