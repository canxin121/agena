# Agena 配置文件参考

- **Language:** 简体中文
- **Alias:** `docs/config.zh-CN.md`

这份文档是 `agena` 运行时配置文件的完整参考，覆盖：

- 配置文件位置与加载优先级
- 所有顶层段、所有字段、所有当前代码可识别的枚举值
- provider / plugin / MCP / LSP / web / hooks / permission 的完整 schema
- 环境变量覆盖与 CLI `-c/--set` 覆盖
- 当前实现中的限制与容易误解的边界

如果你只想先跑起来，先看仓库根目录：

- `config.example.toml`：最小可运行配置
- `config.full.toml`：较完整的示例配置

注意：本文描述的是 **Agena runtime 配置**（默认 `~/.agena/config.toml`），不包括 `agena-studio` 桌面壳自己的 `agena-studio.toml`。

## 1. 基本规则

### 1.1 默认位置

- 默认配置文件路径：`~/.agena/config.toml`
- 可通过环境变量 `AGENA_CONFIG` 指定配置文件
- 也可通过 CLI `--config <path>` 指定配置文件

### 1.2 格式

- 配置格式是 `TOML`
- 顶层对象会被解析成强类型配置
- 未识别字段会在 TOML 解析阶段被忽略还是报错，取决于对应结构；本文只列出当前代码实际支持的字段

### 1.3 配置优先级

`agena` 的配置优先级是：

1. 内建默认值
2. 配置文件
3. 环境变量覆盖
4. CLI `--set/-c` 覆盖

也就是说，后者总是覆盖前者。

### 1.4 不再支持 modes

当前实现已经移除了配置 `mode` / `modes.<name>` 能力：

- 不再支持顶层 `mode = "..."`
- 不再支持 `[modes.<name>]`
- 不再支持环境变量 `AGENA_MODE`
- 不再支持 CLI `--mode`
- 不再支持 CLI `-c mode=<name>`

如果旧配置里还保留这些字段，加载时会直接报错。现在的建议做法是：

- 为不同环境维护不同的完整配置文件
- 或者在单个基础配置文件上叠加环境变量和 `--set` 覆盖

### 1.5 配置解析与检查命令

```bash
agena config resolve --format toml
agena config resolve --format json
agena config validate
```

## 2. 顶层结构总览

完整配置的逻辑结构如下：

```toml
[tracing]
filter = "info"
database_level = "error"

[telemetry]
enabled = false
service_name = "agena"
otlp_endpoint = "http://127.0.0.1:4318/v1/traces"

[telemetry.headers]
Authorization = "Bearer xxx"

[auth]
store_backend = "auto"
store_path = "~/.agena/auth.json"

[ui]
locale = "zh-CN"

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

[agents.plan]
description = "Read-only planning agent"
prompt = "You are a planning agent."
allowed_tools = ["read", "view_file", "glob", "grep", "bash", "todo_write", "enter_plan_mode", "exit_plan_mode"]
mode = "all"

[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "ask" }

[agents.plan.permission.path]
workspace = { read = "allow", write = "deny" }

[agents.plan.permission.tools.first_party]
enter_plan_mode = "allow"
exit_plan_mode = "allow"
todo_write = "allow"

[memory.project_instructions]
enabled = true
include_global = true

[[hooks]]
event = "user_prompt_submit"
command = "python3 .agena/hooks/enrich.py"
timeout_ms = 3000

[[hooks]]
event = "pre_tool_use"
url = "http://127.0.0.1:8080/hook"
matcher = { tool = "bash" }
timeout_ms = 5000

[plugins]
enabled = true
timeouts = { tool_invoke = "60s", permission_ask = "10s" }

[plugins.default_quota]
rate_per_sec = 0
burst = 0
max_concurrent = 0

[plugins.quotas.some_plugin]
rate_per_sec = 5
burst = 10
max_concurrent = 2

[plugins.trusted_keys]
release = "0123abcd..."

[plugins.list.local_static]
kind = "static"
options = { enabled = true }

[plugins.list.echo_cdylib]
kind = "cdylib"
path = "./plugins/libecho.so"
options = { uppercase = true }
timeouts = { tool_invoke = "30s" }
sha256 = "..."

[plugins.list.echo_cdylib.signature]
key_id = "release"
signature = "..."

[plugins.list.worker_stdio]
kind = "stdio"
command = "node"
args = ["./plugins/worker/index.js"]
env = { NODE_ENV = "production" }
cwd = "/abs/path/to/project"
restart = { policy = "on-failure", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { profile = "default" }
timeouts = { tool_invoke = "45s" }
sha256 = "..."

[plugins.list.remote_http]
kind = "http"
url = "https://plugins.example.com/rpc"
auth = { kind = "bearer", token_env = "PLUGIN_TOKEN" }
options = { org = "acme" }
timeouts = { chat = "5s" }

[plugins.list.sandboxed_wasm]
kind = "wasm"
path = "./plugins/tool.wasm"
options = { mode = "safe" }
timeouts = { tool_invoke = "20s" }
sha256 = "..."

[plugins.list.sandboxed_wasm.sandbox]
allow_fs_read = ["/repo"]
allow_fs_write = ["/tmp"]
allow_net = false
allow_env = ["HOME"]

[providers.openai]
kind = "openai"
enabled = true
base_url = "https://api.openai.com/v1"
default_model = "gpt-5"
api_key_env = "OPENAI_API_KEY"
api_mode = "responses"
stream_mode = "sse"
realtime_ws_url = "wss://..."
default_thinking = "standard"
thinking_depths = { light = 3000, standard = 10000, deep = 30000 }
extra_headers = { "X-Trace" = "1" }

[providers.openai.models."gpt-5"]
input = { unsupported = ["image"] }
features = ["tool_calling"]

[providers.compat]
kind = "openai_compatible"
base_url = "https://example.com/v1"
default_model = "my-model"
api_key_env = "COMPAT_API_KEY"
auth_header = "authorization"
auth_scheme = "Bearer"
stream_mode = "sse"
realtime_ws_url = "wss://..."

[providers.sap]
kind = "sap_ai_core"
base_url = "https://example.sap/v1"
default_model = "anthropic/claude-sonnet-4"
api_key_env = "AICORE_TOKEN"

[providers.anthropic]
kind = "anthropic"
base_url = "https://api.anthropic.com/v1"
default_model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
auth_header = "x-api-key"

[providers.gemini]
kind = "gemini"
base_url = "https://generativelanguage.googleapis.com/v1beta"
default_model = "gemini-2.5-pro"
api_key_env = "GEMINI_API_KEY"

[providers.ollama]
kind = "ollama"
base_url = "http://localhost:11434"
default_model = "qwen3:14b"

[providers.codex]
kind = "codex"
default_model = "gpt-5.3-codex"
auth_provider_id = "openai"

[providers.gitlab]
kind = "gitlab"
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
auth_provider_id = "gitlab"
api_key_env = "GITLAB_TOKEN"
ai_gateway_headers = { "anthropic-beta" = "context-1m-2025-08-07" }
feature_flags = { duo_agent_platform = true }

[providers."github-copilot"]
kind = "copilot"
default_model = "gpt-4o-mini"
base_url = "https://api.githubcopilot.com"
models_url = "https://..."
auth_provider_id = "github-copilot"

[providers.bedrock]
kind = "amazon_bedrock"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"
default_model = "amazon.nova-pro-v1:0"
region = "us-east-1"
profile = "default"
access_key_id = "AKIA..."
secret_access_key = "..."
session_token = "..."

[providers.vertex]
kind = "google_vertex"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
default_model = "google/gemini-2.5-flash"
access_token_env = "GOOGLE_VERTEX_ACCESS_TOKEN"

[providers.cloudflare]
kind = "cloudflare_ai_gateway"
base_url = "https://gateway.ai.cloudflare.com/v1/ACCOUNT/GATEWAY/compat"
default_model = "openai/gpt-4o-mini"
api_key_env = "CLOUDFLARE_API_TOKEN"

[providers.openrouter]
kind = "preset"
default_model = "openai/gpt-4o-mini"

[mcp.servers.my_stdio]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/repo"]
env = { DEBUG = "1" }
cwd = "/repo"

[mcp.servers.my_http]
transport = "http"
url = "https://mcp.example.com"
mode = "streamable_http"
headers = { "X-Client" = "agena" }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }

[lsp.servers.rust_analyzer]
command = "rust-analyzer"
args = []
env = {}
file_extensions = ["rs"]
root_markers = ["Cargo.toml", ".git"]
initialization_options = { cargo = { allFeatures = true } }

[web]
fetch_enabled = true

[web.search]
backend = "duckduckgo_html"
tavily_api_key = "..."
exa_api_key = "..."
brave_api_key = "..."
```

## 3. 已移除的 mode 能力

旧版本文档里曾经描述过：

- 顶层 `mode = "..."`
- `[modes.<name>]`
- `extends`
- `AGENA_MODE`
- `--mode`
- `-c mode=<name>`

这些能力现在都已经移除，不再参与配置合并。保留旧写法会直接报错，而不是静默忽略。

当前推荐方式：

- 用不同的完整配置文件区分环境
- 把少量差异放到环境变量
- 或者用 `-c/--set` 覆盖单个字段
- 或者用外层部署逻辑生成不同 TOML

## 4. 顶层字段参考

## 4.1 `[tracing]`

```toml
[tracing]
filter = "info"
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `filter` | `string` | `"info"` | tracing 过滤表达式 |
| `database_level` | `string` | `"error"` | 单独控制数据库/SQL 相关日志级别，覆盖 `sqlx`、`sea_orm`、`sea_orm_migration`；默认 `error` 不会打印每条 SQL 语句 |

对应环境变量：

- `AGENA_LOG`
- `AGENA_DATABASE_LOG`

## 4.2 `[telemetry]`

```toml
[telemetry]
enabled = false
service_name = "agena"
otlp_endpoint = "http://127.0.0.1:4318/v1/traces"

[telemetry.headers]
Authorization = "Bearer xxx"
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `enabled` | `bool` | `false` | 是否启用 OTel/OTLP 导出 |
| `service_name` | `string` | `"agena"` | service name |
| `otlp_endpoint` | `string \| null` | `null` | OTLP HTTP trace endpoint |
| `headers` | `map<string,string>` | `{}` | OTLP exporter 额外 HTTP header |

对应环境变量：

- `AGENA_TELEMETRY_ENABLED`
- `AGENA_OTEL_SERVICE_NAME`
- `AGENA_OTEL_ENDPOINT`
- `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`

## 4.3 `[auth]`

```toml
[auth]
store_backend = "auto"
store_path = "~/.agena/auth.json"
```

| 字段 | 类型 | 默认值 | 可选值 | 说明 |
|---|---|---:|---|---|
| `store_backend` | `string` | `"auto"` | `auto`, `file`, `keyring` | 鉴权存储后端 |
| `store_path` | `string` | `~/.agena/auth.json` | 任意路径 | 文件存储路径 |

说明：

- `auto`：优先 OS keyring，不可用时回退到文件
- `file`：强制文件存储
- `keyring`：强制 OS keyring，不回退

对应环境变量：

- `AGENA_AUTH_FILE`

CLI `-c` 支持：

- `auth.store_path=...`
- `auth.store_backend=auto|file|keyring`

## 4.4 `[ui]`

```toml
[ui]
locale = "zh-CN"
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `locale` | `string \| null` | `null` | UI locale |

对应环境变量：

- `AGENA_LOCALE`

CLI `-c` 支持：

- `ui.locale=<value>`

## 4.5 `[runtime.*]`

### 4.5.1 `[runtime.provider_http]`

```toml
[runtime.provider_http]
timeout_secs = 120
connect_timeout_secs = 15
```

| 字段 | 类型 | 默认值 | 约束 | 说明 |
|---|---|---:|---|---|
| `timeout_secs` | `u64` | `120` | `> 0` | provider 请求总超时 |
| `connect_timeout_secs` | `u64` | `15` | `> 0` | provider 建连超时 |

对应环境变量：

- `AGENA_PROVIDER_HTTP_TIMEOUT_SECS`
- `AGENA_PROVIDER_CONNECT_TIMEOUT_SECS`

CLI `-c` 支持：

- `runtime.provider_http.timeout_secs=...`
- `runtime.provider_http.connect_timeout_secs=...`

### 4.5.2 `[runtime.request_retry]`

```toml
[runtime.request_retry]
max_retries = 5
base_delay_ms = 250
max_delay_ms = 2000
```

| 字段 | 类型 | 默认值 | 约束 | 说明 |
|---|---|---:|---|---|
| `max_retries` | `u32` | `5` | `>= 0` | provider 请求最大重试次数 |
| `base_delay_ms` | `u64` | `250` | `>= 0` | 初始退避时间 |
| `max_delay_ms` | `u64` | `2000` | `>= base_delay_ms` | 最大退避时间 |

对应环境变量：

- `AGENA_PROVIDER_REQUEST_MAX_RETRIES`
- `AGENA_PROVIDER_RETRY_BASE_DELAY_MS`
- `AGENA_PROVIDER_RETRY_MAX_DELAY_MS`

CLI `-c` 支持：

- `runtime.request_retry.max_retries=...`
- `runtime.request_retry.base_delay_ms=...`
- `runtime.request_retry.max_delay_ms=...`

### 4.5.3 `[runtime.stream_replay]`

```toml
[runtime.stream_replay]
max_retries_after_output = 5
max_tracked_events = 2048
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `max_retries_after_output` | `u32` | `5` | 流式输出开始后允许的恢复重试次数 |
| `max_tracked_events` | `usize` | `2048` | 最多跟踪多少个流事件用于 replay |

对应环境变量：

- `AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES`
- `AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS`

CLI `-c` 支持：

- `runtime.stream_replay.max_retries_after_output=...`
- `runtime.stream_replay.max_tracked_events=...`

### 4.5.4 `[runtime.reload]`

```toml
[runtime.reload]
enabled = true
poll_interval_secs = 2
```

| 字段 | 类型 | 默认值 | 约束 | 说明 |
|---|---|---:|---|---|
| `enabled` | `bool` | `true` |  | 是否启用轮询式 reload |
| `poll_interval_secs` | `u64` | `2` | `> 0` | 轮询间隔 |

### 4.5.5 `[runtime.janitor]`

```toml
[runtime.janitor]
enabled = true
interval_secs = 30
```

| 字段 | 类型 | 默认值 | 约束 | 说明 |
|---|---|---:|---|---|
| `enabled` | `bool` | `true` |  | 是否启用 janitor |
| `interval_secs` | `u64` | `30` | `> 0` | 运行周期 |

### 4.5.6 `[runtime.session_cache]`

```toml
[runtime.session_cache]
max_sessions = 128
ttl_secs = 900
max_bytes = 67108864
```

| 字段 | 类型 | 默认值 | 约束 | 说明 |
|---|---|---:|---|---|
| `max_sessions` | `usize` | `128` | `> 0` | 最多缓存多少 session |
| `ttl_secs` | `u64` | `900` | `> 0` | session cache TTL |
| `max_bytes` | `usize` | `67108864` | `> 0` | session cache 最大总字节数 |

## 4.6 权限 `[permission]` 与 `[agents.<name>.permission]`

权限分两层：顶层 `[permission]` 是默认 policy，具体 agent 的
`[agents.<name>.permission]` 是 overlay。agent 默认继承顶层各 section，
也可以通过 `inherit` 显式关闭继承。

```toml
[runtime]
default_agent = "build"

[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "deny" }

[permission.path.rules]
"<cwd>/.env*" = { read = "ask", write = "deny" }
"<home>/.ssh/**" = { read = "deny", write = "deny" }

[permission.network]
internet = "ask"
private = "deny"
loopback = "deny"

[permission.network.rules]
"github.com:443" = "allow"
"*.corp.local:443" = "ask"

[permission.tools.tags]
read_only = "allow"
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"
internet = "ask"
mutating = "ask"
task = "ask"
shell = "ask"

[permission.tools.first_party]
bash = "ask"
apply_patch = "ask"

[agents.plan]
description = "Read-only planning agent"
prompt = "You are a planning agent."
allowed_tools = ["read", "view_file", "glob", "grep", "bash", "todo_write", "enter_plan_mode", "exit_plan_mode"]
mode = "all"

[agents.plan.permission.inherit]
path = true
network = true
tools = true
plugin_tools = true

[agents.plan.permission.path]
workspace = { read = "allow", write = "deny" }
external = { read = "ask", write = "deny" }

[agents.plan.permission.tools.first_party]
enter_plan_mode = "allow"
exit_plan_mode = "allow"
todo_write = "allow"

[agents.plan.permission.tools.rules.bash]
"git status*" = "allow"
"git push *" = "deny"
"rm -rf *" = "deny"
```

### 4.6.1 Path 权限

`path` 统一描述 workspace 内和 workspace 外的读写权限。旧的
`default_read` / `default_write` / `default_external_directory` /
`read` / `write` / `external_directory` 已移除。

```toml
[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "deny" }

[permission.path.rules]
"<cwd>/src/**" = { read = "allow", write = "ask" }
"<cwd>/secrets/**" = { read = "deny", write = "deny" }
"<tmp>/agena/**" = "read_write"
```

| 字段 | 类型 | 可选值 | 说明 |
|---|---|---|---|
| `workspace` | inline table | `allow`, `ask`, `deny` | workspace 内默认 read/write |
| `external` | inline table | `allow`, `ask`, `deny` | workspace 外默认 read/write |
| `rules` | table | `allow`, `ask`, `deny`, `read`, `read_write`, table | 按路径 pattern 覆盖 read/write |

路径 marker：

- `<cwd>` / `<workspace>`：当前 session 的 effective workspace root
- `<home>`：用户 home
- `<tmp>`：系统临时目录
- 无 marker 的相对 pattern 默认按 `<cwd>` 相对路径解释

### 4.6.2 Network 权限

`network` 统一描述出站连接权限。默认分类包括：

- `internet`：公网目标，以及不能静态证明是内网的域名
- `private`：RFC1918/private IP、link-local、单段主机名、`.local` / `.lan` / `.internal` / `.corp` / `.home.arpa`
- `loopback`：`localhost` / `*.localhost` / loopback IP

```toml
[permission.network]
internet = "ask"
private = "deny"
loopback = "deny"

[permission.network.rules]
"github.com:443" = "allow"
"*.corp.local:443" = "ask"
"10.0.0.0/8:*" = "deny"
"localhost:*" = "deny"
```

network rule 支持：

- host：`github.com`
- host + port：`github.com:443`
- wildcard host：`*.corp.local:443`
- CIDR：`10.0.0.0/8:*`
- `*` / `:*` 风格端口通配

插件可以在 manifest 里用 `input_networks` / `network_access` 声明网络目标，也可以通过 `permission_networks` hook 动态返回目标。host 会在工具执行前把这些目标交给 network policy 判断。

### 4.6.3 工具权限

`tools` 描述 tag 默认值、first-party 工具、plugin 工具，以及工具内部规则：

```toml
[permission.tools.tags]
read_only = "allow"
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"
internet = "ask"
mutating = "ask"
task = "ask"
shell = "ask"

[permission.tools.first_party]
read = "allow"
grep = "allow"
bash = "ask"
apply_patch = "ask"

[permission.tools.plugin]
"github.create_issue" = "ask"
"fs.read_file" = "allow"

[permission.tools.rules.bash]
"git status*" = "allow"
"cargo test*" = "ask"
"git push *" = "deny"
```

当一个 tool 命中多个 tag 时，默认权限按最保守的结果合并：

```text
deny > ask > allow
```

例如一个 tool 同时命中 `read_only = "allow"` 和 `network = "ask"`，最终默认是 `ask`。精确 tool 规则仍然覆盖 tag 默认；bash command rule 仍然可以按命令 pattern 做更细覆盖。

### 4.6.4 Agent 继承

`inherit` 可以是布尔值，也可以按 section 配置：

```toml
[agents.locked.permission]
inherit = false

[agents.locked.permission.path]
workspace = { read = "allow", write = "deny" }
external = { read = "deny", write = "deny" }
```

```toml
[agents.plan.permission.inherit]
path = true
network = true
tools = true
plugin_tools = true
```

agent overlay 的 path rules 会追加在顶层 rules 之后，因此最后匹配的规则优先。

### 4.6.5 配置原则

- 顶层 `[permission]` 是新的默认权限 schema，不兼容旧 `[permission] mode = ...`
- `permission.tools.default` / `read_only_default` / `mutating_default` 已移除，改用 `[permission.tools.tags]`
- 旧的 `[[permission.bash]]`、`[[permission.bash_deny]]`、`tool_rules`、`bash_rules`、`bash_deny_patterns` 配置入口已移除
- 每个 agent 的提示词、工具白名单、模型和 permission overlay 仍放在 `[agents.<name>]`
- 用 `runtime.default_agent` 控制新 root session 的默认 agent

## 4.7 `[memory.project_instructions]`

```toml
[memory.project_instructions]
enabled = true
include_global = true
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `enabled` | `bool` | `true` | 是否启用 project instructions 记忆层 |
| `include_global` | `bool` | `true` | 是否包含全局 instructions |

## 4.8 `[[hooks]]`

```toml
[[hooks]]
event = "user_prompt_submit"
command = "python3 .agena/hooks/enrich.py"
timeout_ms = 3000

[[hooks]]
event = "pre_tool_use"
url = "http://127.0.0.1:8080/hook"
matcher = { tool = "bash" }
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `event` | `string` |  | hook 事件名 |
| `command` | `string \| null` | `null` | 本地 shell command |
| `url` | `string \| null` | `null` | HTTP hook endpoint |
| `matcher.tool` | `string \| null` | `null` | 仅对 tool hook 有意义的 tool 名 glob |
| `timeout_ms` | `u64 \| null` | `30000` | 单次执行超时 |

### 4.8.1 `event` 可选值

当前支持：

- `user_prompt_submit`
- `pre_tool_use`
- `post_tool_use`
- `post_tool_use_failure`
- `stop`
- `session_start`
- `session_end`
- `notification`

兼容别名：

- `tool_before` -> `pre_tool_use`
- `tool_after` -> `post_tool_use`
- `tool_failure` -> `post_tool_use_failure`
- `agent_stop` -> `stop`

说明：

- `command` 与 `url` 可以都写，但当前实现里如果两者都存在，`url` 优先
- hook 失败不会中断整个 agent 主流程，但会记日志
- mode 层中的 `hooks` 是整段替换，不是追加

## 4.9 `[plugins]`

> 这一段是当前仓库里最容易被旧文档误导的部分。  
> 当前运行时不再支持旧的 `[plugins].paths = [...]` 形式；现在必须使用 `[plugins.list.<id>]` 明确声明每个 plugin entry。

### 4.9.1 顶层字段

```toml
[plugins]
enabled = true
timeouts = { tool_invoke = "60s", permission_ask = "10s" }
```

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `enabled` | `bool` | `true` | 是否启用 plugin host；`false` 时包含内建 plugin 在内的 plugin host 全部关闭 |
| `timeouts` | `object` | 全部 `null` | 全局默认超时覆盖 |
| `list` | `map<string, PluginEntry>` | `{}` | plugin 声明表 |
| `default_quota` | `QuotaConfig` | 无限 | 没有单独 quota 的 plugin 默认限额 |
| `quotas` | `map<string, QuotaConfig>` | `{}` | 按 plugin id 的 quota 覆盖 |
| `trusted_keys` | `map<string,string>` | `{}` | `key_id -> hex ed25519 public key` |

说明：

- 即使你不声明任何 plugin，runtime 也会自动注册若干内建 static plugin
- `enabled = false` 会直接返回空 `PluginHost`

### 4.9.2 `[plugins.timeouts]` / `timeouts = {...}`

支持这些字段：

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `init` | `duration` | host 默认值 | `meta/init` timeout |
| `tool_hook` | `duration` | host 默认值 | `tool.execute.before/after` timeout |
| `tool_invoke` | `duration` | host 默认值 | `tool.invoke` timeout |
| `permission_ask` | `duration` | host 默认值 | `permission.ask` timeout |
| `chat` | `duration` | host 默认值 | `chat.*` timeout |
| `fast` | `duration` | host 默认值 | `shell.env` / `command.execute.before` / `config` timeout |

`duration` 支持格式：

- `"200ms"`
- `"5s"`
- `"2m"`
- `"1h"`

### 4.9.3 `[plugins.default_quota]` 与 `[plugins.quotas.<id>]`

```toml
[plugins.default_quota]
rate_per_sec = 0
burst = 0
max_concurrent = 0

[plugins.quotas.my-plugin]
rate_per_sec = 5
burst = 10
max_concurrent = 2
```

字段说明：

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `rate_per_sec` | `u32` | `0` | 持续速率；`0` 表示不限速 |
| `burst` | `u32` | `0` | 突发桶大小；`0` 表示跟随 `rate_per_sec` |
| `max_concurrent` | `u32` | `0` | 最大并发 host call 数；`0` 表示不限制 |

### 4.9.4 `[plugins.trusted_keys]`

```toml
[plugins.trusted_keys]
release = "0123abcd..."
```

- key：`key_id`
- value：十六进制编码的 ed25519 公钥

用于 `cdylib` entry 的签名验证。

### 4.9.5 `[plugins.list.<id>]` 通用规则

- `<id>` 是 plugin id
- `kind` 必填
- `options` 是任意 JSON/TOML 值，原样传给 plugin
- `timeouts` 可以为单个 plugin 覆盖超时

#### `kind` 可选值

- `static`
- `cdylib`
- `stdio`
- `http`
- `wasm`

### 4.9.6 `kind = "static"`

```toml
[plugins.list.my_static]
kind = "static"
options = { enabled = true }
timeouts = { tool_invoke = "30s" }
```

字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `kind` | `string` | 是 | 固定为 `static` |
| `options` | `json-like` | 否 | 传给 static plugin 的配置 |
| `timeouts` | `object` | 否 | 局部 timeout 覆盖 |

### 4.9.7 `kind = "cdylib"`

```toml
[plugins.list.echo]
kind = "cdylib"
path = "./plugins/libecho.so"
options = { uppercase = true }
timeouts = { tool_invoke = "30s" }
sha256 = "..."

[plugins.list.echo.signature]
key_id = "release"
signature = "..."
```

字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `kind` | `string` | 是 | 固定为 `cdylib` |
| `path` | `string` | 是 | 动态库路径 |
| `options` | `json-like` | 否 | 自定义配置 |
| `timeouts` | `object` | 否 | timeout 覆盖 |
| `sha256` | `string \| null` | 否 | 动态库字节的 sha256 hex |
| `signature.key_id` | `string` | 否 | `[plugins.trusted_keys]` 里的 key id |
| `signature.signature` | `string` | 否 | ed25519 签名 hex |

说明：

- 相对 `path` 会相对配置文件所在目录解释
- 如果开启了签名字段但 host 编译时没开 `signing` feature，会加载失败

### 4.9.8 `kind = "stdio"`

```toml
[plugins.list.worker]
kind = "stdio"
command = "node"
args = ["./plugins/worker/index.js"]
env = { NODE_ENV = "production" }
cwd = "/repo"
restart = { policy = "on-failure", min_backoff = "1s", max_backoff = "30s", max_retries = 5 }
options = { profile = "default" }
timeouts = { tool_invoke = "45s" }
sha256 = "..."
```

字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `kind` | `string` | 是 | 固定为 `stdio` |
| `command` | `string` | 是 | 可执行命令 |
| `args` | `string[]` | 否 | 参数列表 |
| `env` | `map<string,string>` | 否 | 子进程环境变量 |
| `cwd` | `string \| null` | 否 | 子进程工作目录 |
| `restart.policy` | `string` | 否 | `never`, `on-failure`, `always` |
| `restart.min_backoff` | `duration` | 否 | 默认 `1s` |
| `restart.max_backoff` | `duration` | 否 | 默认 `30s` |
| `restart.max_retries` | `u32` | 否 | 默认 `5` |
| `options` | `json-like` | 否 | 自定义配置 |
| `timeouts` | `object` | 否 | timeout 覆盖 |
| `sha256` | `string \| null` | 否 | 可执行文件 sha256 |

说明：

- `command` 当前按字面值 / PATH 解析；如果要行为稳定，建议写绝对路径
- `cwd` 当前会原样传给子进程，建议也使用绝对路径

### 4.9.9 `kind = "http"`

```toml
[plugins.list.remote]
kind = "http"
url = "https://plugins.example.com/rpc"
auth = { kind = "bearer", token_env = "PLUGIN_TOKEN" }
options = { org = "acme" }
timeouts = { chat = "5s" }
```

字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `kind` | `string` | 是 | 固定为 `http` |
| `url` | `string` | 是 | JSON-RPC POST endpoint |
| `auth` | `object` | 否 | HTTP 鉴权 |
| `options` | `json-like` | 否 | 自定义配置 |
| `timeouts` | `object` | 否 | timeout 覆盖 |

`auth.kind` 可选值：

- `none`
- `bearer`
- `basic`

`bearer` 结构：

```toml
auth = { kind = "bearer", token = "xxx" }
auth = { kind = "bearer", token_env = "PLUGIN_TOKEN" }
```

字段：

- `token`
- `token_env`

`basic` 结构：

```toml
auth = { kind = "basic", username = "bot", password = "secret" }
auth = { kind = "basic", username = "bot", password_env = "PLUGIN_PASSWORD" }
```

字段：

- `username`
- `password`
- `password_env`

### 4.9.10 `kind = "wasm"`

```toml
[plugins.list.tool]
kind = "wasm"
path = "./plugins/tool.wasm"
options = { mode = "safe" }
timeouts = { tool_invoke = "20s" }
sha256 = "..."

[plugins.list.tool.sandbox]
allow_fs_read = ["/repo"]
allow_fs_write = ["/tmp"]
allow_net = false
allow_env = ["HOME"]
```

字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `kind` | `string` | 是 | 固定为 `wasm` |
| `path` | `string` | 是 | wasm 文件路径 |
| `options` | `json-like` | 否 | 自定义配置 |
| `timeouts` | `object` | 否 | timeout 覆盖 |
| `sha256` | `string \| null` | 否 | wasm 字节 sha256 |
| `sandbox.allow_fs_read` | `string[]` | 否 | 只读预打开路径 |
| `sandbox.allow_fs_write` | `string[]` | 否 | 读写预打开路径 |
| `sandbox.allow_net` | `bool` | 否 | 是否允许网络 |
| `sandbox.allow_env` | `string[]` | 否 | 允许透传的环境变量名 |

说明：

- 相对 `path` 会相对配置文件所在目录解释
- 默认 sandbox 是全拒绝
- 需要 host 编译时启用 `wasm` feature

### 4.9.11 关于 plugin storage

`config.full.toml` 里的注释提到了 `[plugins.storage]`，但 **当前 runtime 配置 schema 里没有这个 TOML 段**。

当前实际行为是：

- plugin storage root 默认：`~/.agena/plugin-storage`
- 可用环境变量 `AGENA_PLUGIN_STORAGE_DIR` 覆盖
- secret store backend / fallback 当前是 runtime 内部默认值，不提供 TOML 配置入口

## 5. `[providers]`

`[providers.<id>]` 是整个配置里最重要的一段。

基本规则：

- 至少配置一个 provider 才能正常运行
- `<id>` 是 provider id，例如 `openai`、`anthropic`、`openrouter`
- 只有显式声明在 `[providers]` 里的 provider 才会注册进 runtime

### 5.1 通用字段

这些字段不是所有 kind 都使用，但它们都属于当前 parser 可识别字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `enabled` | `bool` | 是否启用该 provider，默认 `true` |
| `kind` | `string` | provider 类型，必填 |
| `default_model` | `string` | 默认模型名 |
| `base_url` | `string` | API base URL |
| `api_key` | `string` | 明文 API key |
| `api_key_env` | `string` | 从环境变量读取 API key |
| `auth_header` | `string` | 自定义鉴权 header 名 |
| `auth_scheme` | `string` | 自定义鉴权 scheme |
| `extra_headers` | `map<string,string>` | 额外 HTTP header |
| `stream_mode` | `string` | `sse` / `realtime_websocket` |
| `realtime_ws_url` | `string` | realtime websocket URL |
| `thinking_depths` | `map<string,u32>` | 命名的 thinking budget |
| `default_thinking` | `string` | 默认 thinking preset 名，或 `disabled` |
| `models` | `map<string,object>` | provider 下按模型 id 显式声明的模型配置 |

此外，某些 kind 还会用到：

- `auth_provider_id`
- `instance_url`
- `ai_gateway_url`
- `ai_gateway_headers`
- `feature_flags`
- `models_url`
- `region`
- `profile`
- `access_token`
- `access_token_env`
- `access_key_id`
- `secret_access_key`
- `session_token`

### 5.2 `kind` 可选值

当前代码支持的 `kind`：

- `preset`
- `ollama`
- `openai`
- `openai_compatible`
- `sap_ai_core`
- `anthropic`
- `gemini`
- `codex`
- `gitlab`
- `copilot`
- `amazon_bedrock`
- `google_vertex`
- `cloudflare_ai_gateway`

### 5.3 `thinking_depths` 与 `default_thinking`

```toml
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "o3-mini"
default_thinking = "standard"

[providers.openai.thinking_depths]
light = 3000
standard = 10000
deep = 30000
```

规则：

- `thinking_depths`：`name -> budget_tokens`
- `default_thinking`：
  - 要么是 `thinking_depths` 里的 key
  - 要么是字面量 `"disabled"`
- 如果 `default_thinking` 引用了不存在的 key，会配置校验失败

### 5.4 `models`

```toml
[providers.openai.models."gpt-5"]
input = { unsupported = ["image"] }
features = ["tool_calling"]
```

规则：

- `models` 是一个 `model_id -> object` 的 map，按模型 id 精确匹配
- 不支持 `exact` / `prefix` / `contains` 之类 matcher
- 如果后端已经列出了该模型，runtime 会在后端返回的 capability / metadata 基础上做 patch
- 如果后端没有列出该模型，runtime 仍会把它作为显式声明模型加入该 provider 的模型列表
- capability patch 现在用紧凑格式：
  - `input = [...]` 表示这些输入模态显式标记为 `supported`
  - `input = { supported = [...], unsupported = [...] }` 可同时声明支持和不支持
  - `features = [...]` / `features = { supported = [...], unsupported = [...] }` 语义相同

字段：

| 字段 | 类型 | 必填 | 可选值 | 说明 |
|---|---|---|---|---|
| `<model-id>` | `string` | 是 |  | 作为子表 key 使用，例如 `"gpt-5"` |
| `display_name` | `string` | 否 |  | 覆盖显示名 |
| `family` | `string` | 否 | `gpt`, `codex`, `claude`, `gemini`, `llama`, `mistral`, `deepseek`, `qwen`, `nova`, `grok`, `phi`, `command` | 模型家族 |
| `lifecycle` | `string` | 否 | `active`, `preview`, `beta`, `alpha`, `experimental`, `deprecated` | 生命周期 |
| `context_window_tokens` | `u32` | 否 |  | 上下文窗口大小 |
| `max_output_tokens` | `u32` | 否 |  | 最大输出 token |
| `description` | `string` | 否 |  | 描述文本 |
| `input` | `array<string>` or `object` | 否 | 输入模态：`text`, `image`, `document`, `audio`, `video`, `file` | 紧凑输入能力 patch |
| `features` | `array<string>` or `object` | 否 | 特性：`tool_calling`, `streaming`, `reasoning`, `structured_output`, `temperature` | 紧凑特性 patch |

校验规则：

- `<model-id>` 不能为空白
- 每个 model 至少要设置一个字段
- `input.supported` 和 `input.unsupported` 不能重复或互相重叠
- `features.supported` 和 `features.unsupported` 不能重复或互相重叠

### 5.5 各 provider kind 参考

#### 5.5.1 `kind = "ollama"`

```toml
[providers.ollama]
kind = "ollama"
base_url = "http://localhost:11434"
default_model = "qwen3:14b"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 否 | `http://localhost:11434` | Ollama base URL |
| `default_model` | 是 |  | 默认模型 |

#### 5.5.2 `kind = "openai"`

```toml
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-5"
api_key_env = "OPENAI_API_KEY"
api_mode = "responses"
stream_mode = "sse"
realtime_ws_url = "wss://..."
extra_headers = { "X-Trace" = "1" }
```

| 字段 | 必填 | 默认值 | 可选值 | 说明 |
|---|---|---:|---|---|
| `base_url` | 是 |  | API base URL |
| `default_model` | 是 |  | 默认模型 |
| `api_key` | 否 | `null` |  | 明文 API key |
| `api_key_env` | 否 | `null` |  | API key 环境变量名 |
| `extra_headers` | 否 | `{}` |  | 额外 header |
| `api_mode` | 否 | `responses` | `responses`, `chat`, `auto` | OpenAI API 模式 |
| `stream_mode` | 否 | `sse` | `sse`, `realtime_websocket` | 流式传输方式 |
| `realtime_ws_url` | 否 | `null` |  | realtime websocket URL |
| `thinking_depths` / `default_thinking` | 否 |  |  | reasoning 预算配置 |
| `models` | 否 | `{}` |  | 按模型 id 显式声明模型配置 |

#### 5.5.3 `kind = "openai_compatible"`

```toml
[providers.compat]
kind = "openai_compatible"
base_url = "https://example.com/v1"
default_model = "my-model"
api_key_env = "COMPAT_API_KEY"
auth_header = "authorization"
auth_scheme = "Bearer"
stream_mode = "sse"
realtime_ws_url = "wss://..."
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 是 |  | API base URL |
| `default_model` | 是 |  | 默认模型 |
| `api_key` / `api_key_env` | 否 | `null` | 鉴权 |
| `extra_headers` | 否 | `{}` | 额外 header |
| `auth_header` | 否 | `"authorization"` | 认证 header 名 |
| `auth_scheme` | 否 | `"Bearer"` | 认证 scheme |
| `stream_mode` | 否 | `sse` | 流模式 |
| `realtime_ws_url` | 否 | `null` | 实时 WS URL |
| `thinking_depths` / `default_thinking` | 否 |  | reasoning 预算 |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

#### 5.5.4 `kind = "sap_ai_core"`

```toml
[providers.sap]
kind = "sap_ai_core"
base_url = "https://example.sap/v1"
default_model = "anthropic/claude-sonnet-4"
api_key_env = "AICORE_TOKEN"
auth_header = "authorization"
auth_scheme = "Bearer"
```

字段与 `openai_compatible` 基本一致，差别在于：

- `kind` 为 `sap_ai_core`
- runtime 内部按 SAP AI Core provider 走特殊适配
- 也支持作为 `preset` 被自动补全

#### 5.5.5 `kind = "anthropic"`

```toml
[providers.anthropic]
kind = "anthropic"
base_url = "https://api.anthropic.com/v1"
default_model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
auth_header = "x-api-key"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 是 |  | API base URL |
| `default_model` | 是 |  | 默认模型 |
| `api_key` / `api_key_env` | 否 | `null` | 鉴权 |
| `extra_headers` | 否 | `{}` | 额外 header |
| `auth_header` | 否 | `"x-api-key"` | 认证 header |
| `auth_scheme` | 否 | `null` | 可选认证 scheme |
| `thinking_depths` / `default_thinking` | 否 |  | reasoning 预算 |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

#### 5.5.6 `kind = "gemini"`

```toml
[providers.gemini]
kind = "gemini"
base_url = "https://generativelanguage.googleapis.com/v1beta"
default_model = "gemini-2.5-pro"
api_key_env = "GEMINI_API_KEY"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 是 |  | API base URL |
| `default_model` | 是 |  | 默认模型 |
| `api_key` / `api_key_env` | 否 | `null` | 鉴权 |
| `extra_headers` | 否 | `{}` | 额外 header |
| `thinking_depths` / `default_thinking` | 否 |  | reasoning 预算 |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

#### 5.5.7 `kind = "codex"`

```toml
[providers.codex]
kind = "codex"
default_model = "gpt-5.3-codex"
auth_provider_id = "openai"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `default_model` | 是 |  | 默认模型 |
| `auth_provider_id` | 否 | `"openai"` | 认证提供者 id |

#### 5.5.8 `kind = "gitlab"`

```toml
[providers.gitlab]
kind = "gitlab"
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
auth_provider_id = "gitlab"
api_key_env = "GITLAB_TOKEN"
ai_gateway_headers = { "anthropic-beta" = "context-1m-2025-08-07" }
feature_flags = { duo_agent_platform = true }
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `instance_url` | 否 | `https://gitlab.com` | GitLab 实例地址 |
| `ai_gateway_url` | 否 | `https://cloud.gitlab.com` | GitLab AI gateway |
| `default_model` | 否 | `"claude-sonnet-4-5"` | 默认模型 |
| `auth_provider_id` | 否 | `"gitlab"` | 认证 provider id |
| `api_key` / `api_key_env` | 否 | `null` | 可选 API key |
| `ai_gateway_headers` | 否 | `{}` | 传给 gateway 的额外 header |
| `feature_flags` | 否 | `{}` | GitLab feature flags |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

#### 5.5.9 `kind = "copilot"`

```toml
[providers."github-copilot"]
kind = "copilot"
default_model = "gpt-4o-mini"
base_url = "https://api.githubcopilot.com"
models_url = "https://..."
auth_provider_id = "github-copilot"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `default_model` | 否 | `"gpt-4o-mini"` | 默认模型 |
| `base_url` | 否 | `https://api.githubcopilot.com` | API base URL |
| `models_url` | 否 | `null` | 模型列表 URL |
| `auth_provider_id` | 否 | 当前 provider id | 认证 provider id |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

#### 5.5.10 `kind = "amazon_bedrock"`

```toml
[providers.bedrock]
kind = "amazon_bedrock"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"
default_model = "amazon.nova-pro-v1:0"
region = "us-east-1"
profile = "default"
access_key_id = "AKIA..."
secret_access_key = "..."
session_token = "..."
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 是 |  | OpenAI-compatible Bedrock endpoint |
| `default_model` | 是 |  | 默认模型 |
| `region` | 是 |  | AWS region |
| `api_key` / `api_key_env` | 否 | `null` | 如果提供，则使用 bearer auth |
| `profile` | 否 | `null` | SigV4 profile |
| `access_key_id` | 否 | `null` | SigV4 静态 AK |
| `secret_access_key` | 否 | `null` | SigV4 静态 SK |
| `session_token` | 否 | `null` | SigV4 session token |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

鉴权规则：

- 如果设置了 `api_key` 或 `api_key_env`：
  - 走 `bearer` 模式
- 否则：
  - 走 `sigv4` 模式
- `access_key_id` 和 `secret_access_key` 必须成对出现；只配一个会报错

#### 5.5.11 `kind = "google_vertex"`

```toml
[providers.vertex]
kind = "google_vertex"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
default_model = "google/gemini-2.5-flash"
access_token_env = "GOOGLE_VERTEX_ACCESS_TOKEN"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 是 |  | Vertex OpenAPI endpoint |
| `default_model` | 是 |  | 默认模型 |
| `access_token` | 否 | `null` | 静态 access token |
| `access_token_env` | 否 | `null` | access token 环境变量 |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

鉴权规则：

- 如果设置了 `access_token` 或 `access_token_env`：
  - 使用静态 token
- 否则：
  - 使用 ADC（Application Default Credentials）

#### 5.5.12 `kind = "cloudflare_ai_gateway"`

```toml
[providers.cloudflare]
kind = "cloudflare_ai_gateway"
base_url = "https://gateway.ai.cloudflare.com/v1/ACCOUNT/GATEWAY/compat"
default_model = "openai/gpt-4o-mini"
api_key_env = "CLOUDFLARE_API_TOKEN"
```

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---:|---|
| `base_url` | 是 |  | Gateway 兼容 endpoint |
| `default_model` | 是 |  | 默认模型 |
| `api_key` / `api_key_env` | 否 | `null` | API token |
| `models` | 否 | `{}` | 按模型 id 显式声明模型配置 |

#### 5.5.13 `kind = "preset"`

```toml
[providers.openrouter]
kind = "preset"

[providers.ollama]
kind = "preset"
default_model = "qwen3:14b"
```

`preset` 的含义：

- provider 的 `<id>` 先被当成 provider preset id
- runtime 会从 `models.dev` 的 provider metadata 或内建 preset 数据中补齐真实配置
- 然后再落成某个具体 provider kind

当前行为：

- metadata 缓存在 `~/.agena/provider-presets.json`
- 可用 `AGENA_PROVIDER_PRESETS_PATH` 指向本地 preset JSON
- 只有显式写在 `[providers]` 下的 preset 才会注册

##### 内建常用 preset

当前代码里内建保证可识别的 preset id：

- `ollama`
- `lmstudio`
- `openrouter`
- `deepseek`
- `xai`
- `groq`
- `mistral`

##### preset 的常见特殊处理

部分 provider id 有额外逻辑：

- `ollama`
  - 可读 `OLLAMA_HOST`
- `lmstudio`
  - 可读 `LMSTUDIO_HOST`
- `openrouter`
  - 自动补 `X-Title: agena`
- `vercel`
  - 自动补 `x-title: agena`
- `cerebras`
  - 自动补 `X-Cerebras-3rd-Party-Integration: agena`
- `opencode` / `opencode-go`
  - 没有 `OPENCODE_API_KEY` 时，会回退到公开 key `"public"`
- `cloudflare-workers-ai`
  - 需要 `CLOUDFLARE_ACCOUNT_ID`
- `google-vertex`
  - 若未显式提供 `base_url`，会尝试拼接
  - project 可来自：`GOOGLE_VERTEX_PROJECT` / `GOOGLE_CLOUD_PROJECT` / `GCP_PROJECT` / `GCLOUD_PROJECT`
  - location 可来自：`GOOGLE_VERTEX_LOCATION` / `GOOGLE_CLOUD_LOCATION` / `VERTEX_LOCATION`
- `cloudflare-ai-gateway`
  - 若未显式提供 `base_url`，需要：
  - `CLOUDFLARE_ACCOUNT_ID`
  - `CLOUDFLARE_GATEWAY_ID`
- `sap-ai-core`
  - 若未显式提供 `api_key` + `base_url`，需要 `AICORE_SERVICE_KEY`
  - 若存在 `AICORE_RESOURCE_GROUP`，会自动补 `AI-Resource-Group` header
- `azure`
  - 若未显式提供 `base_url`，需要 `AZURE_RESOURCE_NAME`
- `azure-cognitive-services`
  - 若未显式提供 `base_url`，需要 `AZURE_COGNITIVE_SERVICES_RESOURCE_NAME`

##### preset 最终会落成的 kind

常见情况：

- `ollama` -> `ollama`
- `openai` / `azure` -> `openai`
- `openrouter` / `deepseek` / `groq` / `mistral` / `lmstudio` -> `openai_compatible`
- `anthropic` -> `anthropic`
- `google` -> `gemini`
- `google-vertex` -> `google_vertex`
- `github-copilot` -> `copilot`
- `sap-ai-core` -> `sap_ai_core`

## 6. `[mcp]`

```toml
[mcp.servers.fs]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/repo"]

[mcp.servers.remote]
transport = "http"
url = "https://mcp.example.com"
mode = "streamable_http"
headers = { "X-Client" = "agena" }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
```

### 6.1 顶层结构

```toml
[mcp]
[mcp.servers.<name>]
```

- `servers` 是 `map<string, McpServerConfig>`

### 6.2 `transport = "stdio"`

字段：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `transport` | `string` | 是 | 固定为 `stdio` |
| `command` | `string` | 是 | MCP server 启动命令 |
| `args` | `string[]` | 否 | 参数 |
| `env` | `map<string,string>` | 否 | 环境变量 |
| `cwd` | `string \| null` | 否 | 工作目录 |

### 6.3 `transport = "http"`

字段：

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|---|---|---|---:|---|
| `transport` | `string` | 是 |  | 固定为 `http` |
| `url` | `string` | 是 |  | MCP server URL |
| `mode` | `string` | 否 | `streamable_http` | `sse` / `streamable_http` |
| `headers` | `map<string,string>` | 否 | `{}` | 额外 header |
| `auth` | `object \| null` | 否 | `null` | HTTP 鉴权 |

`auth.kind` 可选值：

- `bearer`
- `bearer_from_env`
- `bearer_from_store`
- `custom`

结构示例：

```toml
auth = { kind = "bearer", token = "xxx" }
auth = { kind = "bearer_from_env", env = "MCP_TOKEN" }
auth = { kind = "bearer_from_store" }
auth = { kind = "custom", headers = { Authorization = "Bearer xxx" } }
```

## 7. `[lsp]`

```toml
[lsp.servers.rust_analyzer]
command = "rust-analyzer"
args = []
env = {}
file_extensions = ["rs"]
root_markers = ["Cargo.toml", ".git"]
initialization_options = { cargo = { allFeatures = true } }
```

字段：

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `command` | `string` |  | 启动命令，必填 |
| `args` | `string[]` | `[]` | 参数 |
| `env` | `map<string,string>` | `{}` | 环境变量 |
| `file_extensions` | `string[]` | `[]` | 负责处理哪些扩展名，不带 `.`；空表示全部 |
| `root_markers` | `string[]` | `[]` | 识别项目根目录的标记文件 |
| `initialization_options` | `json \| null` | `null` | LSP initializationOptions |

## 8. `[web]`

```toml
[web]
fetch_enabled = true

[web.search]
backend = "duckduckgo_html"
```

### 8.1 `[web]`

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---:|---|
| `fetch_enabled` | `bool` | `false` | 是否启用 `web_fetch` 风格能力 |

### 8.2 `[web.search]`

| 字段 | 类型 | 默认值 | 可选值 | 说明 |
|---|---|---:|---|---|
| `backend` | `string` | `duckduckgo_html` | `tavily`, `exa`, `brave`, `duckduckgo_html` | 搜索后端 |
| `tavily_api_key` | `string \| null` | `null` |  | Tavily key |
| `exa_api_key` | `string \| null` | `null` |  | Exa key |
| `brave_api_key` | `string \| null` | `null` |  | Brave key |

key 的回退规则：

- `tavily_api_key` 为空时回退 `TAVILY_API_KEY`
- `exa_api_key` 为空时回退 `EXA_API_KEY`
- `brave_api_key` 为空时回退 `BRAVE_API_KEY`

## 9. 环境变量覆盖

## 9.1 顶层环境变量

| 环境变量 | 对应字段 |
|---|---|
| `AGENA_CONFIG` | 配置文件路径 |
| `AGENA_AUTH_FILE` | `auth.store_path` |
| `AGENA_LOG` | `tracing.filter` |
| `AGENA_DATABASE_LOG` | `tracing.database_level` |
| `AGENA_TELEMETRY_ENABLED` | `telemetry.enabled` |
| `AGENA_OTEL_SERVICE_NAME` | `telemetry.service_name` |
| `AGENA_OTEL_ENDPOINT` | `telemetry.otlp_endpoint` |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | `telemetry.otlp_endpoint`（后备） |
| `AGENA_LOCALE` | `ui.locale` |
| `AGENA_PLUGIN_ENABLED` | `plugins.enabled` |
| `AGENA_PROVIDER_HTTP_TIMEOUT_SECS` | `runtime.provider_http.timeout_secs` |
| `AGENA_PROVIDER_CONNECT_TIMEOUT_SECS` | `runtime.provider_http.connect_timeout_secs` |
| `AGENA_PROVIDER_REQUEST_MAX_RETRIES` | `runtime.request_retry.max_retries` |
| `AGENA_PROVIDER_RETRY_BASE_DELAY_MS` | `runtime.request_retry.base_delay_ms` |
| `AGENA_PROVIDER_RETRY_MAX_DELAY_MS` | `runtime.request_retry.max_delay_ms` |
| `AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES` | `runtime.stream_replay.max_retries_after_output` |
| `AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS` | `runtime.stream_replay.max_tracked_events` |
| `AGENA_PROVIDER_PRESETS_PATH` | provider preset JSON 路径 |
| `AGENA_PLUGIN_STORAGE_DIR` | plugin storage 根目录 |
| `TAVILY_API_KEY` | web search Tavily key |
| `EXA_API_KEY` | web search Exa key |
| `BRAVE_API_KEY` | web search Brave key |

## 9.2 provider 环境变量覆盖

支持的格式：

```text
AGENA_PROVIDER__<PROVIDER_ID>__<FIELD>=...
```

规则：

- `<PROVIDER_ID>` 会先转小写
- 然后 `_` 会被替换成 `-`
- 例如：
  - `AGENA_PROVIDER__GOOGLE_VERTEX__KIND=google_vertex`
  - 实际对应 provider id：`google-vertex`

当前支持这些 `<FIELD>`：

- `ENABLED`
- `KIND`
- `DEFAULT_MODEL`
- `BASE_URL`
- `API_KEY`
- `API_KEY_ENV`
- `AUTH_HEADER`
- `AUTH_SCHEME`
- `STREAM_MODE`
- `API_MODE`
- `REALTIME_WS_URL`
- `DEFAULT_THINKING`
- `TARGET_PROVIDER_ID`
- `AUTH_PROVIDER_ID`
- `INSTANCE_URL`
- `AI_GATEWAY_URL`
- `MODELS_URL`
- `REGION`
- `PROFILE`
- `ACCESS_TOKEN`
- `ACCESS_TOKEN_ENV`
- `ACCESS_KEY_ID`
- `SECRET_ACCESS_KEY`
- `SESSION_TOKEN`

示例：

```bash
export AGENA_PROVIDER__OPENAI__DEFAULT_MODEL=gpt-5
export AGENA_PROVIDER__OPENAI__API_KEY_ENV=OPENAI_API_KEY
export AGENA_PROVIDER__GOOGLE_VERTEX__BASE_URL=https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi
```

## 10. CLI `--set/-c` 覆盖

当前 CLI `-c/--set` 只支持一部分字段，不是完整 schema。

支持项：

- `auth.store_path=<path>`
- `auth.store_backend=auto|file|keyring`
- `tracing.filter=<value>`
- `tracing.database_level=<value>`
- `ui.locale=<value>`
- `runtime.provider_http.timeout_secs=<u64>`
- `runtime.provider_http.connect_timeout_secs=<u64>`
- `runtime.request_retry.max_retries=<u32>`
- `runtime.request_retry.base_delay_ms=<u64>`
- `runtime.request_retry.max_delay_ms=<u64>`
- `runtime.stream_replay.max_retries_after_output=<u32>`
- `runtime.stream_replay.max_tracked_events=<usize>`
- `providers.<id>.default_model=<string>`
- `providers.<id>.base_url=<string>`
- `providers.<id>.api_key=<string>`
- `providers.<id>.api_key_env=<string>`
- `providers.<id>.enabled=true|false`

示例：

```bash
agena \
  -c providers.openai.default_model=gpt-5 \
  -c runtime.provider_http.timeout_secs=60 \
  config resolve --format toml
```

## 11. 校验规则与常见坑

### 11.1 会报错的典型情况

- 顶层 `mode = "..."`
- `[modes.<name>]`
- `AGENA_MODE`
- `-c mode=<name>`
- provider 缺少 `kind`
- 某个 provider kind 缺少必填字段
- `runtime.provider_http.timeout_secs = 0`
- `runtime.provider_http.connect_timeout_secs = 0`
- `runtime.reload.poll_interval_secs = 0`
- `runtime.janitor.interval_secs = 0`
- `runtime.session_cache.max_sessions = 0`
- `runtime.session_cache.ttl_secs = 0`
- `runtime.session_cache.max_bytes = 0`
- `default_thinking` 指向不存在的 `thinking_depths` key
- Bedrock 只写了 `access_key_id` 或只写了 `secret_access_key`
- `providers.<id>.models` 里某个 model id 为空白
- `providers.<id>.models."<model-id>"` 没有设置任何字段

### 11.2 容易误解的地方

- `plugins.paths`：
  - **当前不支持**
  - 要改用 `[plugins.list.<id>]`
- `[plugins.storage]`：
  - `config.full.toml` 注释里提到过
  - **当前 schema 不支持**
- plugin `cdylib/wasm` 的相对 `path`：
  - 相对配置文件目录
- plugin `stdio.command` / `stdio.cwd`：
  - 建议使用绝对路径，避免依赖进程当前目录

## 12. 最小配置示例

```toml
[auth]
store_backend = "auto"

[providers.anthropic]
kind = "anthropic"
base_url = "https://api.anthropic.com/v1"
default_model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"

[runtime]
default_agent = "build"

[agents.plan]
description = "Read-only planning agent"
prompt = "You are a planning agent."
allowed_tools = ["read", "view_file", "glob", "grep", "bash", "todo_write", "enter_plan_mode", "exit_plan_mode"]
mode = "all"

[agents.plan.permission.path]
workspace = { read = "allow", write = "deny" }
external = { read = "ask", write = "ask" }

[agents.plan.permission.tools.first_party]
enter_plan_mode = "allow"
exit_plan_mode = "allow"
todo_write = "allow"
```

## 13. 推荐阅读顺序

如果你是在实际落配置，建议按这个顺序看：

1. `config.example.toml`
2. 本文第 3 节到第 5 节
3. 你实际会用到的 provider kind 小节
4. 如果要扩展工具，再看第 4.9 节、6 节、7 节、8 节
5. 如果要做部署/切环境，再看第 9 节和第 10 节
