# 配置说明

本文说明 Agena 的运行时配置、环境变量、CLI 覆盖、provider、权限、插件和相关服务参数。配置实现主要在 `crates/agena/src/config/`，示例文件为仓库根目录的 `config.example.json` 和 `config.full.json`。

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
- `desktop`: Agena Desktop 壳自己的启动配置，和主运行时配置一起保存在同一个 `agena.json` 里。
- `providers.default`: 全局默认 provider 名称。
- `providers.<id>.defaults`: provider-local 默认 adapter/model/thinking/speed/verbosity/parallel 设置。
- `providers.<id>.adapters.<adapter-id>.models."<model-id>".native_tools`: model-scoped provider-native 远程内置 tool 路由、默认 hosted 参数、harness 绑定和 connector 引用。
- `providers.<id>`: 至少配置一个逻辑 provider，通常由 provider-local `auth` + 一个或多个 `adapters` 组成。
- `runtime`: provider HTTP、retry、reload、catalog 等运行时基础设施参数。
- `runtime.session`: session cache、session gc。
- `session`: compaction。
- `agents.default`: 全局默认 agent 名称。
- `agents.<name>`: 自定义 agent。
- `permission`: 路径、网络、tool 权限。
- `plugins.list."agena.memory".config` / `plugins.list."agena.web".config` / `plugins.list."agena.mcp".config` / `plugins.list."agena.lsp".config`: 内建 static plugin 的配置。
- `harnesses`: browser/shell/editor harness 配置。

`config.full.json` 展示了更完整的功能面：

- provider HTTP timeout、retry、stream replay。
- runtime reload、runtime.session.gc、runtime.session.cache。
- permission path/network/tool rules。
- `memory` durable memory / retrieval 配置。
- `web` 本地网页搜索、单页抓取、多页采集 / 索引默认参数。
- plugin transport、restart、storage、marketplace 安装后的配置形态。
- provider model metadata，以及拆分后的 model thinking/speed modes。

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

- `agents.<name>`: 同名 agent 整体替换，`agents.default` 单独按标量覆盖。
- `plugins.list.<id>`: 同 plugin id 整体替换；`plugins.host.quotas.<plugin-id>`、`plugins.host.trusted_keys.<key-id>`、`plugins.policy.tool_presentation.plugins.<plugin-id>` 和 `plugins.policy.tool_presentation.tools.<tool>` 按各自主键覆盖，其他 plugin host/policy 标量按字段覆盖。
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

## CLI 覆盖

`agena` 主 CLI 支持全局 `--set key=value`，解析逻辑在 `crates/agena/src/config/overrides.rs`。

通用覆盖：

```text
tracing.filter
tracing.database
ui.locale
providers.default
agents.default
runtime.providers.http.timeout_secs
runtime.providers.http.connect_timeout_secs
runtime.providers.retry.max_retries
runtime.providers.retry.base_delay_ms
runtime.providers.retry.max_delay_ms
runtime.providers.stream_replay.max_retries_after_output
runtime.providers.stream_replay.max_tracked_events
runtime.model_catalog.cache_max_age_secs
runtime.session.cache.max_sessions
runtime.session.cache.ttl_secs
runtime.session.cache.max_bytes
runtime.session.gc.enabled
runtime.session.gc.interval_secs
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
providers.<id>.auth.api_key.kind
providers.<id>.auth.api_key.value
providers.<id>.enabled
providers.<id>.adapters.<adapter>.models.<model>.native_tools.enabled
providers.<id>.adapters.<adapter>.models.<model>.native_tools.routes.web_search
providers.<id>.adapters.<adapter>.models.<model>.native_tools.hosted.web_search.allowed_domains
providers.<id>.adapters.<adapter>.models.<model>.native_tools.harness.computer.kind
providers.<id>.adapters.<adapter>.models.<model>.native_tools.connectors.<name>.server
harnesses.browser.<name>.driver
```

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
- `plugins` 只合并 `timeouts`、`list`、`trusted_keys`、`default_quota` 和 `tool_presentation`；没有总开关。
- plugin 专属配置统一位于 `plugins.list.<id>.config`，host 不再有 `memory`、`web`、`mcp`、`lsp` 顶层配置源。
- `plugins.list` 按 plugin id 合并；每个 plugin 的 `config` 是 plugin 自己的 JSON object，由 plugin manifest 的 JSON Schema 描述和校验。

这些规则由 `crates/agena/src/config/raw.rs` 中的 `Merge` 实现定义。

## 环境变量

### 核心 overlay

```text
AGENA_LOG
AGENA_DATABASE_LOG
AGENA_LOCALE
AGENA_PROVIDER_HTTP_TIMEOUT_SECS
AGENA_PROVIDER_CONNECT_TIMEOUT_SECS
AGENA_PROVIDER_REQUEST_MAX_RETRIES
AGENA_PROVIDER_RETRY_BASE_DELAY_MS
AGENA_PROVIDER_RETRY_MAX_DELAY_MS
AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES
AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS
AGENA_MODEL_CATALOG_CACHE_MAX_AGE_SECS
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
- 通过 `--set providers.default=...`、`--set agents.default=...`、`--set providers.<id>.defaults.model=...` 或 `--set providers.<id>.auth.api_key=env:OPENAI_API_KEY` 这类 canonical override 设置。

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
```

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

## Runtime

```json
{
  "runtime": {
    "providers": {
      "http": {
        "timeout_secs": 120,
        "connect_timeout_secs": 15
      },
      "retry": {
        "max_retries": 5,
        "base_delay_ms": 250,
        "max_delay_ms": 2000
      },
      "stream_replay": {
        "max_retries_after_output": 5,
        "max_tracked_events": 2048
      }
    },
    "reload": {
      "enabled": true,
      "poll_interval_secs": 2
    },
    "model_catalog": {
      "cache_max_age_secs": 604800
    }
  }
}
```

`runtime` 只放基础设施参数：

- `runtime.providers.http`：provider HTTP client 超时。
- `runtime.providers.retry`：请求重试退避。
- `runtime.providers.stream_replay`：流式 replay-safe 重试。
- `runtime.reload`：配置文件变更轮询。
- `runtime.model_catalog`：公共模型目录缓存过期。
- `runtime.session.cache`：session cache 的容量和 TTL。
- `runtime.session.gc`：session cache 的定期清理任务。

`runtime` 里的这些值只影响运行时基础设施，不决定默认 provider、默认 adapter/model 或默认 agent。那部分分别由 `providers.default`、`providers.<id>.defaults` 和 `agents.default` 管。

校验规则：

- provider HTTP timeout 和 connect timeout 必须大于 0。
- reload poll interval 必须大于 0。
- runtime session cache TTL、max sessions、max bytes 必须大于 0。
- runtime session GC interval 必须大于 0。
- model catalog cache max age 必须大于 0。
- `runtime.providers.retry.max_delay_ms` 会至少等于 `base_delay_ms`。

Runtime 会根据配置构建 snapshot。手动 reload 或配置文件变更触发 reload 时，新的 snapshot 会重新构建 provider registry、plugin host、agent registry、MCP/LSP registry 等服务。

## Providers

Provider 定义在 `[providers.<id>]`。当前 canonical 结构是：

- `[providers.default]`：全局默认 provider 名称。
- `[providers.<id>.defaults]`：provider-local 默认 adapter/model/thinking/speed/verbosity/parallel。
- `[providers.<id>.auth]`：认证与身份来源。
- `[providers.<id>.adapters.<adapter-id>]`：协议实现。
- `[providers.<id>.adapters.<adapter-id>.models."<model-id>"]`：真实上游模型节点，以及该 model 的 native tools 配置。

更完整的架构说明见 [Provider / Auth / Adapter 架构](provider-auth-adapters.md)。

最小示例：

```json
{
  "providers": {
    "default": "openai",
    "openai": {
      "defaults": {
        "adapter": "openai",
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
        "openai": {
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

因此生产配置里建议把实际要启用的 adapter 明确写成 `enabled = true`。

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

### Provider-native tools

`providers.<id>.adapters.<adapter-id>.models."<model-id>".native_tools` 是 provider 原生远程内置 tool 的 canonical 配置入口。它和 `agena.web` plugin tool 是两条平行链路：

- plugin tool 继续表示 host-executed function tools。
- `plugins.list."agena.web".config` 继续表示 Agena 本地 `agena.web` 的 fetch / local crawl-index search。
- `providers.<id>.adapters.<adapter-id>.models."<model-id>".native_tools` 表示某个具体 model 自己托管或 provider 规划、host 执行的 native tools。

默认行为不是“一律开启”：

- runtime 解析阶段不会再根据 auth 或 base URL 隐式推导 native tool 默认值。
- 自定义 OpenAI-compatible / gateway / proxy provider 默认不会自动开启任何 native tool。
- TUI / Studio Web 在创建 provider 时只会为官方 auth 默认勾选一组显式 hosted tool presets；非官方 auth 默认不勾选，但仍可手动开启。
- 保存后这些选择会直接写进 `providers.<id>.adapters.<adapter>.models.<model>.native_tools.*`。
- `enabled = false` 也是显式配置的一部分，用来明确关闭某个 model 的这一层能力。

结构分四层：

- `enabled`：是否为这个 model 打开 native tool 系统。
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

创建界面的默认勾选规则：

- OpenAI first-party auth：默认勾选 `web_search`
- Anthropic first-party auth：默认勾选 `web_search`
- Gemini first-party auth：默认勾选 `web_search`、`url_context`、`code_execution`
- 其他 auth provenance：默认不勾选，但只要 adapter 支持，仍可手动开启对应 preset

这里的 “first-party auth” 指官方 auth provenance，而不是单纯的 adapter kind。例如同样走 `openai` adapter 的 Copilot、AtomGit、SAP AI Core、任意自建 gateway，都不会默认勾选 OpenAI hosted tools。

示例：

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai": {
          "models": {
            "gpt-5": {
              "native_tools": {
                "enabled": true,
                "routes": {
                  "web_search": "provider_hosted"
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
```

`hosted.*.provider_options` 是 escape hatch，用于写 provider-specific 原始 JSON；优先只用 canonical 字段。

当前 adapter/runtime 已经接通的 provider-hosted 组合是：

- OpenAI：`web_search`、`file_search`、`code_execution`
- Anthropic：`web_search`
- Gemini：`web_search`、`url_context`、`code_execution`

`image_generation`、`remote_mcp`、以及 provider-harness 路径已经有 canonical 配置模型，但当前对话 runtime 还没有把它们投影成一等消息输出或执行循环；如果为这些 route 写了显式配置，运行时会直接报不支持，而不是静默忽略。

### Harnesses

`provider_harness` 路由不把执行环境挂在 provider 下，而是引用顶层 `harnesses`。原因是 browser / shell / editor sandbox 属于 host 资产，不属于 provider 账号。

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
              "native_tools": {
                "enabled": true,
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
        "adapter": "openai",
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
        "openai": {
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
        "adapter": "openai",
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
        "openai": {
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
    "atomgit": {
      "defaults": {
        "adapter": "openai",
        "model": "Kimi-K2-Instruct"
      },
      "auth": {
        "mode": "credential",
        "issuer": "atomgit",
        "credential": {
          "type": "oauth",
          "issuer": "atomgit",
          "refresh": "...",
          "access": "...",
          "expires_at_ms": 4102444800000,
          "account_id": "atomgit-user"
        }
      },
      "adapters": {
        "openai": {
          "enabled": true,
          "models": {
            "Kimi-K2-Instruct": {
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
        "adapter": "openai",
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
        "openai": {
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

OpenCode Go / Zen 也是这类共享网关：Go 大多数模型走 OpenAI-compatible `/chat/completions`，MiniMax 模型走 Anthropic Messages `/messages`；Zen 还包含 OpenAI Responses 和 Gemini 路由。可复制配置见 [OpenCode 接入](opencode-go.md)。

### Model metadata 和 modes

canonical 路径是 `providers.<id>.adapters.<adapter>.models."<real-model-id>"`。

示例：

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai": {
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
                "light": {
                  "display_name": "Light",
                  "thinking": {
                    "type": "effort",
                    "effort": "low"
                  }
                },
                "deep": {
                  "display_name": "Deep",
                  "thinking": {
                    "type": "effort",
                    "effort": "high"
                  }
                }
              },
              "speed_modes": {
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

`thinking_modes.<name>` 字段包括 `display_name`、`description`、`thinking`、`disabled`。

`thinking` 写法：

```json
[
  {
    "thinking": {
      "type": "budget",
      "budget_tokens": 4096
    }
  },
  {
    "thinking": {
      "type": "effort",
      "effort": "medium"
    }
  },
  {
    "thinking": {
      "type": "disabled"
    }
  }
]
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

```json
{
  "providers": {
    "openai": {
      "adapters": {
        "openai": {
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
                    "openai": {
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

## Agents

Agent 只通过 `~/agena/agena.json` 中的 `agents` 配置。

JSON 示例：

```json
{
  "agents": {
    "plan": {
      "description": "Read-only planning agent",
      "prompt": "You are a planning agent...",
      "defaults": {
        "model": "anthropic/claude-sonnet-4-6"
      }
    }
  }
}
```

Markdown frontmatter 示例：

```markdown
---
description: "Read-only planning agent"
defaults:
  model: "anthropic/claude-sonnet-4-6"
permission:
  path:
    workspace:
      read: allow
      write: deny
  tools:
    names:
      shell: ask
---
You are a planning agent...
```

JSON agent 的 `prompt` 字段是 system prompt；Markdown agent 的正文是 system prompt。Markdown frontmatter 支持的字段和 JSON agent 基本一致，但不使用 `prompt` 和 `disabled`。

字段：

- `description`
- `prompt`
- `permission`
- `defaults`
- `disabled`

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

未显式配置 `permission` 时，Agena 的全局权限默认值是：允许读取当前 workspace，workspace 写入、外部路径读写、网络区域和未覆盖工具调用均为 `ask`。显式配置的字段会覆盖这些默认值，未配置的字段继续保留默认值。

Agent 也可以有自己的权限：

```json
{
  "agents": {
    "plan": {
      "permission": {
        "path": {
          "workspace": {
            "read": "allow",
            "write": "deny"
          },
          "external": {
            "read": "ask",
            "write": "ask"
          }
        },
        "tools": {
          "names": {
            "plan": "allow",
            "tools": "allow",
            "user": "allow",
            "agent": "allow",
            "session": "allow"
          }
        }
      }
    }
  }
}
```

Agent permission 会在顶层 permission 之上继续合并，不支持单独的 `inherit` 字段。

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
        "shell": "ask",
        "fs": "ask",
        "my-plugin.echo": "ask"
      },
      "rules": {
        "my-plugin.echo": {
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

`plugins.list."agena.memory".config` 会驱动 `agena.memory` 插件；模型可见 tool 名是 `memory`，支持 `search`、`get`、`list`、`write`、`delete` 五个 action。检索索引是工作区本地的 Tantivy 索引，不需要单独配置服务地址。`search` 和自动回忆会按需从 memory 文件重建索引，因此始终以磁盘上的 memory 文件为准。

## Workflow Tool Search

`agena.catalog/tools` 的 `search` action 现在使用进程内 Tantivy 索引。每次搜索都会基于当前已注册的 tool catalog 在本地重建索引，因此不依赖 Meilisearch 或其他外部服务。

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
        "echo": {
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
      "echo": {
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

`plugins.policy.tool_presentation` 控制模型请求里的 tool 说明是完整发送，还是只发送短说明并引导调用 `tools help`。

Tool presentation 支持全局、按 plugin、按 tool 覆盖。模式值：

- `detailed`: 使用 tool manifest / `tool.definition` hook 给出的完整 `description`。
- `help`: 只发送短说明和 help 引导，完整用法通过 `tools` tool 的 `help` action 读取。

```json
{
  "plugins": {
    "policy": {
      "tool_presentation": {
        "default_mode": "help",
        "plugins": {
          "agena.skills": "help",
          "agena.mcp": "help"
        },
        "tools": {
          "fs": "detailed",
          "agena.catalog/tools": "detailed"
        }
      }
    }
  }
}
```

按 tool 覆盖可以使用模型可见名（如 `fs`）、`plugin_id/tool_name`（如 `agena.catalog/tools`），或无冲突的原始 tool 名。具体 tool 覆盖优先于 plugin 覆盖；plugin 覆盖优先于 manifest 的 `description_mode`；最后才使用 `default_mode`。

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
      "agena.catalog": {
        "package": {
          "kind": "static"
        },
        "timeouts": {
          "init": "5s"
        }
      },
      "native": {
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
      "worker": {
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
      "policy": {
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
      "sandboxed": {
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
        "cloud-policy": {
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
              "url": "https://mcp.example.com",
              "headers": {},
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
headers
auth
```

HTTP transport 固定使用 streamable HTTP，不再支持 `mode` 字段，也不再支持 `ws` / `websocket` transport。`headers` 是普通 header map，`auth` 可以省略。

HTTP auth:

```json
[
  { "auth": { "kind": "bearer", "token": "..." } },
  { "auth": { "kind": "bearer_from_env", "env": "MCP_TOKEN" } },
  { "auth": { "kind": "bearer_from_store" } },
  { "auth": { "kind": "custom", "headers": { "X-Token": "..." } } }
]
```

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
              "command": "rust-analyzer",
              "args": [],
              "env": {},
              "file_extensions": [
                "rs"
              ],
              "root_markers": [
                "Cargo.toml"
              ],
              "initialization_options": {}
            }
          }
        }
      }
    }
  }
}
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

`file_extensions` 不带前导 `.`；写空数组表示该 server 匹配所有文件。`root_markers` 是用于识别项目根目录的文件名列表。`initialization_options` 是传给 language server 的 JSON object。

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

如果要启用 OpenAI / Anthropic / Gemini 这类 provider-native remote tools，不要写在 `agena.web` plugin config，而是写在 `providers.<id>.adapters.<adapter>.models.<model>.native_tools.*`。

## Studio 服务配置

Studio server 是 `agena-studio` 二进制，参数定义在 `apps/agena-studio-server/src/main.rs`。

常用启动：

```bash
agena-studio \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace-root "$PWD"
```

服务参数：

```text
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
