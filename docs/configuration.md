# 配置说明

本文说明 Agena 的运行时配置、环境变量、CLI 覆盖、provider、权限、插件和相关服务参数。配置实现主要在 `crates/agena/src/config/`，示例文件为仓库根目录的 `config.example.json` 和 `config.full.json`。

## 配置文件

Agena 使用 JSON 配置文件。最小可用配置见仓库根目录的 `config.example.json`，完整功能示例见 `config.full.json`。

建议从最小配置开始：

```bash
mkdir -p ~/.agena
cp config.example.json ~/.agena/config.json
agena config validate
```

`config.example.json` 展示了最小启动面：

- `[tracing]`: 日志过滤。
- `[default]`: 默认 provider、adapter、model 和 agent。
- `[providers.<id>]`: 至少配置一个逻辑 provider，通常由 provider-local `auth` + 一个或多个 `adapters` 组成。
- `[runtime]`: runtime HTTP、retry、reload、cache、catalog 等行为参数。
- `[agents.<name>]`: 自定义 agent。
- `[permission]`: 路径、网络、tool 权限。

`config.full.json` 展示了更完整的功能面：

- telemetry。
- provider HTTP timeout、retry、stream replay。
- runtime reload、janitor、session cache。
- permission path/network/tool rules。
- `agena.memory` project instructions。
- plugin transport、restart、storage、marketplace 安装后的配置形态。
- provider model metadata，以及拆分后的 model thinking/speed modes。

这两个示例文件有解析测试，测试位置为 `crates/agena/tests/config_examples.rs`。

## 加载路径与优先级

配置加载入口是 `ConfigLoader`。实际默认路径如下：

1. 如果显式传入 `--config <path>`，使用该路径。
2. 否则如果设置 `AGENA_CONFIG`，使用该路径。
3. 否则使用 `~/.agena/config.json`。

缺失配置文件不是错误。没有文件时，Agena 仍会使用内置默认值、环境变量和 CLI 覆盖解析出配置。

合并优先级从低到高：

1. 内置默认值。
2. JSON 配置文件。
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
default.provider
default.adapter
default.model
default.agent
runtime.provider_http.timeout_secs
runtime.provider_http.connect_timeout_secs
runtime.request_retry.max_retries
runtime.request_retry.base_delay_ms
runtime.request_retry.max_delay_ms
runtime.stream_replay.max_retries_after_output
runtime.stream_replay.max_tracked_events
runtime.model_catalog.cache_max_age_secs
```

Provider 覆盖：

当前 CLI provider 覆盖只接受 canonical 路径。

```text
providers.<id>.default_model
providers.<id>.auth.base_url
providers.<id>.auth.protocol_paths.<adapter>
providers.<id>.auth.api_key
providers.<id>.auth.api_key_env
providers.<id>.enabled
```

示例：

```bash
agena \
  --set tracing.filter=debug \
  --set default.provider=openai \
  --set default.adapter=openai \
  --set default.model=gpt-4.1-mini \
  config resolve
```

## Merge 规则

配置层之间不是简单替换整个文件，而是按类型合并：

- 顶层可选 struct 通常按字段合并。
- map 通常按 key 合并。
- provider config 按字段合并，`auth` 按字段合并，`adapters`、`extra_headers`、`ai_gateway_headers`、`feature_flags` 以及 provider/adapter 的 `models` map 会按 key 扩展或覆盖。
- `plugins` 的 `enabled` 和 `timeouts` 会被 overlay 替换；非空 plugin list 会替换嵌套 plugin tools。
- MCP、LSP、web 和 memory 都作为 runtime-provided static plugin 的 `options` 解析。
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
AGENA_MODEL_CATALOG_CACHE_MAX_AGE_SECS
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
- 通过 `--set default.model=...`、`--set providers.<id>.default_model=...` 或 `--set providers.<id>.auth.api_key_env=...` 这类 canonical override 设置。

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
[default]
provider = "openai"
adapter = "openai"
model = "gpt-5"
agent = "build"

[runtime]
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

[runtime.model_catalog]
cache_max_age_secs = 604800
```

全局默认项集中放在 `[default]`。`provider` 是默认逻辑 provider，`adapter` 是默认协议路由，`model` 是 backend-visible model id，`agent` 是新 root session 的默认 agent；未配置时默认 agent 是 `build`。

Model catalog 按 model 管理元数据和本地模型覆盖，不再保存 default model。catalog key 是真实 model id，例如 `models."gpt-5"`，不是 `openai/gpt-5` 这种 provider/adapter 路由，也不再绑定某个 adapter。catalog model 定义支持一个纯展示用的 `origin` 字段，用来标记模型来源/厂商，便于 UI 分类；它不参与任何 provider/adapter/model 路由或能力推断。Agena 会在运行时优先从公开 online sources 拉 richer metadata，目前包括 `https://models.dev/api.json`、`https://raw.githubusercontent.com/openai/codex/main/codex-rs/models-manager/models.json` 和 `https://models.router-for.me/models.json`，再叠加 live provider model lists 做 canonicalize / 去重 / origin 推断，最后把整理后的 official catalog 和本地 custom overrides 存到运行时数据库中。公开 sources 会按优先级合并，当前顺序是 `models.dev` > `openai/codex models.json` > `router-for.me`，低优先级 source 只补缺，不会覆盖更高优先级 source 已经给出的 speed/thinking patch。`cache_max_age_secs` 控制 official catalog 的刷新过期时间，默认 7 天；可以通过 Runtime Overview 或 API 手动刷新。旧的 workspace `.agena/catalog/model-catalog-cache.json` 和 `.agena/catalog/model-catalog-custom.json` 会在数据库为空时迁移一次。如果需要只依赖 live provider model lists，可以设置环境变量 `AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES=1`。

运行时 provider 会在 live model 列表里返回独立的 `adapter_id` 和真实 `model_id`，不会把二者拼成 provider-local route。`providers.<id>.default_adapter` 和 `providers.<id>.default_model` 可用作 provider 内部默认选择；如果该 provider 正好是 `default.provider` 且省略了 provider-local 默认值，解析器会分别使用 `default.adapter` 和 `default.model`。Studio Runtime Overview 页面可以刷新 catalog、从 live provider model 带入草稿、保存/删除 model-level 本地 override。Studio Settings / Providers 页面可以创建 provider，查看 provider 已启用 adapter 和 live models，把任意 catalog model 复制到某个 provider 的目标 adapter 下，也可以实时手动添加或修改 provider-local adapter model。

Provider 的 live `/models` 列表是实时请求，不做磁盘缓存，也不会在失败时 fallback 到旧结果。请求失败会直接返回错误。

校验规则：

- provider HTTP timeout 和 connect timeout 必须大于 0。
- reload poll interval 必须大于 0。
- janitor interval 必须大于 0。
- session cache TTL、max sessions、max bytes 必须大于 0。
- model catalog cache max age 必须大于 0。
- `runtime.request_retry.max_delay_ms` 会至少等于 `base_delay_ms`。

Runtime 会根据配置构建 snapshot。手动 reload 或配置文件变更触发 reload 时，新的 snapshot 会重新构建 provider registry、plugin host、agent registry、MCP/LSP registry 等服务。

## Providers

Provider 定义在 `[providers.<id>]`。当前 canonical 结构是 `provider + auth + adapters + models`。更完整的架构说明见 [Provider / Auth / Adapter 架构](provider-auth-adapters.md)。

最小示例：

```toml
[default]
provider = "openai"
adapter = "openai"
model = "gpt-5"
agent = "build"

[providers.openai]
enabled = true

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5"]
enabled = true
```

这四层的职责是：

- `provider`：逻辑入口，对外暴露 `provider_id` 和 provider 默认模型。
- `auth`：认证与身份来源，只负责 token / OAuth / ADC / SigV4 / service key。
- `adapter`：协议实现，例如 `openai`、`anthropic`、`gemini`、`gitlab`、`amazon_bedrock`、`ollama`。
- `model`：真实上游模型节点，key 就是上游 model id，本身支持 metadata/capabilities/thinking_modes/speed_modes patch。

关键规则：

- 全局默认 provider/adapter/model/agent 写在 `[default]`。
- `providers.<id>.default_adapter` 和 `providers.<id>.default_model` 是 provider-local 默认选择；`default_model` 必须是真实上游 model id。
- adapter 不再有 `default_model`。
- model key 就是真实 model id，不再有 `target_model`。
- `enabled` 可挂在 provider / adapter / model 三层。
- 运行时模型选择由 `provider_id`、`adapter_id`、`model_id` 三个字段共同决定，不使用三段字符串编码。

默认值：

- provider：默认 `enabled = true`
- adapter：默认 `enabled = false`
- model：默认 `enabled = true`

因此生产配置里建议把实际要启用的 adapter 明确写成 `enabled = true`。

`provider.auth.mode` 可选值：

```text
none
api
credential
bedrock_sigv4
google_adc
sap_ai_core
```

常用字段：

- `api`：`base_url`、`protocol_paths`、`api_key`、`api_key_env`
- `credential`：`issuer`、`credential`
- `google_adc`：`base_url`、`protocol_paths`
- `bedrock_sigv4`：`base_url`、`region`、`profile`、`access_key_id`、`secret_access_key`、`session_token`
- `sap_ai_core`：`base_url`、`api_key`、`api_key_env`、`service_key_env`

adapter 常见额外字段：

- 通用：`model_discovery`，默认 `live`；设为 `configured_only` 时不调用远程模型列表，只展示该 adapter 下显式配置的 models。
- `openai`：`backend`、`api_mode`、`stream_mode`、`models_url`、`realtime_ws_url`、`auth_header`、`auth_scheme`、`capability_family`、`user_agent`、`extra_headers`
- `anthropic`：`models_url`、`messages_url`、`auth_header`、`auth_scheme`、`extra_beta_header`、`eager_input_streaming`、`user_agent`、`extra_headers`
- `gemini`：`auth_header`、`auth_scheme`、`stream_mode`、`realtime_ws_url`、`user_agent`、`extra_headers`
- `gitlab`：`instance_url`、`ai_gateway_url`、`ai_gateway_headers`、`feature_flags`
- `ollama`：`base_url`

HTTP adapter 的 `user_agent` 会覆盖该 adapter 根据 auth credential 优先、
adapter 协议兜底推导出的默认 User-Agent；其他自定义 header 继续通过
`extra_headers` 配置。当前内置 credential 默认包括 AtomGit -> AtomCode、
OpenAI ChatGPT -> Codex、Google ADC -> Gemini CLI；没有专属身份的 auth
再按 adapter 使用 Codex / Claude Code API / Gemini CLI 风格的默认值。内置
默认值使用固定的官方产品版本字符串，不使用当前 agena 二进制名称或版本。

关于 Anthropic 适配器的认证约束：

- `auth.mode = "api"` 是 Agena 面向 Anthropic 官方一方接口的标准方式，使用 Claude Console API Key。
- `auth.mode = "credential"` 目前只用于 `issuer = "github_copilot"` 的兼容路径。
- Agena 不提供 Claude.ai / Claude Code 订阅 OAuth 登录。对第三方工具场景，官方当前文档要求使用 Claude Console API Key 或受支持的云提供商认证。

常见示例：

```toml
[providers.openai_chatgpt]
default_adapter = "openai"
default_model = "gpt-5.3-codex"

[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "...", access = "...", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai_chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"

[providers.openai_chatgpt.adapters.openai.models."gpt-5.3-codex"]
enabled = true
```

```toml
[providers."github-copilot"]
default_adapter = "openai"
default_model = "gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "...", access = "...", expires_at_ms = 4102444800000 }

[providers."github-copilot".adapters.openai]
enabled = true

[providers."github-copilot".adapters.openai.models."gpt-4o-mini"]
enabled = true
```

```toml
[providers.atomgit]
default_adapter = "openai"
default_model = "Kimi-K2-Instruct"

[providers.atomgit.auth]
mode = "credential"
issuer = "atomgit"
credential = { type = "oauth", issuer = "atomgit", refresh = "...", access = "...", expires_at_ms = 4102444800000, account_id = "atomgit-user" }

[providers.atomgit.adapters.openai]
enabled = true

[providers.atomgit.adapters.openai.models."Kimi-K2-Instruct"]
enabled = true
```

```toml
[providers.shared]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.auth.protocol_paths]
openai = "/v1"
anthropic = "/v1"
gemini = "/v1beta"

[providers.shared.adapters.openai]
enabled = true

[providers.shared.adapters.openai.models."gpt-4.1-mini"]
enabled = true

[providers.shared.adapters.anthropic]
enabled = true

[providers.shared.adapters.anthropic.models."claude-sonnet-4"]
enabled = true
```

当一个 auth 网关同时提供多种协议时，`base_url` 表示共享根路径，`auth.protocol_paths` 显式指定每个 adapter 的协议前缀。默认值是：

- `openai = "/v1"`
- `anthropic = "/v1"`
- `gemini = "/v1beta"`

OpenCode Go / Zen 也是这类共享网关：Go 大多数模型走 OpenAI-compatible `/chat/completions`，MiniMax 模型走 Anthropic Messages `/messages`；Zen 还包含 OpenAI Responses 和 Gemini 路由。可复制配置见 [OpenCode 接入](opencode-go.md)。

### Model metadata 和 modes

canonical 路径是 `providers.<id>.adapters.<adapter>.models."<real-model-id>"`。

示例：

```toml
[providers.openai.adapters.openai.models."gpt-4.1-mini"]
lifecycle = "active"
context_window_tokens = 200000
max_output_tokens = 16384
description = "Fast general-purpose model."
input = { supported = ["text", "image"], unsupported = ["audio"] }
features = { supported = ["tool_calling", "streaming"], unsupported = ["temperature"] }

[providers.openai.adapters.openai.models."gpt-4.1-mini".thinking_modes.light]
display_name = "Light"
thinking = { type = "effort", effort = "low" }

[providers.openai.adapters.openai.models."gpt-4.1-mini".thinking_modes.deep]
display_name = "Deep"
thinking = { type = "effort", effort = "high" }

[providers.openai.adapters.openai.models."gpt-4.1-mini".speed_modes.fast]
display_name = "Fast"
request_override = { body_patch = { service_tier = "priority" } }
```

模型节点本身建议只放会影响行为或能力元数据的字段；真正参与路由的是 provider、adapter、model 三级 id。`display_name` 不再作为 model 节点配置字段；mode 的 `display_name` 只用于展示。

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

`lifecycle` 可选值：

```text
active
preview
beta
alpha
experimental
deprecated
```

每个 model id 可以定义两套 mode：

- `thinking_modes`：控制 reasoning effort / budget / disabled
- `speed_modes`：控制请求级 patch，例如 headers、body patch、adapter-specific overrides

`thinking_modes.<name>` 字段包括 `display_name`、`description`、`thinking`、`disabled`。

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

`speed_modes.<name>` 字段包括：

- `display_name`
- `description`
- `disabled`
- `request_override.headers`
- `request_override.body_patch`
- `adapter_overrides.<adapter>.headers`
- `adapter_overrides.<adapter>.body_patch`

示例：

```toml
[providers.openai.adapters.openai.models."gpt-4.1-mini".speed_modes.fast]
display_name = "Fast"
description = "Prefer priority service tier"
request_override = { body_patch = { service_tier = "priority" } }

[providers.openai.adapters.openai.models."gpt-4.1-mini".speed_modes.fast.adapter_overrides.openai]
headers = { openai-beta = "fast-mode-2026-02-01" }
```

## Agents

Agent 可通过 JSON 配置，也可通过 `.agena/agents/*.md` 和 `~/.agena/agents/*.md` 发现。

TOML 示例：

```toml
[agents.plan]
description = "Read-only planning agent"
prompt = "You are a planning agent..."
allowed_entries = ["fs", "shell", "todo", "plan"]
mode = "all"
model = "anthropic/claude-sonnet-4-6"
aliases = ["planner"]
```

Markdown frontmatter 示例：

```markdown
---
description: "Read-only planning agent"
mode: "all"
allowed_entries: ["fs", "shell", "todo", "plan"]
model: "anthropic/claude-sonnet-4-6"
aliases: ["planner"]
permission:
  path:
    workspace:
      read: allow
      write: deny
  entries:
    names:
      shell: ask
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

`allowed_entries` 会收窄 agent 能调用的 entries 集合，同时保留已有 `shell` command pattern 规则。省略或写空数组表示不额外收窄。

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
plan = "allow"
todo = "allow"
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

### Tool permission

```toml
[permission.entries.tags]
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"

[permission.entries.names]
shell = "ask"
fs_edit = "ask"
"my-plugin.echo" = "ask"

[permission.entries.rules]
"my-plugin.echo" = "ask"

[permission.entries.rules."my-plugin.echo"]
"*" = "ask"

[permission.entries.rules.shell]
"git status" = "allow"
"git push *" = "deny"
"*" = "ask"
```

`tags` 用于 tool 没有精确规则时的默认策略。Tool 由 plugin manifest 声明自己的 tags，常见 tags 如 `filesystem_read`、`filesystem_write`、`network`、`internet`、`task`、`shell`。`names` 按 tool 名匹配；runtime-provided and user-configured plugin tools 使用同一个名字表。

`rules.<tool>` 可以直接写 mode，也可以写 pattern table。`shell` 的 pattern table 按实际 shell command 覆盖，`"*"` 是 fallback。其他 tool 使用直接 mode；需要 fallback 时也可以写 `rules.<tool>."*" = "ask"`。

## Memory

```toml
[plugins.list."agena.memory"]
kind = "static"

[plugins.list."agena.memory".options.project_instructions]
enabled = true
include_global = true
```

默认两项都为 true。该配置会影响项目指令/记忆是否进入上下文。Memory 配置属于 `agena.memory` static plugin options。

## Removed: `agena.hooks`

`agena.hooks` 这个配置驱动的 shell/HTTP hook bridge 已移除。旧的两种写法都会报配置错误：

- 顶层 `hooks`
- `plugins.list."agena.hooks"`

如果还需要 turn、tool、provider 或 permission 相关 hook 行为，请改成常规 plugin，在 manifest 中声明对应 `hooks` 订阅并实现 plugin SDK 的 hook 接口。

## Plugins

Plugin 是 Agena 的统一能力入口。模型可见 entries、MCP 暴露能力、LSP、skills、memory 等都会通过 plugin 或 plugin tool 接入 runtime。完整体系说明见 [Plugin 体系](plugin.md)。

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
- `tool_presentation`: 控制模型请求里的 tool 说明是完整发送，还是只发送短说明并引导调用 `tools help`。

Tool presentation 支持全局、按 plugin、按 tool 覆盖。模式值：

- `detailed`: 使用 tool manifest / `tool.definition` hook 给出的完整 `description`。
- `help`: 只发送短说明和 help 引导，完整用法通过 `tools` tool 的 `help` 子命令读取。

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

按 tool 覆盖可以使用模型可见名（如 `fs`）、`plugin_id/tool_name`（如 `agena.workflow/tools`），或无冲突的原始 tool 名。具体 tool 覆盖优先于 plugin 覆盖；plugin 覆盖优先于 manifest 的 `description_mode`；最后才使用 `default_mode`。

Plugin transport kind：

- `static`: 编译期注册的 runtime-provided static 插件。
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

`options` 是传给 plugin 的自由 JSON 配置；runtime-provided static plugin 也通过 `options` 接收自己的配置。

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

Runtime-provided static plugins 由 runtime 注册，包括文件系统、shell、web、workflow、skills、LSP、cron、memory、MCP、settings 等。它们和用户配置的 plugin 一样进入 plugin host 与 tool registry。

插件存储默认目录是 `~/.agena/plugin-storage`，可通过 `AGENA_PLUGIN_STORAGE_DIR` 覆盖。插件 secret 默认使用 `agena.plugin` keyring service，并可 fallback 到文件。

## MCP

MCP server 配置在 `agena.mcp` static plugin options。
Runtime 会把配置后的 MCP servers 通过 `agena.mcp` static plugin 暴露成 plugin tools，并统一进入 plugin host 和 tool registry。

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

WebSocket:

```toml
[plugins.list."agena.mcp".options.servers.browser]
transport = "ws"
url = "wss://mcp.example.com/socket"
headers = { }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
```

MCP server transport：

```text
stdio
http
ws
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

`ws` 字段：

```text
url
headers
auth
```

HTTP mode:

- `streamable_http`
- `sse`

`mode` 省略时使用 `streamable_http`。`headers` 是普通 header map，`auth` 可以省略。`streamable_http` 会自动尝试打开可选的 GET/SSE server-events 通道；如果服务端返回 `404` 或 `405`，runtime 会回退到仅使用 POST/response 路径。

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

LSP registry 是 lazy-spawn 的。相关 tool 首次触及匹配文件时才会启动对应 server。

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

`fetch_enabled` 控制 `web` 的 `fetch` command。Search backend 省略时使用 `duck_duck_go_html`。

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
  --config ~/.agena/config.json
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
