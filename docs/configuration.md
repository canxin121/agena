# 配置说明

本文说明 Agena 的持久化设置、环境变量、CLI 覆盖、provider、权限、插件和相关服务参数。配置实现主要在 `crates/agena-runtime/src/config/`，示例文件为仓库根目录的 `config.example.json` 和 `config.full.json`。

## 配置文件

Agena 使用 JSON 配置文件。最小可用配置见仓库根目录的 `config.example.json`，完整功能示例见 `config.full.json`。

建议从最小配置开始：

```bash
mkdir -p ~/agena
cp config.example.json ~/agena/agena.json
agena config validate
```

`config.example.json` 展示了最小启动面：

- `tracing`: 日志过滤。
- `providers.default`: 全局默认 provider 名称。
- `providers.<id>.defaults`: provider-local 默认 adapter/model/thinking/speed/verbosity/parallel 设置。
- `providers.<id>.adapters.<adapter-id>.models."<model-id>".agena_tools.mode`: 该 model route 的唯一工具模式：`provider_protocol`、`prompt_envelope` 或 `disabled`。
- `providers.<id>.adapters.<adapter-id>.models."<model-id>".agena_tools.direct`: 仅对 `provider_protocol` 生效的 Direct Tool presentation policy；可用 canonical-name `*` include/exclude、`max_tools` 和 `max_schema_tokens` 限制模型实际收到的 Direct schema。被限制的工具仍由五个 Tool API gateway 发现/调用，不改变权限或执行可用性。
- `providers.<id>.adapters.<adapter-id>.models."<model-id>".agena_tools.provider_native`: model-scoped Provider 工具路由、默认 hosted 参数、harness 绑定和 connector 引用。
- `providers.<id>.adapters.<adapter-id>.models."<model-id>".native_compaction`: 是否优先使用该路由的 Provider 原生会话压缩接口，默认 `true`；不支持或调用失败时回退到 Agena 文本总结。
- `providers.<id>`: 至少配置一个逻辑 provider，通常由 provider-local `auth` + 一个或多个 `adapters` 组成。
- `providers.<id>.network`: 该 provider 的请求超时和连接超时。
- `runtime.providers.client_versions`: Codex、Claude Code 与 Gemini CLI 的请求身份兼容版本。
- `session.compaction`: 自动 compaction 开关与保留 token 数。
- `permission`: 路径、网络、tool 权限。
- `plugins.list."agena.memory".config` / `plugins.list."agena.web".config` / `plugins.list."agena.mcp".config` / `plugins.list."agena.lsp".config`: 内建 static plugin 的配置。
- `plugins.policy`: 模型工具描述与 TUI plugin/tool 元数据的全局、按 plugin、按 tool 展示策略。
- `harnesses`: browser/shell/editor harness 配置。

`config.full.json` 展示了更完整的功能面：

- provider-local HTTP request/connect timeout。
- permission path/network/tool rules。
- `memory` durable memory / retrieval 配置。
- `web` 本地网页搜索、单页抓取、多页采集 / 索引默认参数。
- plugin transport、restart、storage、marketplace 安装后的配置形态。
- provider model metadata，以及拆分后的 model think/speed modes。

这两个示例文件由仓库的 integration / e2e 配置测试套件覆盖。

## 加载路径与优先级

配置加载入口是 `ConfigLoader`。全局 JSON 配置路径固定为：

```text
~/agena/agena.json
```

工作区可以额外提供一个局部 JSON 配置：

```text
<workspace>/.agena/agena.json
```

两个配置文件都可以只写部分字段。缺失配置文件不是错误。没有文件时，Agena 仍会使用内置默认值、环境变量和 CLI 覆盖解析出配置。

合并优先级从低到高：

1. 内置默认值。
2. 全局 JSON 配置文件 `~/agena/agena.json`。
3. 工作区 JSON 配置文件 `<workspace>/.agena/agena.json`。
4. 环境变量 overlay。
5. CLI 全局 `--set key=value` 覆盖。

配置始终解析为单个生效快照。

工作区配置使用主键边界合并，避免同名实体被跨层深层混合：

- `plugins.list.<id>`: 同 plugin id 整体替换；`plugins.host.quotas.<plugin-id>` 和 `plugins.host.trusted_keys.<key-id>` 按各自主键覆盖，其他 plugin host 标量按字段覆盖。
- `providers.<id>.defaults`: 默认 provider/adapter/model/thinking/speed/verbosity/parallel 选择作为一个元组整体替换。
- `providers.<id>.auth`: auth 配置整体替换。
- `providers.<id>.adapters.<adapter-id>`: adapter 内的标量字段按字段覆盖，`models.<model-id>` 按 model id 整体替换。
- `permission` 和 `harnesses`: 按各自配置里的自然键覆盖，例如 path/network/tool rules、tool tag/name、harness name。
- 其他没有 map 主键的结构字段按已有 partial overlay 语义合并，标量和数组由高优先级覆盖。

## 查看与验证配置

解析并输出最终配置：

```bash
agena config resolve --format json
```

只验证配置是否可加载：

```bash
agena config validate
```

诊断命令会输出全局/工作区配置路径、是否找到配置文件、应用层级、provider 数量、plugin 数量和相关环境变量是否设置：

```bash
agena diagnostics
```

TUI 中的 `/settings` 是唯一配置入口，固定为六个顶层分区：Models & Providers、Permissions、Plugins & Tools、Runtime & Session、Interface、Diagnostics。Permission Studio 与 Plugin Workbench 作为分区内的深入页面保留，不再注册独立的 `/permissions` 或 `/plugins` 命令；故障排除、配置文件入口、tracing 和运行快照状态都集中在 Diagnostics。

Permissions 分区只列出当前会话（存在时）、全局和工作区权限文档，不再额外提供与这些入口重叠的 Manage Permission Rules。Permission Studio 的规则页即使为空也会显示 `+ New Rule`；Tool Tags 和 Tool Names 使用来自当前已注册插件与工具的可搜索多选目录批量创建 `ask` 规则，同时保留自定义名称入口。

### AI settings 工具

内置 `agena.settings` plugin 把同一套配置能力暴露给模型，不要求模型直接用通用文件工具修改 JSON：

- `agena.settings.inspect`：一次查看某个路径在全局文件、工作区文件和最终生效快照中的值，以及实际文件路径和应用层级。
- `agena.settings.get` / `list`：读取单个值或递归枚举路径；`source=file` 时用 `layer=global|workspace` 选择文件，`source=effective` 时读取合并后的运行时快照。
- `agena.settings.set` / `delete` / `patch`：修改 `layer` 指定的全局或工作区配置；默认先验证并在实际变化后 reload。`dry_run=true` 只预览，不写盘。
- `agena.settings.validate`：在全局 + 工作区合并上下文中验证 `layer` 指定的配置文件，工作区 partial overlay 不会被错误地当成独立完整配置。

这些工具可以覆盖 `/settings` 中由 `agena.json` 支持的 provider、permission、plugin、presentation policy、client version、compaction、memory、harness、tracing 和 UI 配置；动态 provider/plugin/tool id 使用与配置文件相同的引号路径语法。`inspect/get` 在读取完整 secret-source 对象时会保留环境变量引用，所有读取工具都会遮蔽 inline API key、OAuth token、credential、password、cookie 等 secret；`list` 的敏感叶节点也会直接遮蔽。

settings 工具不绕过权限系统：

- 每次调用先按工具名和 `settings`、`settings_read` / `settings_write`、`filesystem_read` / `filesystem_write` 等 tag 计算 tool policy。
- 随后按真实全局或工作区配置路径计算 path policy。读取 effective 快照会同时声明全局和工作区两个来源；分层验证同样读取两层。
- 非 dry-run 的 `set/delete/patch` 对实际目标申请 write path，并对另一层申请 read path 以验证合并结果；dry-run 对两层都只申请 read path。默认全局配置位于 workspace 外，因此仍会按 external path policy 询问或拒绝。
- 可以通过 `permission.tools.names."agena.settings.set"` 这类精确 tool-name 规则覆盖某个工具，也可以用 `permission.tools.tags.settings_read` / `settings_write` 做分组策略；path policy 仍然独立生效。

## CLI 覆盖

`agena` 主 CLI 支持全局 `--set key=value`，解析逻辑在
`crates/agena-runtime/src/config_override.rs`；Runtime loader 在加载 raw
configuration 时应用已解析的值。

通用覆盖：

```text
tracing.filter
tracing.database
tracing.adapter
ui.locale
ui.tui.color_scheme
ui.tui.graphics
ui.tui.theme
providers.default
```

Provider 覆盖：

当前 CLI provider 覆盖只接受 canonical 路径。

```text
providers.<id>.defaults.provider
providers.<id>.defaults.adapter
providers.<id>.defaults.model
providers.<id>.defaults.thinking_mode
providers.<id>.defaults.speed_mode
providers.<id>.defaults.verbosity
providers.<id>.defaults.parallel_tool_calls
providers.<id>.auth.base_url
providers.<id>.auth.protocol_paths.<adapter>
providers.<id>.auth.api_key
providers.<id>.enabled
providers.<id>.network.request_timeout_secs
providers.<id>.network.connect_timeout_secs
```

`providers.<id>.auth.api_key` 的 CLI 值可以使用 `env:NAME` 或 `inline:VALUE` 前缀。复杂对象、plugin 配置、permission 和 harness 应通过 JSON、`/settings` 或 `agena.settings.*` 工具修改，不把 CLI `--set` 扩展成第二套完整配置语言。

示例：

```bash
agena \
  --set tracing.filter=debug \
  --set providers.default=openai \
  --set providers.openai.defaults.adapter=openai \
  --set providers.openai.defaults.model=gpt-4.1-mini \
  config resolve
```

## Merge 规则

配置层之间不是简单替换整个文件，而是按类型合并：

- 顶层可选 struct 通常按字段合并。
- map 通常按 key 合并。
- provider config 按字段合并，`auth` 按字段合并，`adapters`、`extra_headers`、`ai_gateway_headers`、`feature_flags` 以及 provider/adapter 的 `models` map 会按 key 扩展或覆盖。
- `plugins` 合并 `host`、`policy` 与 `list`；`policy` 的全局默认值、plugin override 和 tool override 按各自 key 合并。
- plugin 专属配置统一位于 `plugins.list.<id>.config`，host 不再有 `memory`、`web`、`mcp`、`lsp` 顶层配置源。
- `plugins.list` 按 plugin id 合并；每个 plugin 的 `config` 是 plugin 自己的 JSON object，由 plugin manifest 的 JSON Schema 描述和校验。

这些规则由 `crates/agena-runtime/src/config/raw.rs` 中的 `Merge` 实现定义。

## 环境变量

### 核心 overlay

```text
AGENA_LOG
AGENA_DATABASE_LOG
AGENA_LOCALE
AGENA_CODEX_CLIENT_VERSION
AGENA_CLAUDE_CLIENT_VERSION
AGENA_GEMINI_CLIENT_VERSION
AGENA_SESSION_COMPACTION_AUTO
AGENA_SESSION_COMPACTION_RESERVED_TOKENS
```

插件通过 `plugins.list.<id>` 显式配置，插件存储和 marketplace cache 可以通过上面的环境变量改写。

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
- 通过 `--set providers.default=...`、`--set providers.<id>.defaults.model=...` 或 `--set providers.<id>.auth.api_key=env:OPENAI_API_KEY` 这类 canonical override 设置。

### 数据库、Studio、TUI

数据库路径由 `StorageConfig` 从 CLI/env 读取：

```text
AGENA_DATABASE_URL
AGENA_DATABASE_PATH
```

默认数据库路径为 `~/agena/agena.db`，最终 SQLite URL 形如：

```text
sqlite://~/agena/agena.db?mode=rwc
```

Agena server 参数：

```text
AGENA_SERVER_HOST
AGENA_SERVER_PORT
AGENA_SERVER_UI_PASSWORD
AGENA_WORKSPACE_ROOT
AGENA_DATABASE_URL
AGENA_DATABASE_PATH
AGENA_SERVER_UI_DIR
AGENA_SERVER_CORS_ORIGINS
AGENA_SERVER_CORS_ALLOW_ALL
AGENA_SERVER_UI_COOKIE_SAMESITE
```

TUI 参数：

```text
AGENA_LOG_FILE
AGENA_LOG_STDERR
AGENA_TUI_COLOR_SCHEME
AGENA_TUI_THEME
AGENA_TUI_TERMINAL
AGENA_TUI_TERMINAL_VERSION
AGENA_TUI_KEYBOARD_PROTOCOL
AGENA_TUI_OSC52
AGENA_TUI_NATIVE_CLIPBOARD
AGENA_TUI_KITTY_FILE_TRANSFER
AGENA_TUI_DOWNLOAD_DIR
AGENA_TUI_KITTEN
AGENA_TUI_HELPER_TIMEOUT_SECS
```

TUI 默认使用 `auto` 配色模式。启动时唯一一次有界终端协商会同时查询 OSC 11
背景色，并在创建输入事件流之前完整消费响应；终端返回值优先于 `COLORFGBG`、
`TERM_BACKGROUND` 和 `VSCODE_THEME_KIND` 环境提示，无法判断时回退到暗色配色。
主界面不会覆盖终端背景，普通正文继续使用终端默认前景色，只有
强调、状态、弱化文字和选中区域使用经过亮色/暗色对比度校验的语义色。

可以在 `agena.json` 中显式覆盖自动检测：

```json
{
  "ui": {
    "tui": {
      "color_scheme": "auto",
      "graphics": "auto",
      "theme": null
    }
  }
}
```

`color_scheme` 支持 `auto`、`dark` 和 `light`。`graphics` 支持 `auto`、`native`
和 `unicode`，默认 `auto`：在能够验证完整终端链路时协商 Kitty、Sixel 或 iTerm2
原生图片协议，否则使用确定性的 Unicode/文本输出；`native` 用于已经由用户验证
透传链路的环境，`unicode` 会完全关闭原生图形协商。`theme` 是可选的 plugin TUI
theme ID。也可以分别通过 `AGENA_TUI_COLOR_SCHEME`、`AGENA_TUI_GRAPHICS`、
`AGENA_TUI_THEME`，或 `--set ui.tui.color_scheme=light`、
`--set ui.tui.graphics=unicode`、`--set ui.tui.theme=<theme-id>` 设置。TUI 设置页中的
配色改动会立即生效；图形协议只在输入事件流启动前协商一次，因此图形模式改动在
重启 TUI 后生效。

终端识别、键盘协议、剪贴板、Kitty/iTerm2 文件传输以及复用器降级行为见
[`tui-terminal-compatibility.md`](tui-terminal-compatibility.md)。

### 插件、marketplace

```text
AGENA_PLUGIN_STORAGE_DIR
AGENA_MARKETPLACE_DIR
```

`AGENA_PLUGIN_STORAGE_DIR` 覆盖插件存储根目录。默认是 `~/agena/plugin-storage`。

`AGENA_MARKETPLACE_DIR` 覆盖 marketplace cache。默认是 `~/agena/marketplace`。

## Tracing

```json
{
  "tracing": {
    "filter": "info",
    "database": "error"
  }
}
```

默认值：

- `filter = "info"`
- `database = "error"`

`database` 可选：

```text
off
error
warn
info
debug
trace
```

它会独立应用到 `sqlx` 和 `sea_orm`，避免数据库日志淹没主应用日志。

## Provider Auth

provider 凭据的 canonical 位置是 `[providers.<id>.auth]`。常见来源有：

- `api_key`
- `credential`

其中 `credential` 主要用于 provider-local OAuth token。CLI 登录、REST 登录、token refresh 都会直接回写当前 provider 的 `auth`，不会再经过独立的全局 auth store，也不会跨 provider 共享认证状态。

## 运行时身份、会话压缩与 Provider 网络

Agena 只在顶层 `runtime` 暴露 provider 客户端身份版本，在顶层 `session` 暴露 compaction 策略；reload、模型目录缓存、session cache/GC、tool 并发、请求重试和 stream replay 仍由程序内部策略管理，不作为用户调优面暴露。

```json
{
  "runtime": {
    "providers": {
      "client_versions": {
        "codex": "0.144.4",
        "claude": "2.1.209",
        "gemini": "0.50.0"
      }
    }
  },
  "session": {
    "compaction": {
      "auto": true,
      "reserved_tokens": 12000
    }
  }
}
```

版本值必须是最长 128 字符的安全 ASCII 版本标识，只能包含字母、数字、`.`、`-`、`+`、`_`。`session.compaction.auto = false` 会关闭自动压缩，手动压缩不受影响；`reserved_tokens` 可省略，此时按上下文窗口比例自动计算。`/settings` 还可以一次从 npm 刷新三个精确客户端版本。

需要因上游服务差异调整的网络超时属于 provider，本地写在 `providers.<id>.network`：

```json
{
  "providers": {
    "openai": {
      "network": {
        "request_timeout_secs": 120,
        "connect_timeout_secs": 15
      }
    }
  }
}
```

两个值都必须大于 0。CLI 覆盖路径分别是 `providers.<id>.network.request_timeout_secs` 和 `providers.<id>.network.connect_timeout_secs`。

配置文件仍会自动监测并生成新 snapshot；Diagnostics 展示 generation、加载时间、配置来源和终端兼容信息。用户可以观察状态、刷新设置视图和打开配置文件，但不能修改内部缓存、GC、重试或 reload 轮询常量。

## Providers

Provider 定义在 `[providers.<id>]`。当前 canonical 结构是：

- `[providers.default]`：全局默认 provider 名称。
- `[providers.<id>.defaults]`：provider-local 默认 adapter/model/thinking/speed/verbosity/parallel。
- `[providers.<id>.auth]`：认证与身份来源。
- `[providers.<id>.adapters.<adapter-id>]`：协议实现。
- `[providers.<id>.adapters.<adapter-id>.models."<model-id>"]`：真实上游模型节点，以及该 model 的能力覆盖、Agena 工具传输和 Provider 工具配置。

更完整的认证和运行时刷新说明见 [Provider Auth 与 Credential](provider-credentials.md)。

最小示例：

```json
{
  "providers": {
    "default": "openai",
    "openai": {
      "defaults": {
        "adapter": "openai_responses",
        "model": "gpt-5"
      },
      "auth": {
        "mode": "api",
        "subtype": "custom",
        "base_url": "https://api.openai.com",
        "api_key": {
          "kind": "env",
          "value": "OPENAI_API_KEY"
        }
      },
      "adapters": {
        "openai_responses": {
          "enabled": true,
          "models": {
            "gpt-5": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

关键规则：

- `providers.<id>.defaults.adapter` 和 `providers.<id>.defaults.model` 是 provider-local 默认选择。
- `defaults.model` 必须是真实上游 model id。
- adapter 不再有自己的默认模型字段。
- model key 就是真实 model id，不再有 `target_model`。
- `enabled` 可挂在 provider / adapter / model 三层。
- 运行时模型选择由 `provider_id`、`adapter_id`、`model_id` 三个字段共同决定，不使用三段字符串编码。

默认值：

- provider：默认 `enabled = true`
- adapter：默认 `enabled = false`
- model：默认 `enabled = true`
- model 原生压缩：默认 `native_compaction = true`

因此生产配置里建议把实际要启用的 adapter 明确写成 `enabled = true`。

原生会话压缩按 model route 控制。例如只让某个 OpenAI Responses 模型跳过
`/responses/compact`、始终使用 Agena 文本总结：

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai_responses": {
          "enabled": true,
          "models": {
            "gpt-5": {
              "native_compaction": false
            }
          }
        }
      }
    }
  }
}
```

省略该字段或显式设为 `true` 时，compact 会先尝试 Provider 原生接口；Provider
返回不支持、请求失败或结果不能有效缩短 Prompt 时，仍会自动回退到 Agena 文本总结。
设为 `false` 后，运行时也不会继续复用该路由以前生成的 Provider 原生 checkpoint，
而会从 Agena 保存的规范会话历史重新构造 Prompt；后续 compact 直接生成本地文本 checkpoint。

`provider.auth.mode` 可选值：

```text
none
api
credential
```

常用字段：

- `api`：
  - custom subtype：`base_url`、`protocol_paths`、`api_key`
  - gitlab subtype：`access`、`instance_url`、`ai_gateway_url`、`ai_gateway_headers`、`feature_flags`
  - bedrock_sigv4 subtype：`base_url`、`region`、`profile`、`access_key_id`、`secret_access_key`、`session_token`
- `credential`：
  - 通用：`issuer`
  - `openai_chatgpt` / `github_copilot`：`credential`
  - `gitlab`：`credential`、`instance_url`、`ai_gateway_url`、`ai_gateway_headers`、`feature_flags`
  - `google_adc`：`base_url`、`protocol_paths`
  - `sap_ai_core`：`base_url`、`protocol_paths`、`service_key_env`

adapter 常见额外字段：

- 通用：`model_discovery`，默认 `live`；设为 `configured_only` 时不调用远程模型列表，只展示该 adapter 下显式配置的 models。
- `openai_responses`：OpenAI Responses 协议；可配 `backend`、`models_url`、`auth_header`、`auth_scheme`、`capability_family`、`user_agent`、`extra_headers`
- `openai_chat_completions`：OpenAI Chat Completions 协议；可配 `models_url`、`auth_header`、`auth_scheme`、`capability_family`、`user_agent`、`extra_headers`
- `openai_realtime`：OpenAI Realtime WebSocket 协议；可配 `realtime_ws_url`、`models_url`、`auth_header`、`auth_scheme`、`capability_family`、`user_agent`、`extra_headers`
- `anthropic`：`models_url`、`messages_url`、`auth_header`、`auth_scheme`、`extra_beta_header`、`eager_input_streaming`、`user_agent`、`extra_headers`
- `gemini`：`auth_header`、`auth_scheme`、`stream_mode`、`realtime_ws_url`、`user_agent`、`extra_headers`
- `gitlab`：`instance_url`、`ai_gateway_url`、`ai_gateway_headers`、`feature_flags`
- `ollama`：`base_url`

三个 OpenAI adapter 是互斥的 wire protocol 边界，不是同一个 adapter 的运行模式：

- `openai_responses` 请求 `/responses`，使用 `input`、typed Items、`text.format`、`function_call` / `function_call_output` 和 Responses 流事件。
- `openai_chat_completions` 请求 `/chat/completions`，使用 `messages`、`choices`、`response_format`、`tool_calls` 和 Chat Completions delta。
- `openai_realtime` 建立 Realtime WebSocket，会话、对话 item 和响应都通过 Realtime 事件驱动。

配置不再接受旧的 `openai` adapter、`api_mode`、`auto` 协议推断或 Responses 失败后回退 Chat Completions。上游只实现哪套协议，就显式选择对应 adapter；ChatGPT Codex credential 只能使用 `openai_responses` 并设置 `backend = "chatgpt_codex"`。协议差异可对照 OpenAI 官方的 [Migrate to the Responses API](https://developers.openai.com/api/docs/guides/migrate-to-responses) 文档。

HTTP adapter 的 `user_agent` 会覆盖该 adapter 根据 auth credential 优先、
adapter 协议兜底推导出的默认 User-Agent；其他自定义 header 继续通过
`extra_headers` 配置。当前内置 credential 默认包括 OpenAI ChatGPT -> Codex、
Google ADC -> Gemini CLI；其余 auth 按显式 adapter 使用 Codex、Claude Code API
或 Gemini CLI 的官方请求身份。内置默认值使用固定的官方产品版本字符串，
不会把当前 agena 二进制名称或版本作为上游 Agent/CLI 标识符。

### Agena 工具传输

Agena 工具协议明确区分三类对象：

- 真正声明给 Provider tool/function protocol 的只有五个固定 Tool API functions：
  `tools_list`、`tools_search`、`tools_help`、`tools_tags`、`tools_call`。
- `session.rename`、`shell.run` 等是 execution tools（执行工具）；它们的工具名只能出现在
  `tools_help.tool` 或 `tools_call.tool` 中。
- `agena.session.rename` 等是 execution tool 的内部 registry key，不能作为 Provider
  function name 发送。五个 Tool API functions 自身不再携带 plugin key 或点号 handler identity。

Provider 返回的 function name 必须与本次请求声明的 Tool API function 完全一致；点号形式、
首尾空白、未知名称和其他别名都会在本地直接拒绝，不会再转交工具执行。Tool API history
同样只接受显式 `ToolApiFunction` identity 和对应的精确 `tools_*` 名称；旧的点号 handler key
不会被迁移或当作协议别名重放。

在 `provider_protocol` 模式下，发现、帮助、调用和完整输入要求写在这五个 Tool API
functions 自身的 description 与参数 schema 中，不会再追加到 agent/system prompt。
system prompt 只保留固定 Agena 核心指令、适用的 `AGENA.md` 项目指令和用户显式配置的系统指令；Agena 也不会把 execution
tool 名称或摘要索引注入 system prompt。模型需要了解当前能力时，应先调用 `tools_list`，
需要按用途定位目标时调用 `tools_search`，再按需使用 `tools_help` 和 `tools_call`。这样工具
目录变更只影响实时协议定义与发现结果，不会污染或使 system prompt 失效。

有些网关虽然暴露 OpenAI、Anthropic 或 Gemini 的消息接口，却不接受相应 Provider
协议的 `tools` / function declarations，也不会按该协议返回 tool call。对于这类后端，可以在
具体 adapter 的具体 model 上把 `agena_tools.mode` 设为 `prompt_envelope`：

```json
{
  "providers": {
    "message-only-gateway": {
      "defaults": {
        "adapter": "openai_chat_completions",
        "model": "gateway-model"
      },
      "auth": {
        "mode": "api",
        "subtype": "custom",
        "base_url": "https://gateway.example.com",
        "api_key": { "kind": "env", "value": "GATEWAY_API_KEY" }
      },
      "adapters": {
        "openai_chat_completions": {
          "models": {
            "gateway-model": {
              "agena_tools": {
                "mode": "prompt_envelope"
              }
            }
          }
        }
      }
    }
  }
}
```

`agena_tools.mode` 是请求期工具行为的唯一权威字段，有三种取值：

- `provider_protocol`：五个 Tool API functions 通过所选 Provider API 的 tool/function
  protocol 发送定义和调用；配置的 `agena_tools.provider_native` 也只有在该模式下才会发送。execution
  tools 仍由 Agena 执行。这里的 `provider` 只说明传输协议，不表示工具改由 Provider 执行。
- `prompt_envelope`：消息兼容模式。Agena 不向上游发送任何 Provider 工具字段，而是在 system
  prompt 中提供五个 Tool API functions 的名称、说明、输入 JSON Schema 和一个带明确边界的
  JSON 调用协议；历史工具调用和结果也会投影成普通 assistant/user 消息。模型按该
  协议输出后，兼容层会把文本调用转换回标准 Agena tool call，后续权限判断、执行、
  结果持久化以及继续对话仍走原有 session/tool 流水线。
- `disabled`：不向上游发送 Tool API function definitions 或 `agena_tools.provider_native`，不注入提示词
  信封，也不接受该 route 发起新的工具调用。历史工具调用会先降级为普通文本记录，不会继续
  使用 adapter 的工具消息协议；不透明的 Provider continuation ID 也不会跨入该模式。这是缺少
  明确原生 tool-calling 支持时的默认值。

在 `provider_protocol` route 中，`agena_tools.direct` 可以控制 hybrid surface 的 Direct 部分。例如：

```json
{
  "agena_tools": {
    "mode": "provider_protocol",
    "direct": {
      "include": ["agena.fs.*", "agena.shell.*", "agena.interaction.*"],
      "exclude": ["agena.shell.stop"],
      "max_tools": 12,
      "max_schema_tokens": 3200
    }
  }
}
```

include/exclude 使用简单、大小写敏感的 `*` wildcard，canonical name 例如 `agena.fs.read`；exclude 优先。Direct candidates 按 canonical name 稳定排序，在完整 Provider function schema 序列化后按 `ceil(chars / 4)` 的确定性估算累计 token。超出 `max_tools` 或 `max_schema_tokens` 的工具不会消失，而是继续经 `tools_list/search/help/call` 访问。`direct` 不能写在 `prompt_envelope` 或 `disabled` route，以免配置看似生效而实际没有任何 native declaration。

运行时不会根据 capability、请求失败或 Provider 响应在三种 mode 之间自动切换。Provider 模型
refresh 在生成 model route 配置时是唯一的自动分配点：最终 `features` 明确支持
`tool_calling` 时写入 `provider_protocol`；不支持或未知时写入 `disabled`。需要让不支持原生
function calling 的消息模型使用 Agena 工具时，必须显式改为 `prompt_envelope`。配置只接受
`agena_tools.mode`；不存在旧字段或别名兼容。

`prompt_envelope` 是消息后端没有原生 function-definition 通道时的显式兼容例外；只有该模式
必须把五个 Tool API functions 的协议定义编码进提示词。它同样不会注入完整 execution tool
索引，实际工具仍通过 `tools_list` / `tools_search` 发现。

该设置位于 model route，因此同一个 provider 可以让一个 model 使用 Provider 工具协议、
另一个 model 使用提示词信封、第三个 model 完全禁用工具；它同样适用于 `openai_responses`、
`openai_chat_completions`、`anthropic` 和 `gemini` adapter。切换模式会改变 prompt
cache shape，已有 provider continuation 不会跨模式错误复用。

`provider_protocol` 模式严格区分两层名称：Provider 只声明并接受 `tools_list`、
`tools_search`、`tools_help`、`tools_tags`、`tools_call` 五个 function；`fs.read`、
`session.rename` 等点号名称只允许作为 `tools_help` / `tools_call` 的 `tool` 字段。若
Provider 返回了未声明的点号 function、把 Tool API function 错塞进 `tools_call.tool`，
或生成了不完整的 Tool API 参数，Agena 会先截留整个调用批次，并把拒绝原因和精确改写方式
内部返回给模型，最多修复两次。被拒绝的调用不会进入 session、不会执行，也不会把原始
Provider 校验错误直接显示给用户；持续失败时只返回不含调用内容的概括性错误。

`tools_help` 只负责可复用的实时 schema 发现，不创建或消耗执行授权。模型第一次使用某个
execution tool 时，除非当前会话里已经有该精确 identifier 的可复用 help、参数校验失败所附的
完整 help，或可以安全复用相同输入形状的成功调用，否则必须先通过 `tools_list` / `tools_search`
取得实时名称，再调用 `tools_help`，不能根据其他 Agent、产品、版本或历史会话中的记忆猜测
名称和参数。同一次 help 后可以执行多个完整调用。真正的工具权限仍在最终执行边界逐次检查，
和 help 状态无关。

五个 Tool API functions 不属于 execution-tool catalog，不能发现、帮助或调用自身：
`tools_help.tool` 和 `tools_call.tool` 若填写任一 `tools_*` 协议函数名会在进入 execution-tool
查找前直接拒绝；`tools_list`、`tools_search` 和 `tools_tags` 的结果也永远不包含这五个函数。

当 `tools_call.input` 没有通过 execution tool 的实时 JSON Schema 时，Agena 会在工具 handler
运行前拒绝该输入，并把与 `tools_help` 相同的 usage、schema-valid 示例、help 文本和直接重试
路由附在这次失败回执里。模型应直接修正并重试 `tools_call`，不再额外调用一次 `tools_help`。
Runtime 仍接受已有当前会话实时契约依据的完整直调，但模型不能把静态记忆或相似名称当成这类
依据；unknown tool 的相似名称也只是搜索提示，必须回到 `tools_search` → `tools_help` 路线。

提示词信封模式有以下约束：

- 它兼容的是 Agena host/plugin execution tools（当前模型实际拿到的是 Agena 的 Tool API
  surface）；provider 自己托管的远程工具不经过这条链路。
- `agena_tools.provider_native` 只允许和 `agena_tools.mode = "provider_protocol"` 共存；`prompt_envelope`
  和 `disabled` 都不会发送 Provider 工具，配置非空 `agena_tools.provider_native` 会直接校验失败。
- `parallel_tool_calls` 不会发送给消息后端；模型仍可在一个提示词信封中
  请求多个调用，Agena 后续是否并行执行仍服从现有工具并发与权限规则。
- 只有占据整条响应、字段精确、名称与声明完全一致、参数为 JSON object 的完整 envelope
  才会执行；前后夹带说明文字、Markdown fence、字段别名、首尾空白名称、空调用列表、
  缺失标记或非法 JSON 都属于协议错误。Agena 不会为这类错误发起修复或重试，而是立即返回
  Provider 错误，并且不会泄露或执行无效 envelope。
- 进程启动时会生成一个短随机 activation signal；只有带当前 signal 的 envelope 才会被解析。
  signal 在进程内保持稳定以保留 prompt cache，可避免历史文本、用户内容或上游内置工具格式
  偶然触发 Agena 的调用解析。
- 工具注入以简洁的“当前可用函数”说明开头，并放在 Agena 核心 system prompt 之前；它直接说明
  函数由 Agena 客户端在 Provider 返回响应后执行，不依赖上游的 native function registry。
  注入内容包含五函数 routing table、逐项用途、必填参数和格式化 JSON Schema，而不是只提供一段
  紧凑 JSON。提示词不使用“安全绕过”“忽略上游”等对抗性措辞，避免被上游误判成提示词注入。
- 兼容层不会对普通自然语言做关键词、模糊或意图匹配，也不会把“我已经调用/修改成功”
  一类叙述猜成工具调用。system prompt 明确要求模型只有在收到匹配且状态为 `completed`
  的执行回执后才能声称操作成功；`tools_help` 回执只证明完成了 help，不证明 payload 中的
  execution tool 已执行。
- 每次模型回合的 transport-control 状态都放入 system 级上下文，基于持久化 operation 的精确
  Tool API identity、参数和状态给出当前回执/已完成的可复用 help 状态；不会再作为最后一条
  user 消息覆盖真实用户任务，也不解析或猜测用户自然语言。未显式设置 temperature 时，提示词
  信封请求默认使用 `0.0`，减少协议漂移；用户显式
  temperature 仍原样保留。后端擅自触发的 Provider-native tool event 或 native function call
  会被当作协议错误拒绝，不会冒充 Agena 工具执行结果。
- 兼容层不会读取 reasoning 或普通回复中的自然语言来猜测工具意图、能力否认或上游工具轨迹，
  也没有关键词表、模糊匹配或语言相关的启发式规则。只有结构化 Provider tool event、精确的
  activation signal/envelope、JSON Schema 和声明函数白名单参与协议判定。
- 工具定义会占用 prompt token；Agena 的 prompt budget/fingerprint 已把工具定义计入。

Studio Web 的 Provider 创建页可选择 “Prompt envelope”；TUI Provider Studio 的
model 详情页可在“Agena 工具模式”字段选择“提示词信封”。

关于 Anthropic 适配器的认证约束：

- `auth.mode = "api"` 是 Agena 面向 Anthropic 官方一方接口的标准方式，使用 Claude Console API Key。
- `auth.mode = "credential"` 目前只用于 `issuer = "github_copilot"` 的兼容路径。
- Agena 不提供 Claude.ai / Claude Code 订阅 OAuth 登录。对第三方工具场景，官方当前文档要求使用 Claude Console API Key 或受支持的云提供商认证。

### Provider Native Tools

`providers.<id>.adapters.<adapter-id>.models."<model-id>".agena_tools.provider_native` 是 Provider 定义的特殊工具能力及其执行路由的 canonical 配置入口。它和 Agena 管理的 plugin tool 是两条平行链路：

- `agena_tools.mode` 是 model route 的总工具模式；只有 `provider_protocol` 会发送
  `agena_tools.provider_native`，`prompt_envelope` 只暴露 Agena Tool API，`disabled` 不暴露任何工具。
- plugin tool 继续由 Agena 执行，并以 execution tool 名称经五个 Tool API functions 调度；
  plugin tool 名称本身不会成为 Provider function declaration。
- `plugins.list."agena.web".config` 继续表示 Agena 本地 `agena.web` 的 fetch / local crawl-index search。
- `agena_tools.provider_native` 表示 Web Search、Code Execution、Computer Use 等由 Provider API 定义的特殊工具；`routes` 再决定由 Provider 托管、Agena harness 或 connector 执行。

`agena_tools.provider_native` 是唯一合法入口。顶层 `provider_tools`、`provider_native_tools`、
`native_tools` 以及 `agena_tools.transport` 都会作为未知字段直接拒绝，不做迁移或别名兼容。

默认行为不是“一律开启”：

- runtime、TUI 和 Studio Web 都不会根据 auth、base URL 或 adapter 自动推导 Provider 原生工具。
- 所有 provider、gateway 和 proxy 默认都不启用 Provider 原生工具。
- hosted tool preset 只能由用户主动选择；preset 会展开成实际 routes，不作为 runtime 模式保存。
- 保存后这些选择会直接写进 `providers.<id>.adapters.<adapter>.models.<model>.agena_tools.provider_native.*`。
- 不写 `provider_native`（或删除该对象）就是关闭这一层能力；写入该对象后，也只有显式配置的 route 才会生效，没有额外的 `enabled` 开关。

结构分四层：

- `routes`：每个逻辑工具走 `plugin`、`provider_hosted`、`provider_harness`、`provider_connector` 还是 `disabled`。
- `hosted`：provider-hosted tool 的默认资源和参数。
- `harness`：provider-harness tool 绑定到哪个顶层 harness。
- `connectors`：provider 代连远程服务时，引用哪个顶层 MCP server。

当前内置的逻辑工具名：

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

route 约束：

- `web_search`：`disabled` / `plugin` / `provider_hosted`
- `file_search` / `code_execution` / `image_generation` / `url_context`：`disabled` / `provider_hosted`
- `computer` / `bash` / `text_editor`：`disabled` / `provider_harness`
- `remote_mcp`：`disabled` / `provider_connector`

创建界面始终默认不勾选 Provider 原生工具。用户可以为当前 adapter 主动应用可用 preset；
OpenAI hosted tools 只由 `openai_responses` adapter 暴露，Chat Completions 和 Realtime
不会接收 Responses 专属工具形状。

示例：

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai_responses": {
          "models": {
            "gpt-5": {
              "agena_tools": {
                "mode": "provider_protocol",
                "provider_native": {
                  "routes": {
                    "web_search": "provider_hosted",
                    "image_generation": "provider_hosted"
                  },
                  "hosted": {
                    "web_search": {
                      "allowed_domains": [
                        "platform.openai.com",
                        "developers.openai.com"
                      ],
                      "freshness": "cached"
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

`hosted.*.provider_options` 是 escape hatch，用于写 provider-specific 原始 JSON；优先只用 canonical 字段。

当前 adapter/runtime 已经接通的 provider-hosted 组合是：

- OpenAI Responses：`web_search`、`file_search`、`code_execution`、`image_generation`
- Anthropic：`web_search`
- Gemini：`web_search`、`url_context`、`code_execution`

Gemini 的这些 Provider-hosted tools 不能和 Agena 的五个 custom Tool API functions
混在同一次请求中。只要两类声明同时启用，Agena 就会在发送请求前直接报配置错误，避免把
未经支持的组合交给后端后再得到不明确的 400 响应。

OpenAI Responses 的 `image_generation` 有两条共用同一 adapter 能力的真实执行路径：

- 对话内的 provider-native `image_generation_call`：结果只接受 image base64，落入每 workspace/session 的 process-managed local artifact，并回填统一 attachment 的 local path、size 与 SHA-256；解码前后均有 50 MiB 上限。
- 主动的 `agena.image.generate/edit`：plugin 通过 `host/image.execute` 调用当前 session selected provider/model/adapter route；MultiAdapter 先确认该 route 的 `routes.image_generation` 是 `provider_hosted`，再调用 adapter 的 direct image port。OpenAI Responses adapter 在同一个强制 `tool_choice=required` 请求中获得 terminal `image_generation_call` 结果，而不是要求下一轮模型自行决定是否生成。

`image.edit` 只接受已授权的本地 path，或 Host API 内已经物化的 base64/data-URL/local-path image attachment。Host 会统一执行 path permission、普通文件、非空、最多 50 MiB（或更小的 route limit）、base64、图片 signature、MIME、声明 size 与 SHA-256 校验；URL 和 provider file-id 输入会拒绝。输出必须是 base64 image data URL，并在返回 plugin 前复制到 process-managed artifact；临时 URL/file id 不会成为成功结果。当前实现该 direct port 的 adapter 是 OpenAI Responses API route（不包括 ChatGPT Codex 和 GitHub Copilot profile）；其他 Provider 只有在 adapter 真正实现同一 port 后才会通过 active-route capability check。

`remote_mcp`、以及 provider-harness 路径仍只有 canonical 配置模型，当前对话 runtime 还没有把它们投影成完整执行循环；如果为这些 route 写了显式配置，运行时会直接报不支持，而不是静默忽略。

### Harnesses

`provider_harness` 路由不把执行环境挂在 provider 下，而是引用顶层 `harnesses`。原因是 browser / shell / editor 的 host-managed execution environment 属于 host 资产，不属于 provider 账号；这只是配置所有权划分，不代表 Agena 提供 OS 级 sandbox 或强制隔离。

顶层结构：

```json
{
  "harnesses": {
    "browser": {
      "default": {
        "driver": "playwright",
        "headless": true,
        "viewport": {
          "width": 1280,
          "height": 800
        },
        "allowed_domains": [
          "example.com",
          "github.com"
        ]
      }
    },
    "shell": {
      "default": {
        "workspace_only": true,
        "deny_commands": [
          "sudo",
          "rm -rf /"
        ]
      }
    },
    "editor": {
      "default": {
        "workspace_only": true,
        "max_file_bytes": 262144
      }
    }
  }
}
```

provider 侧只保存引用：

```json
{
  "providers": {
    "anthropic": {
      "adapters": {
        "anthropic": {
          "models": {
            "claude-sonnet-4-6": {
              "agena_tools": {
                "mode": "provider_protocol",
                "provider_native": {
                  "routes": {
                    "web_search": "provider_hosted",
                    "bash": "provider_harness",
                    "text_editor": "provider_harness",
                    "computer": "provider_harness"
                  },
                  "harness": {
                    "bash": {
                      "kind": "shell",
                      "name": "default"
                    },
                    "text_editor": {
                      "kind": "editor",
                      "name": "default"
                    },
                    "computer": {
                      "kind": "browser",
                      "name": "default"
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

配置校验会在加载期拒绝：

- 不支持的 route 组合。
- `provider_harness` 但未绑定 harness。
- harness 引用缺失。
- `provider_connector` 但未配置 connector。
- connector 指向不存在的 `mcp.servers.<name>`。

常见示例：

```json
{
  "providers": {
    "openai_chatgpt": {
      "defaults": {
        "adapter": "openai_responses",
        "model": "gpt-5.3-codex"
      },
      "auth": {
        "mode": "credential",
        "issuer": "openai_chatgpt",
        "credential": {
          "type": "oauth",
          "issuer": "openai_chatgpt",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000,
          "account_id": "acct-123"
        }
      },
      "adapters": {
        "openai_responses": {
          "enabled": true,
          "backend": "chatgpt_codex",
          "models": {
            "gpt-5.3-codex": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

```json
{
  "providers": {
    "github-copilot": {
      "defaults": {
        "adapter": "openai_chat_completions",
        "model": "gpt-4o-mini"
      },
      "auth": {
        "mode": "credential",
        "issuer": "github_copilot",
        "credential": {
          "type": "oauth",
          "issuer": "github_copilot",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000
        }
      },
      "adapters": {
        "openai_chat_completions": {
          "enabled": true,
          "models": {
            "gpt-4o-mini": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

```json
{
  "providers": {
    "shared": {
      "defaults": {
        "adapter": "openai_chat_completions",
        "model": "gpt-4.1-mini"
      },
      "auth": {
        "mode": "api",
        "subtype": "custom",
        "base_url": "https://gateway.example.com",
        "api_key": {
          "kind": "env",
          "value": "SHARED_GATEWAY_API_KEY"
        },
        "protocol_paths": {
          "openai": "/v1",
          "anthropic": "/v1",
          "gemini": "/v1beta"
        }
      },
      "adapters": {
        "openai_chat_completions": {
          "enabled": true,
          "models": {
            "gpt-4.1-mini": {
              "enabled": true
            }
          }
        },
        "anthropic": {
          "enabled": true,
          "models": {
            "claude-sonnet-4": {
              "enabled": true
            }
          }
        }
      }
    }
  }
}
```

当一个 auth 网关同时提供多种协议时，`base_url` 表示共享根路径，`auth.protocol_paths` 显式指定每个 adapter 的协议前缀。默认值是：

- `openai = "/v1"`
- `anthropic = "/v1"`
- `gemini = "/v1beta"`

OpenCode Go / Zen 也是这类共享网关：Go 大多数模型走 OpenAI-compatible `/chat/completions`，MiniMax 模型走 Anthropic Messages `/messages`；Zen 还包含 OpenAI Responses 和 Gemini 路由。

### Model metadata 和 modes

canonical 路径是 `providers.<id>.adapters.<adapter>.models."<real-model-id>"`。

示例：

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai_responses": {
          "models": {
            "gpt-4.1-mini": {
              "lifecycle": "active",
              "context_window_tokens": 200000,
              "max_output_tokens": 16384,
              "description": "Fast general-purpose model.",
              "input": {
                "supported": [
                  "text",
                  "image"
                ],
                "unsupported": [
                  "audio"
                ]
              },
              "features": {
                "supported": [
                  "tool_calling",
                  "streaming"
                ],
                "unsupported": [
                  "temperature"
                ]
              },
              "thinking_modes": {
                "default": "low",
                "low": {
                  "display_name": "Light",
                  "strategy": "effort",
                  "effort": "low"
                },
                "high": {
                  "display_name": "Deep",
                  "strategy": "effort",
                  "effort": "high"
                }
              },
              "speed_modes": {
                "default": "fast",
                "fast": {
                  "display_name": "Fast",
                  "request_override": {
                    "body_patch": {
                      "service_tier": "priority"
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

`thinking_modes` 和 `speed_modes` 都是命名 map，键名就是运行时 selector。组内保留键
`default` 引用同一 map 中的 mode；它也可以设为 `null` 来清除继承的默认项。键名只负责
标识 mode，不参与推导行为；每个 thinking mode 都必须显式设置 `strategy` 以及该策略需要
的字段。`default` 必须引用存在且未禁用的 mode。显式运行请求中的 `thinking_mode` 和
`speed_mode` 仍优先于模型默认项。

需要非标准名字或非 effort 策略时，直接在对应键下写扁平字段，例如：

```json
{
  "thinking_modes": {
    "default": "deep",
    "deep": {
      "strategy": "budget",
      "budget_tokens": 16000
    },
    "auto": {
      "strategy": "adaptive",
      "effort": "high",
      "display": "summarized"
    }
  }
}
```

关闭 thinking 也必须显式声明：

```json
{
  "thinking_modes": {
    "off": {
      "strategy": "disabled"
    }
  }
}
```

也就是说，旧的 mode 数组、mode 内的 `thinking: { type, ... }`、mode 内的 `is_default`、
模型级 `default_thinking_mode`，以及省略 `strategy` 后根据键名推导行为的写法都不再接受。

模型节点本身建议只放会影响行为或能力元数据的字段；真正参与路由的是 provider、adapter、model 三级 id。`display_name` 不再作为 model 节点配置字段；mode 的 `display_name` 只用于展示。

`input` 和 `features` 都支持 compact array：

```json
{
  "input": [
    "text",
    "image"
  ],
  "features": [
    "tool_calling",
    "streaming"
  ]
}
```

也可以显式区分 `supported` 和 `unsupported`：

```json
{
  "input": {
    "supported": [
      "text",
      "document"
    ],
    "unsupported": [
      "audio",
      "video"
    ]
  },
  "features": {
    "supported": [
      "reasoning"
    ],
    "unsupported": [
      "temperature"
    ]
  }
}
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

`thinking_modes` 和 `speed_modes` 都是由名称索引的对象，保留键 `default` 用来引用默认
mode。thinking mode 的键名只作为 selector，不推导 strategy 或 effort。每个未禁用的
thinking mode 都必须显式定义 `strategy`；同一个 map 中不会出现重复 selector。

完整写法：

```json
{
  "default": "medium",
  "off": {
    "strategy": "disabled"
  },
  "medium": {
    "strategy": "effort",
    "effort": "medium"
  },
  "budget-4k": {
    "strategy": "budget",
    "budget_tokens": 4096
  }
}
```

`strategy` 可选值：`disabled`、`effort`、`budget`、`adaptive`、`request_only`。

- `disabled`：不需要额外字段。
- `effort`：必须设置 `effort`。
- `budget`：必须设置 `budget_tokens`。
- `adaptive`：可设置 `effort` 和 `display`。
- `request_only`：必须设置 `request_override` 或 `adapter_overrides`。

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
- mode map 的保留键 `default`：值为默认 mode 的键名；不属于单个 mode 对象。
- `disabled`：仅 JSON 配置支持；设为 `true` 时会遮蔽同名的项目、用户或内置 profile，而不是退回下层定义。
- `request_override.headers`
- `request_override.body_patch`
- `adapter_overrides.<adapter>.headers`
- `adapter_overrides.<adapter>.body_patch`

`body_patch` 只能覆盖普通 Provider 请求参数，不能包含顶层 `tools` 或 `functions`。Agena
会在发送请求前拒绝这两个保留字段，避免绕过类型化 Tool API 声明而向 Provider 注入任意
function definition。Agena 管理的五个 Tool API functions 必须来自会话函数集合，Provider
托管工具必须通过 model 的 `agena_tools.provider_native` 配置。

示例：

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai_responses": {
          "models": {
            "gpt-4.1-mini": {
              "speed_modes": {
                "fast": {
                  "display_name": "Fast",
                  "description": "Prefer priority service tier",
                  "request_override": {
                    "body_patch": {
                      "service_tier": "priority"
                    }
                  },
                  "adapter_overrides": {
                    "openai_responses": {
                      "headers": {
                        "openai-beta": "fast-mode-2026-02-01"
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

## Agena identity 与子任务能力

Runtime 只有一个固定主 Agent：`agena`。研究、计划、实现、review 和验证是同一个 Agena 在完成任务时经历的工作阶段，不是可配置或可切换的 Agent profile。配置文件不接受 `[agents]` / `agents.*`，CLI 和 UI 也不提供 Agent 注册、默认值或切换入口。

项目指令由核心 prompt 构造器直接发现，不依赖 memory plugin：用户级文件为 `~/.agena/AGENA.md`；工作区会从当前 workspace 向上查找 `AGENA.md`，按从外到内的顺序注入，越靠近 workspace 的文件在冲突时优先。每个文件最多读取 50,000 bytes，并在 prompt 中保留来源路径。

`agena.tasks.run` / `agena.tasks.create` 创建的是同一个 Agena 的隔离实例。请求可以独立设置模型 `selection`，并通过 `access` 选择：

- `inherit`：继承完整实时工具目录。
- `read_only`：只呈现当前实时 registry 中带 `read_only` tag 的 execution tools；Tool API gateway 仍用于发现和调用这些只读目标。

模型选择、工具 capability、permission policy 和 Agena identity 相互独立。子任务继承父会话的有效 workspace root 与 permission ceiling，不能通过更具体的规则放宽父会话拒绝的权限；read-only 父任务创建的子任务也始终保持 read-only。委派 prompt 只作为子会话的 user message，不会变成另一种 identity。

## Permissions

权限 mode 固定为：

```text
allow
ask
deny
```

顶层权限 schema：

```json
{
  "permission": {
    "path": {
      "workspace": {
        "read": "allow",
        "write": "ask"
      },
      "external": {
        "read": "ask",
        "write": "ask"
      }
    },
    "network": {
      "internet": "ask",
      "private": "ask",
      "loopback": "ask"
    },
    "tools": {
      "default": "ask",
      "tags": {
        "filesystem_read": "allow",
        "filesystem_write": "ask",
        "network": "ask",
        "shell": "ask"
      }
    }
  }
}
```

未显式配置 `permission` 时，Agena 的全局权限默认值是：允许读取当前 workspace，workspace 写入、外部路径读写、网络区域和未覆盖工具调用均为 `ask`。`agena.web.search` 和 `agena.web.fetch` 是例外：它们的只读 tool 调用默认允许，实际 URL 仍逐项服从 network zone 和 network rule，因此把 `internet`、`private`、`loopback` 全部设为 `allow` 后不会再出现一层重复的通用 tool 审批。显式配置的字段会覆盖这些默认值，未配置的字段继续保留默认值。

### Path permission

```json
{
  "permission": {
    "path": {
      "workspace": {
        "read": "allow",
        "write": "ask"
      },
      "external": {
        "read": "ask",
        "write": "deny"
      },
      "rules": {
        "<cwd>/.env*": {
          "read": "ask",
          "write": "deny"
        },
        "<cwd>/secrets/**": "deny",
        "/tmp/allowed/**": "read_write",
        "<home>/Downloads/*.txt": "read"
      }
    }
  }
}
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

```json
{
  "permission": {
    "network": {
      "internet": "ask",
      "private": "deny",
      "loopback": "deny",
      "rules": {
        "github.com:443": "allow",
        "api.github.com": "allow",
        "*.corp.local": "ask",
        "*.corp.local:8443": "allow",
        "10.0.0.0/8": "deny",
        "172.16.0.0/12:*": "ask",
        "fd00::/8": "deny",
        "[::1]:3000": "ask"
      }
    }
  }
}
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

```json
{
  "permission": {
    "tools": {
      "default": "ask",
      "tags": {
        "filesystem_read": "allow",
        "filesystem_write": "ask",
        "network": "ask"
      },
      "names": {
        "agena.shell.run": "ask",
        "agena.fs.read": "ask",
        "example.echo.echo": "ask"
      },
      "rules": {
        "example.echo.echo": {
          "*": "ask"
        },
        "shell": {
          "git status": "allow",
          "git push *": "deny",
          "*": "ask"
        }
      }
    }
  }
}
```

`tags` 用于 tool 没有精确规则时的默认策略。Tool 由 plugin manifest 声明自己的 tags，常见 tags 如 `filesystem_read`、`filesystem_write`、`network`、`internet`、`task`、`shell`。`names` 按 tool 名匹配；runtime-provided and user-configured plugin tools 使用同一个名字表。

`rules.<tool>` 可以直接写 mode，也可以写 pattern table。`shell` 的 pattern table 按实际 shell command 覆盖，`"*"` 是 fallback。其他 tool 使用直接 mode；需要 fallback 时也可以写 `rules.<tool>."*" = "ask"`。

## Memory

```json
{
  "plugins": {
    "list": {
      "agena.memory": {
        "package": {
          "kind": "static"
        },
        "config": {
          "project_instructions": {
            "enabled": true,
            "include_global": true
          },
          "retrieval": {
            "enabled": true,
            "limit": 3,
            "min_query_chars": 8
          }
        }
      }
    }
  }
}
```

`memory` 现在是“文件持久化 + 进程内 Tantivy 检索 + 会话前自动回忆”的组合，不再只是把 `MEMORY.md` 整体注入 prompt。

字段说明：

- `project_instructions.enabled`: 是否启用工作区项目记忆/指令。
- `project_instructions.include_global`: 是否同时读取全局 project instructions。
- `retrieval.enabled`: 会话消息进入模型前，是否基于最后一条用户消息自动检索相关 memory。
- `retrieval.limit`: 自动回忆最多注入多少条命中。
- `retrieval.min_query_chars`: 用户消息短于这个长度时跳过自动回忆。

`plugins.list."agena.memory".config` 会驱动 `agena.memory` 插件；它注册 `search`、`get`、`list`、`write`、`delete` execution tools，模型通过 `tools_call` 运行它们。检索索引是工作区本地的 Tantivy 索引，不需要单独配置服务地址。`search` 和自动回忆会按需从 memory 文件重建索引，因此始终以磁盘上的 memory 文件为准。

## Workflow Tool Search

`agena.tools.search` 现在使用进程内 Tantivy 索引。每次搜索都会基于当前可用的 execution tools 在本地重建索引，因此不依赖 Meilisearch 或其他外部服务。检索会混合精确匹配、ngram、子串、简易模糊匹配和联想补召回；`list`/`search` 默认返回 50 条、最大 100 条；`tag` 之外也支持 `tags` 传多个 tag 做交集过滤。

旧的 external search backend 配置已经移除。当前版本不会读取 `tool_search.url`、`tool_search.api_key`、`tool_search.index` 这些字段。

## Removed: `agena.hooks`

`agena.hooks` 这个配置驱动的 shell/HTTP hook bridge 已移除。旧的两种写法都会报配置错误：

- 顶层 `hooks`
- `plugins.list."agena.hooks"`

如果还需要 run、tool、provider 或 permission 相关 hook 行为，请改成常规 plugin，在 manifest 中声明对应 `hooks` 订阅并实现 plugin SDK 的 hook 接口。

## Plugins

Plugin 是 Agena 的统一能力入口。模型可见 entries、MCP 暴露能力、LSP、skills、memory 等都会通过 plugin 或 plugin tool 接入 runtime。完整体系说明见 [Plugin 体系](plugin.md)。

```json
{
  "plugins": {
    "host": {
      "timeouts": {
        "init": "10s",
        "tool_invoke": "60s",
        "permission_ask": "10s",
        "fast": "500ms"
      },
      "default_quota": {
        "rate_per_sec": 20,
        "burst": 40,
        "max_concurrent": 8
      },
      "quotas": {
        "example.echo": {
          "rate_per_sec": 5,
          "burst": 10,
          "max_concurrent": 2
        }
      },
      "trusted_keys": {
        "acme": "0123456789abcdef..."
      }
    },
    "list": {
      "example.echo": {
        "package": {
          "kind": "stdio",
          "command": "node",
          "args": [
            "./plugins/echo/index.js"
          ],
          "env": {
            "LOG_LEVEL": "info"
          },
          "cwd": ".",
          "sha256": "...",
          "restart": {
            "policy": "on-failure",
            "min_backoff": "1s",
            "max_backoff": "30s",
            "max_retries": 5
          }
        },
        "config": {
          "uppercase": true
        },
        "timeouts": {
          "tool_invoke": "30s"
        }
      }
    }
  }
}
```

顶层 `plugins` 字段：

- `host`
- `policy`
- `list`

`plugins.host` 字段：

- `timeouts`
- `default_quota`
- `quotas`
- `trusted_keys`

每个 plugin 的启用状态、package、transport 和 plugin-specific `config` 都位于 `plugins.list.<id>`。Plugin/tool manifest 可以声明展示默认值，`plugins.policy` 可以在用户配置层覆盖它们：

加载时，`meta/manifest` 的 `namespace.name` 必须与 `plugins.list.<id>` 完全一致，当前只接受
manifest schema version 1；version、tool name、command id 不允许空值或首尾空白，同一
manifest 内的 tool name 和 command id 不允许重复。Tool `input_schema` 以及非空的
`output_schema` 必须是合法的 JSON Schema object 或 boolean；它们作为 execution-tool output
可以描述任意 JSON 形状，只有真正进入 Provider 协议的五个 Tool API functions 会额外强制
object 参数 schema。Host 会先用预取 manifest 校验 plugin config，再调用 `meta/init`；init
返回的 manifest 必须与预取版本完全一致。任何一步失败都会回滚该 plugin 在初始化期间注册
的工具、capability、hook、statusline、theme 和 callback 状态，不会留下半初始化插件。

Plugin command 是用户显式控制和 UI 路由，不是 execution tool。Runtime 不会为 command id
合成 `plugin.command.*` tool 权限，也不会在用户运行 slash command、命令面板 action 或
Studio action 时重复询问是否允许“调用该 command”。`OpenPluginWorkbench`、`Message` 和
`SubmitPrompt` 等本地 effect 可直接返回；需要文件、网络、shell、credential 或其他受保护
副作用时，command 必须返回 `InvokeTool`、调用受权限控制的 Host API，或把执行逻辑迁移到
registered tool。目标 tool 及其 path/network effect 仍完整服从 tool policy、会话 permission ceiling、
持久规则和用户审批；command 不是这些执行权限的绕过通道。

```json
{
  "plugins": {
    "policy": {
      "tool_presentation": {
        "default_mode": "brief",
        "plugins": {
          "agena.settings": "detailed"
        },
        "tools": {
          "agena.settings.get": "brief"
        }
      },
      "ui_presentation": {
        "default_mode": "summary",
        "plugins": {
          "agena.settings": "detailed"
        },
        "tools": {
          "agena.settings.get": "summary"
        }
      }
    }
  }
}
```

`tool_presentation.default_mode` 为 `detailed|brief`，plugin/tool override 为 `tool_default|detailed|brief`。`ui_presentation.default_mode` 为 `detailed|summary`，plugin/tool override 为 `default|detailed|summary`。优先级为 tool override、plugin override、manifest 声明、全局 default。

Plugin transport kind：

- `static`: 编译期注册的 runtime-provided static 插件。
- `cdylib`: 本地动态库。
- `stdio`: 子进程 JSON-RPC over stdin/stdout。
- `http`: 远端 JSON-RPC over POST。
- `wasm`: WebAssembly module。

每种 transport 的字段：

```json
{
  "plugins": {
    "list": {
      "agena.tools": {
        "package": {
          "kind": "static"
        },
        "timeouts": {
          "init": "5s"
        }
      },
      "example.native": {
        "package": {
          "kind": "cdylib",
          "path": "./plugins/native/libnative.so",
          "sha256": "...",
          "signature": {
            "key_id": "acme",
            "signature": "..."
          }
        },
        "config": {
          "mode": "strict"
        },
        "timeouts": {
          "tool_invoke": "20s"
        }
      },
      "example.worker": {
        "package": {
          "kind": "stdio",
          "command": "node",
          "args": [
            "./plugins/worker/index.js"
          ],
          "env": {
            "LOG_LEVEL": "info"
          },
          "cwd": ".",
          "sha256": "...",
          "restart": {
            "policy": "always",
            "min_backoff": "1s",
            "max_backoff": "30s",
            "max_retries": 5
          }
        },
        "config": {
          "project": "rust"
        },
        "timeouts": {
          "tool_invoke": "45s"
        }
      },
      "example.policy": {
        "package": {
          "kind": "http",
          "url": "https://policy.example.com/agena/rpc",
          "auth": {
            "kind": "bearer",
            "token_env": "AGENA_POLICY_TOKEN"
          }
        },
        "config": {
          "org_id": "acme"
        },
        "timeouts": {
          "fast": "2s"
        }
      },
      "example.sandboxed": {
        "package": {
          "kind": "wasm",
          "path": "./plugins/sandboxed/plugin.wasm",
          "sha256": "..."
        },
        "config": {},
        "timeouts": {
          "init": "20s"
        }
      }
    }
  }
}
```

`config` 是传给 plugin 的自由 JSON 配置；runtime-provided static plugin 也通过 `config` 接收自己的配置。

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

Timeout 值在 JSON 中写字符串。不写单位时按秒解析，例如 `"30"` 等价于 `"30s"`。

Quota 字段：

```json
{
  "plugins": {
    "host": {
      "default_quota": {
        "rate_per_sec": 20,
        "burst": 40,
        "max_concurrent": 8
      },
      "quotas": {
        "example.cloud-policy": {
          "rate_per_sec": 5,
          "burst": 10,
          "max_concurrent": 2
        }
      }
    }
  }
}
```

`rate_per_sec = 0` 表示不限制速率；`burst = 0` 表示使用 `rate_per_sec`；`rate_per_sec` 和 `burst` 都为 0 时关闭 token bucket。`max_concurrent = 0` 表示不限制并发。

Stdio restart policy：

- `never`
- `on-failure`
- `always`

默认是 `on-failure`、最小 backoff 1s、最大 backoff 30s、最多 5 次 retry。

HTTP plugin auth 支持：

```json
[
  { "auth": { "kind": "none" } },
  { "auth": { "kind": "bearer", "token": "..." } },
  { "auth": { "kind": "bearer", "token_env": "PLUGIN_TOKEN" } },
  { "auth": { "kind": "basic", "username": "user", "password": "..." } },
  { "auth": { "kind": "basic", "username": "user", "password_env": "PLUGIN_PASSWORD" } }
]
```

Runtime-provided static plugins 由 runtime 注册，包括文件系统、shell、web、plan、skills、LSP、cron、memory、MCP、settings 等。它们和用户配置的 plugin 一样进入 plugin host 与 tool registry。

### Skills plugin catalog policy

`agena.skills` 的 plugin-specific config 只控制 filesystem-backed Skill/command 的发现策略。标准 roots 始终保留；`disabled` 按 canonical name 从 `skills.list/get` 中移除。额外 roots 只接受 workspace-relative 路径，且拒绝空值、绝对路径与 `..`；若 root 已存在，还会 canonicalize 并拒绝解析到 workspace 外的 symlink，因此项目配置不能把任意用户目录静默作为模型可读 prompt 内容。

```json
{
  "plugins": {
    "list": {
      "agena.skills": {
        "package": { "kind": "static" },
        "config": {
          "disabled": ["legacy_deploy"],
          "additional_roots": ["team/skills"],
          "additional_command_roots": ["team/commands"],
          "watcher": {
            "enabled": true
          }
        }
      }
    }
  }
}
```

`skills.refresh` 和每次 Skills Tool 都会重新 discovery，并用 catalog fingerprint 与 monotonic generation 说明是否发生变化。默认启用的 `watcher.enabled` 使用平台 filesystem notification（macOS FSEvents、Linux inotify、Windows 等由 `notify` 适配）监听标准及额外 root；它只递增 catalog invalidation generation，绝不在后台读取或注入 Skill。下一个正常 Tool 调用边界仍进行完整 discovery 和 content-hash 计算。不存在的 root 会对最近存在父目录做 non-recursive 监听，以发现目录创建而避免递归监控整个 workspace；创建后的下一次刷新会重新扫描实际 root。关闭 watcher 不会关闭 request-driven refresh。

Skill 是纯文本 instruction package，没有 active status、session 持久状态、隐式路径激活、工具 allowlist 或模型切换。用户通过 `/skill` 明确附加 Skill；AI 需要 reusable workflow 时，可以通过实时 Tool API 发现 `agena.skills.list/get`，读取完整 body 后直接应用到当前任务。

### Message-scoped Skill references

聊天输入框的 `/skill` 会打开真实 `agena.skills.list` catalog 的分页选择器；“Attach Skill”按钮使用相同入口。选择一项时，Web 客户端通过 `agena.skills.get` 读取 canonical name、description、instructions、source、aliases 与 exact content hash，并把这一确切版本作为 `skill_reference` message part 放入 composer。它与文件/图片附件一样可以在发送前移除、进入 pending message queue，并在已发送用户消息上留下可见的 Skill chip。

`skill_reference` 保存的是发送时的不可变快照，而不是只保存一个以后可能解析到不同内容的名称。因此会话持久化、导出/导入、回放、prompt digest 与 compaction 都能看到用户当时真正选择的 instructions；后续编辑或删除 `SKILL.md` 不会静默改变历史消息的语义。Provider 投影会把快照编码为 `<agena_skill_references>` 用户消息上下文，核心 system prompt 同时解释了该结构，要求模型真正按 Skill 完成任务而不是只复述其内容。

这个入口是消息级纯文本引用，不会产生隐藏的 session 状态或运行时副作用。单条消息最多引用 8 个 Skill；每项 instructions 最多 64 KiB，全部引用的 instructions 最多 256 KiB，完整引用 payload 还受每项 80 KiB、合计 384 KiB 上限保护。

### Plugin-contributed Skills

Plugin manifest schema v1 可以声明 `skills` 数组。每项是 self-contained 的 `name`、`description`、`instructions` 和 `aliases`；它不接受由插件提供的任意扫描目录。Host 会验证 plugin 内 canonical/alias lookup name 唯一以及 instructions/description 上限。Agena 把这些声明标为 `plugin:<id>@<version>` provenance，并禁止 `skills.read_resource` 借此读取任意 package 文件。

插件存储默认目录是 `~/agena/plugin-storage`，可通过 `AGENA_PLUGIN_STORAGE_DIR` 覆盖。插件 secret 默认使用 `agena.plugin` keyring service，并可 fallback 到文件。

## MCP

MCP server 配置位于 `plugins.list."agena.mcp".config`。Runtime 会把配置后的 MCP servers 通过 `agena.mcp` static plugin 暴露成 plugin tools，并统一进入 plugin host 和 tool registry。

Stdio:

```json
{
  "plugins": {
    "list": {
      "agena.mcp": {
        "package": {
          "kind": "static"
        },
        "config": {
          "servers": {
            "filesystem": {
              "transport": "stdio",
              "process": {
                "command": "npx",
                "args": [
                  "-y",
                  "@modelcontextprotocol/server-filesystem",
                  "."
                ],
                "env": {},
                "cwd": "."
              }
            }
          }
        }
      }
    }
  }
}
```

HTTP:

```json
{
  "plugins": {
    "list": {
      "agena.mcp": {
        "package": {
          "kind": "static"
        },
        "config": {
          "servers": {
            "remote": {
              "transport": "http",
              "endpoint": {
                "url": "https://mcp.example.com",
                "headers": {}
              },
              "auth": {
                "kind": "bearer_from_env",
                "env": "MCP_TOKEN"
              }
            }
          }
        }
      }
    }
  }
}
```

MCP server transport：

```text
stdio
http
```

`stdio` server 的字段：

```text
process.command
process.args
process.env
process.cwd
```

`http` server 的字段：

```text
endpoint.url
endpoint.headers
auth
```

HTTP transport 固定使用 streamable HTTP，不再支持 `mode` 字段，也不再支持 `ws` / `websocket` transport。`headers` 是普通 header map，`auth` 可以省略。

HTTP auth:

```json
[
  { "auth": { "kind": "bearer", "token": "..." } },
  { "auth": { "kind": "bearer_from_env", "env": "MCP_TOKEN" } },
  { "auth": { "kind": "bearer_from_store" } },
  { "auth": { "kind": "oauth", "scopes": ["mcp:read"] } },
  { "auth": { "kind": "custom", "headers": { "X-Token": "..." } } }
]
```

`runtime.token_store` controls where `bearer_from_store` credentials are read:

```json
{
  "runtime": {
    "token_store": {
      "enabled": true,
      "backend": "keyring",
      "file_fallback": false
    }
  }
}
```

`keyring` is the default and stores MCP bearer credentials under the dedicated
`agena.mcp` system-keyring service; they are not written to `agena.json` or
shown by settings inspection. `file` selects the old `~/agena/mcp-tokens.json`
compatibility backend (chmod `0600` on Unix). For a controlled migration,
`file_fallback: true` makes a keyring-backed setup read that legacy file only
when the keyring has no value or is unavailable; it does not write new
credentials to the file. Keep it disabled for normal deployments.

`oauth` stores only requested scopes in configuration. The registered client,
access token and refresh token are serialized as one record in the same
`agena.mcp` system-keyring service, under a server-name hash with a distinct
`mcp-oauth-v1-` prefix. They are never written to `agena.json`, output by
`status`, or included in logs. The runtime uses protected-resource metadata /
authorization-server discovery, S256 PKCE, dynamic client registration and
automatic refresh through the MCP SDK.

### MCP CLI 管理

`agena mcp` 是配置、连接状态和 bearer credential 的管理面：

```text
agena mcp status
agena mcp list
agena mcp get <server>
agena mcp add <server> --url https://mcp.example.com --auth bearer-from-store
agena mcp add <server> --url https://mcp.example.com --auth oauth --scope mcp:read
agena mcp add <server> --url https://mcp.example.com --include-tool 'repo_*' --exclude-tool repo_delete
agena mcp add <server> --command npx --arg -y --arg @scope/mcp-server
agena mcp remove <server>
agena mcp enable
agena mcp disable
agena mcp reconnect <server>
printf '%s' "$MCP_TOKEN" | agena mcp login <server> --token-stdin
agena mcp logout <server>
agena mcp logout <server> --oauth
agena mcp logout <server> --oauth --revoke --url https://mcp.example.com
```

`add/remove/enable/disable` 默认编辑全局 `~/agena/agena.json`，可通过
`--layer workspace` 改写工作区 `.agena/agena.json`；它们支持 `--dry-run`
和 `--no-reload`。`add` 始终维护 `plugins.list."agena.mcp"` 为 static
plugin record，保留同一记录中的其他字段。HTTP URL 不能包含 user/password，
`--header Authorization=...` 也会被拒绝；应使用 `--auth bearer-from-store`
配合 `mcp login --token-stdin`（默认 keyring），或 `--auth bearer-from-env`
配合 `--auth-env NAME`。`mcp login --token` 仅为受控自动化保留；交互和 shell
脚本优先使用标准输入，以免 token 落入 shell history 或进程参数列表。

`--include-tool` 和 `--exclude-tool` 可以重复使用，并接受 `*` 通配符。
如果 include 非空，只有匹配项可被发现和调用；exclude 永远优先。此策略在
MCP manager 内对初始 `tools/list`、list-changed refresh、`tools.search`、手动 refresh
和实际 `tools.call` 共用同一判断，因而不是只隐藏模型可见名称的 UI 过滤。

OAuth login 使用 loopback callback：

```bash
agena mcp login <server> --browser --url https://mcp.example.com --scope mcp:read
```

CLI 会输出 authorization URL 并在 `127.0.0.1:1455`（可用 `--port` 覆盖）
等待五分钟。回调中的 `state` 和可选 RFC 9207 `iss` 均会交由 OAuth state
machine 验证；失败、超时或 issuer 不匹配不会写入凭据。OAuth 与手动 bearer
是独立的 keyring records，`mcp logout <server> --oauth` 只删除 OAuth record。

需要同时撤销授权服务器中的 credential 时，显式使用
`mcp logout <server> --oauth --revoke --url <MCP_ENDPOINT>`。命令会重新从所给
MCP resource endpoint 发现 authorization-server metadata，只在服务器公布可选的
RFC 7009 `revocation_endpoint` 时发送 `application/x-www-form-urlencoded` 的撤销请求；
请求禁止跨 origin redirect，远端返回成功后才删除本地 keyring record。没有发布
endpoint、远端请求失败或本地删除失败都会保留本地 record 以供重试。`--url` 不能单独
使用，也不会从某个不相关的 config layer 猜测远端 authority；普通
`mcp logout <server> --oauth` 始终是无网络副作用的本地删除。

bearer 与 OAuth 的 keyring records 被故意隔离，runtime 不会因为两者同时存在而
静默选择、合并或迁移。`mcp status` / `mcp.servers.status` 对已经配置 OAuth 但仍有
手动 bearer record（或反向的 `bearer-from-store` 配置仍存 OAuth record）会返回不含
secret 的 `credential_migration` advisory 与清理建议。推荐顺序是：先显式切换配置、
连接验证成功，再由用户执行对应的 `mcp logout` 删除旧 record；绝不在连接过程中自动
清除或使用另一种 credential。

`status/list/get` 的输出只投影连接状态、server 名、工具数量、network target、
generation、错误和 reconnect supervisor 状态；它不会输出 authorization header、
bearer token 或 initialization instructions 本文。

配置了 MCP server 时，runtime 会构建 `McpConnectionManager`，并注册 MCP static plugin。

## LSP

LSP server 配置位于 `plugins.list."agena.lsp".config`：

```json
{
  "plugins": {
    "list": {
      "agena.lsp": {
        "package": {
          "kind": "static"
        },
        "config": {
          "servers": {
            "rust": {
              "process": {
                "command": "rust-analyzer",
                "args": [],
                "env": {}
              },
              "routing": {
                "file_extensions": [
                  "rs"
                ],
                "root_markers": [
                  "Cargo.toml"
                ]
              },
              "session": {
                "initialization_options": {}
              }
            }
          }
        }
      }
    }
  }
}
```

LSP server 字段：

```text
process.command
process.args
process.env
routing.file_extensions
routing.root_markers
session.initialization_options
```

`file_extensions` 不带前导 `.`；写空数组表示该 server 匹配所有文件。`root_markers` 是用于识别项目根目录的文件名列表。`initialization_options` 是传给 language server 的 JSON object。可选的 `defaults.env`、`defaults.root_markers` 和 `defaults.initialization_options` 会作为所有 server 的回退值；每个 server 的 `process`、`routing`、`session` 都是独立嵌套对象。

LSP registry 是 lazy-spawn 的。相关 tool 首次触及匹配文件时才会启动对应 server。

## Web

```json
{
  "plugins": {
    "list": {
      "agena.web": {
        "package": {
          "kind": "static"
        },
        "config": {
          "fetch_enabled": true,
          "default_max_pages": 10,
          "max_pages_limit": 100,
          "default_max_depth": 1,
          "max_depth_limit": 4,
          "default_same_host_only": true,
          "request_delay_ms": 400,
          "fetch_timeout_secs": 30,
          "max_body_bytes": 5242880,
          "respect_robots_txt": true,
          "document_cache_ttl_secs": 86400,
          "fetch_cache_ttl_secs": 900,
          "fetch_cache_capacity": 128,
          "store_max_documents": 200,
          "store_max_bytes": 104857600,
          "default_chunk_chars": 1800,
          "near_duplicate_hamming_distance": 3,
          "search_default_limit": 5,
          "search_max_limit": 20,
          "list_default_limit": 20,
          "list_max_limit": 100,
          "browser_enabled": false,
          "browser_wait_for_network_idle": true,
          "browser_wait_timeout_secs": 10,
          "browser_wait_for_delay_ms": 0
        }
      }
    }
  }
}
```

`plugins.list."agena.web".config` 控制内建 `agena.web` plugin 的默认行为。这个 plugin 是现在推荐的本地网页入口：

- `fetch`: 单页抓取，带进程内 TTL cache，但不会写入本地索引。
- `crawl`: 多页抓取、用 Spider 抓取页面、用 CRW extract 抽取 Markdown、落盘、维护 metadata、去重，并重建本地 Tantivy 索引。
- `search`: 直接调用内置 ferris-style 搜索实现做网页搜索，当前支持 `bing`、`duckduckgo`、`baidu`，不需要 API key，也不启动外部服务。
- `query`: 查询当前 workspace 的本地 crawl 语料；使用 Tantivy BM25 / ngram 全文检索，不加载 embedding 或 rerank 模型。
- `get` / `list`: 检查已保存文档。

字段说明：

```text
default_max_pages
max_pages_limit
default_max_depth
max_depth_limit
default_same_host_only
request_delay_ms
fetch_timeout_secs
max_body_bytes
respect_robots_txt
document_cache_ttl_secs
fetch_cache_ttl_secs
fetch_cache_capacity
store_max_documents
store_max_bytes
default_chunk_chars
near_duplicate_hamming_distance
search_default_limit
search_max_limit
list_default_limit
list_max_limit
browser_enabled
browser_executable_path
browser_wait_for_network_idle
browser_wait_timeout_secs
browser_wait_for_selector
browser_wait_for_delay_ms
```

说明：

- `web` 的 `crawl` 和 `fetch` action 都使用 Spider 抓取。
- `respect_robots_txt` 打开后 Spider 会按 robots.txt 约束抓取。
- `document_cache_ttl_secs` 控制已保存文档在后续 crawl 中被直接复用的时长。
- `fetch_cache_ttl_secs` / `fetch_cache_capacity` 控制进程内 HTTP fetch cache。
- `store_max_documents` / `store_max_bytes` 控制本地 crawl cache 的上限；每次 `crawl` 写入后会删除最旧文档并重建索引，避免本地目录无限增长。
- `near_duplicate_hamming_distance` 用于基于 SimHash 的近重复过滤。
- `web search` 的 `engine` 参数由 AI 在调用时选择：`auto`、`duckduckgo`、`bing` 或 `baidu`。省略或传 `auto` 时会按 `duckduckgo -> bing -> baidu` 自动尝试，直到拿到结果。
- `browser_enabled` 会让 `web fetch` / `web crawl` 默认通过 Agena 托管的本地 Chrome/Chromium 进程抓取 JS 页面；单次 tool call 也可以用 `render_js` 覆盖。
- `browser_executable_path` 可选，用于指定本地 Chrome/Chromium 可执行文件路径；不支持配置远端 DevTools/WebSocket 链接。
- `browser_wait_for_network_idle`、`browser_wait_timeout_secs`、`browser_wait_for_selector` 和 `browser_wait_for_delay_ms` 控制渲染等待策略。

如果你只是想临时拉一页内容，用 `web` 的 `fetch` action；如果你需要站内多页抓取、复用本地 crawl cache、或者想避免再接 Firecrawl 这类远程服务，用 `web` 的 `crawl` action。

如果要启用 OpenAI / Anthropic / Gemini 定义的 Provider 工具，不要写在 `agena.web` plugin config，而是写在 `providers.<id>.adapters.<adapter>.models.<model>.agena_tools.provider_native.*`。

## Studio 服务配置

Server 是 `agena` 二进制，参数定义在 `crates/agena-cli/src/cli/mod.rs`。

常用启动：

```bash
agena \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace-root "$PWD"
```

服务参数：

```text
--set key=value
--host / AGENA_SERVER_HOST
--port / AGENA_SERVER_PORT
--ui-password / AGENA_SERVER_UI_PASSWORD
--workspace-root / AGENA_WORKSPACE_ROOT
--database-url / AGENA_DATABASE_URL
--database-path / AGENA_DATABASE_PATH
--ui-dir / AGENA_SERVER_UI_DIR
--cors-origin / AGENA_SERVER_CORS_ORIGINS
--cors-allow-all / AGENA_SERVER_CORS_ALLOW_ALL
--ui-cookie-samesite / AGENA_SERVER_UI_COOKIE_SAMESITE
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

- Loader、默认路径、优先级: `crates/agena-runtime/src/config/loader.rs`
- Raw schema、env overlay、merge、validation: `crates/agena-runtime/src/config/raw.rs`
- Resolved schema 和默认值: `crates/agena-runtime/src/config_values.rs`
- CLI override value/parser: `crates/agena-runtime/src/config_override.rs`
- CLI/raw-schema override application: `crates/agena-runtime/src/config/overrides.rs`
- Provider registry materialization: `crates/agena-runtime/src/config/registry.rs`
- Auth store: `crates/agena-runtime/src/provider/auth/store.rs`
- Runtime builder/snapshot/reload: `crates/agena-runtime/src/runtime/`
- Permission schema/principal: `crates/agena-runtime-contracts/src/authorization/mod.rs`；permission policy 位于 `crates/agena-runtime-contracts/src/permission/`
- Plugin config: `crates/agena-plugin-host/src/config.rs`
- Plugin storage: `crates/agena-runtime/src/plugins/storage.rs`
- Server args: `crates/agena-cli/src/cli/mod.rs`
