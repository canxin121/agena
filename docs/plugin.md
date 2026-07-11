# Plugin 体系

Agena 的扩展能力统一通过 plugin host 接入。模型可见的 tools、MCP server 暴露出来的能力、LSP 查询、skills、plan、memory、cron 等，都会在 runtime 中表达为 plugin 或 plugin tool，然后进入同一个 tool registry。

这意味着使用者和开发者应该把 plugin 当成主要抽象：

- 要新增一个模型可调用能力，写 plugin tool。
- 要接入 MCP server，配置 MCP server，然后由 `agena.mcp` static plugin 暴露 tools。
- 要扩展 prompt、provider、权限、shell env、事件等运行时行为，订阅 plugin hook。
- 要扩展 TUI 或 Studio 前端界面，在 manifest 的 `ui.tui` / `ui.studio` 中声明 UI contributions。
- 要给 plugin 持久化状态或 secret，使用 plugin host callback。
- 要观察运行状态、日志、marketplace 安装状态，走 plugin CLI/API。

## 总览

Runtime build 会构建一个 `PluginHost`。这个 host 加载 runtime 注册的 static plugin，以及用户在 `plugins.list.<id>` 中声明的 plugin。

```text
agena.json
  |
  +-- plugins ---------------------+
  |                                |
  +-- plugins.list."agena.*"       |
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

## Tool Registry 观测

Plugin tool registry 现在有三条统一观测路径，分别面向快照、增量和实时事件：

- `GET /api/v1/runtime` 与 `GET /api/v1/plugins/ui` 返回当前 UI catalog 快照，同时带 `tool_registry_generation` 和 `tool_registry_last_event`。
- `GET /api/v1/plugins/tools/changes?after_generation=<n>` 返回按 generation 递增的 registry change 列表，适合轮询式前端或 catalog cache。
- `GET /api/v1/events/stream?kinds=plugin_tool_registry_changed` 在统一事件总线上实时推送 registry 变化，适合 Studio/TUI 或外部调试器做即时刷新。

`plugin_tool_registry_changed` 的 payload 和 `/api/v1/plugins/tools/changes` 返回的单条记录一致，字段包括：

- `kind`: `registered`、`updated`、`removed`
- `generation`: 当前 registry generation
- `plugin_id`
- `plugin_tool_name`
- `model_name`
- `tool`: 注册或更新时的 `ToolDefinition`；删除时通常为空

如果一个客户端同时消费 plugin UI 和 runtime 事件，推荐策略是：

1. 启动时先读 `/api/v1/runtime` 或 `/api/v1/plugins/ui` 建立完整快照。
2. 记住 `tool_registry_generation`。
3. 运行中优先订阅 `/api/v1/events/stream?kinds=plugin_tool_registry_changed` 做实时刷新。
4. 检测到断流、lagged 或重连后，用 `/api/v1/plugins/tools/changes?after_generation=<last_generation>` 做精确补齐。

## Plugin 和 Tool

一个 plugin 是一组扩展能力。它可以只提供 hooks，也可以提供一个或多个 tools。

一个 tool 是模型可调用的能力单元。Tool 使用统一的 `ToolDefinition`，顶层只保留少量入口字段：

- `name`: plugin 内部 tool 名称。
- `contract`: 调用契约，包含 `input_schema`、`output_schema`、`strict`。
- `model`: 给模型看的内容，只保留 `examples`。
- `docs`: 给 help/catalog/UI 使用的文档，包含 `summary`、`help`、`before_help`、`after_help`。
- `display`: 展示策略，包含 `description_mode`、`ui_display_mode`。运行时配置可以覆盖它。
- `permissions`: 权限声明，包含 `input_paths`、`input_networks`、`path_access`、`network_access`、`tags`。
- `runtime`: 运行策略，包含 `concurrency_safe`、`streaming`、`result_policy`。
- `capabilities`: 这个 tool 调用 host callback 时需要的能力声明。

Rust SDK / 宏定义只保留 `summary` 作为 tool 的短描述；详细说明统一进入 `help`。

Tool registry 使用稳定的 canonical tool id：

- canonical tool id 始终是 `namespace.plugin.tool`，例如 `agena.web.fetch`。
- plugin id 是 `namespace.plugin`，来自 manifest 的 `namespace` 和 `name`；`plugins.list` 的 key、plugin quota 和 plugin presentation 配置都使用这个 id。
- `tool` 是 manifest `tools[*].name`。`namespace` 和 `plugin` 不能包含 `.`；`tool` 可以包含 `.`，且第二个 `.` 之后的全部内容都属于 tool 名称。

模型只收到 gateway function tools：`tools_list`、`tools_search`、`tools_help`、`tools_tags` 和 `tools_call`。要传给 `tools_help` 或 `tools_call` 的目标名称，以 `tools_list` 返回的 `name` 为准：通常是紧凑的 `plugin.tool`（例如 `fs.read`）；若有重名冲突，列表会返回完整 canonical id（例如 `example.notes.format`）。

## Tool 说明模式和 Help

Tool 的模型可见说明默认只给短 `summary`，并提示模型在需要时调用 `tools_help` 获取完整帮助。

详细帮助不随 provider 请求一起发送，避免把大量 tool、MCP server 或 skill 的长说明塞进每次模型调用。需要完整用法时，模型可以调用 `tools_help`：

```json
{"tool": "fs.read", "include_schema": true}
```

其中 `tool` 是 gateway 目录返回的目标名称；如果有重名冲突，使用目录返回的完整 canonical id。`include_schema` 默认为 `true`，会把注册后的 input schema 一并返回。

配置优先级从高到低：

1. `plugins.policy.tool_presentation.tools` 中的具体 tool 覆盖。
2. `plugins.policy.tool_presentation.plugins` 中的 plugin 覆盖。
3. manifest tool definition 里的 `display.description_mode`。
4. `plugins.policy.tool_presentation.default_mode`。

示例：

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
          "agena.fs.read": "detailed",
          "agena.tools.help": "detailed"
        }
      }
    }
  }
}
```

`tools_search` 用于发现已注册 tools；`tools_help` 用于拿到任意已注册 tool 的详细说明。Plugin 作者可以在 manifest 中设置 `docs.summary` 和 `docs.help`，也可以通过 `tool.definition` hook 改写 `docs.summary`、`docs.help`、`display.description_mode` 和 `contract.input_schema`。

## Provided Static Plugins

Runtime 会注册一组 runtime-provided static plugins。这些 plugin 和用户配置 plugin 使用同一个 host、manifest、tool registry、hook dispatch 和 permission 路径。

Runtime build 注册：

| Plugin id | 作用 |
| --- | --- |
| `agena.fs` | 文件系统读写相关 tools |
| `agena.process` | 前台命令执行和后台进程管理 tools |
| `agena.web` | 本地 Spider/CRW web search/fetch/crawl、嵌入式 crawl cache、去重、进程内 cache 和可选 Agena 托管浏览器渲染 tools |
| `agena.code` | `ast-grep` / `tree-sitter` 驱动的多语言结构化代码搜索和语法树检查 tools |
| `agena.tools` | tool discovery/help/gateway tools |
| `agena.runtime` | agent、session 和 user input 等 runtime control tools |
| `agena.plan` | plan 和 plan autorun 等 planning tools |
| `agena.tasks` | delegated subtask orchestration tools |
| `agena.snapshot` | repository worktree management tools |
| `agena.skills` | 扫描 `SKILL.md`、slash command，以及内置 `init/review/security_review` skills，并动态注册 tools |
| `agena.lsp` | LSP server 观测和 LSP 查询 tools |
| `agena.cron` | cron 和 one-shot wakeup 调度 tools |
| `agena.memory` | memory 配置和项目记忆相关能力 |
| `agena.mcp` | 已配置 MCP server 的 tool/resource/prompt tools |
| `agena.settings` | 读取、列出、校验和编辑当前 `agena.json` 的 settings tools |

内置 static plugin 会把动作拆成独立的 registry tool。它们是通过 gateway 调用的目标；常见 action 如下：

| Plugin | Tools |
| --- | --- |
| `agena.fs` | `read`, `glob`, `grep`, `apply_patch` |
| `agena.process` | `run`, `list`, `logs`, `stop` |
| `agena.web` | `fetch`, `search`, `crawl` |
| `agena.code` | `search_ast`, `syntax_tree` |
| `agena.lsp` | `servers`, `definition`, `references`, `hover`, `diagnostics` |
| `agena.memory` | `search`, `get`, `list`, `write`, `delete` |
| `agena.settings` | `get`, `list`, `validate`, `set`, `delete`, `patch` |
| `agena.cron` | `list`, `create`, `delete`, `wakeup` |
| `agena.tools` | `list`, `search`, `tags`, `help`, `call` |
| `agena.runtime` | `switch`, `restore`, `get`, `rename`, `request_input` |
| `agena.plan` | `get`, `set`, `update`, `clear` |
| `agena.tasks` | `run` |
| `agena.snapshot` | `enter`, `exit` |

`agena.mcp` 读取 MCP server snapshot，但不再把每个 MCP capability 展开成一个单独的 gateway function：

| Tool | 作用 |
| --- | --- |
| `mcp.resources.list`, `mcp.resources.read`, `mcp.prompts.list`, `mcp.prompts.get`, `mcp.tools.call` | resource/prompt 读取和 MCP tool 调用 |

因此，MCP 对模型的可见面统一进入 plugin host 和 plugin tool registry，同时不会随 server/tool 数量线性膨胀。MCP 的网络权限按调用里的 `server` 动态审计。

`agena.settings` 使用当前 runtime 的 active config path。对 effective 读操作，推荐显式传 `scope = config|meta` 和相对 `path`，避免依赖 `config.` / `meta.` 前缀魔法。`settings` 里的写入类 action 默认先校验再写入，并在有实际变更时通过 `host/config.reload` reload runtime；`dry_run=true` 会返回差异但不落盘、不 reload。

## Transport

`plugins.list.<id>.package.kind` 选择 plugin transport。

| kind | 场景 | 关键字段 |
| --- | --- | --- |
| `static` | runtime 内注册的 in-process plugin | `package`, `config`, `timeouts` |
| `cdylib` | 本地动态库 plugin | `package.path`, `config`, `timeouts`, `sha256`, `signature` |
| `stdio` | 子进程 JSON-RPC plugin | `package.command`, `package.args`, `package.env`, `package.cwd`, `package.restart`, `config`, `timeouts`, `sha256` |
| `http` | 远端 JSON-RPC plugin | `package.url`, `package.auth`, `config`, `timeouts` |
| `wasm` | WebAssembly plugin | `package.path`, `config`, `timeouts`, `sha256` |

选择建议：

- 需要最低开销并且随 runtime 一起发布：用 `static`。
- Rust 本地 plugin、追求低延迟：用 `cdylib`。
- Node/Python/Go/任意语言子进程：用 `stdio`。
- 组织内已有服务或远端策略服务：用 `http`。
- 需要更强隔离和可移植 artifact：用 `wasm`。

## 配置

最小配置：

```json
{
  "plugins": {
    "list": {
      "example.echo": {
        "package": {
          "kind": "stdio",
          "command": "node",
          "args": [
            "./plugins/echo/index.js"
          ]
        },
        "config": {
          "uppercase": true
        }
      }
    }
  }
}
```

完整一些的 stdio 配置：

```json
{
  "plugins": {
    "host": {
      "timeouts": {
        "init": "10s",
        "tool_invoke": "60s",
        "permission_ask": "10s"
      }
    },
    "list": {
      "example.lint": {
        "package": {
          "kind": "stdio",
          "command": "node",
          "args": [
            "./plugins/lint/index.js"
          ],
          "env": {
            "LOG_LEVEL": "info"
          },
          "cwd": ".",
          "restart": {
            "policy": "on-failure",
            "min_backoff": "1s",
            "max_backoff": "30s",
            "max_retries": 5
          }
        },
        "config": {
          "project": "rust"
        },
        "timeouts": {
          "tool_invoke": "30s"
        }
      }
    }
  }
}
```

HTTP plugin：

```json
{
  "plugins": {
    "list": {
      "example.cloud-policy": {
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
        }
      }
    }
  }
}
```

cdylib plugin：

```json
{
  "plugins": {
    "list": {
      "example.echo": {
        "package": {
          "kind": "cdylib",
          "path": "examples/echo_plugin/target/debug/libagena_echo_plugin.so"
        },
        "config": {
          "uppercase": true
        }
      }
    }
  }
}
```

Wasm plugin：

```json
{
  "plugins": {
    "list": {
      "example.sandboxed": {
        "package": {
          "kind": "wasm",
          "path": "./plugins/sandboxed/plugin.wasm",
          "sha256": "..."
        }
      }
    }
  }
}
```

Static plugin config override：

```json
{
  "plugins": {
    "list": {
      "agena.web": {
        "package": {
          "kind": "static"
        },
        "timeouts": {
          "tool_invoke": "45s"
        }
      }
    }
  }
}
```

`PluginHostBuilder::register_static` 只注册可用的 in-process factory；是否加载由 active config 中的 `plugins.list.<id>` 决定。内建能力没有特殊配置通道，实际配置写在对应 static plugin config 的 `config` 中。

## `plugins` 字段

顶层 `plugins` 支持：

| 字段 | 含义 |
| --- | --- |
| `host` | plugin host 生命周期、timeout、quota、trusted key 配置。 |
| `policy` | plugin/tool 展示策略。 |
| `list` | plugin 声明表，key 是 plugin id（`namespace.plugin`，例如 `example.echo`）。 |

`plugins.host` 支持：

| 字段 | 含义 |
| --- | --- |
| `timeouts` | 全局 plugin timeout overlay。 |
| `default_quota` | 没有单独 quota 的 plugin 使用的默认 host callback quota。 |
| `quotas` | 按 plugin id 设置 host callback quota。 |
| `trusted_keys` | cdylib signature 校验使用的 ed25519 public key。 |

`plugins.policy` 支持：

| 字段 | 含义 |
| --- | --- |
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

```json
{
  "plugins": {
    "host": {
      "timeouts": {
        "init": "10s",
        "tool_invoke": "2m",
        "fast": "500ms"
      }
    }
  }
}
```

Quota 用于限制 plugin 调用 host callback 的频率和并发数。默认不限制。

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

Supply-chain 校验可以使用 artifact hash 和 trusted key：

```json
{
  "plugins": {
    "host": {
      "trusted_keys": {
        "acme": "0123456789abcdef..."
      }
    },
    "list": {
      "example.secure-plugin": {
        "package": {
          "kind": "cdylib",
          "path": "./plugins/secure/libsecure.so",
          "sha256": "...",
          "signature": {
            "key_id": "acme",
            "signature": "..."
          }
        }
      }
    }
  }
}
```

## Stdio Restart

`stdio` plugin 支持 restart policy：

```json
{
  "plugins": {
    "list": {
      "example.worker": {
        "package": {
          "kind": "stdio",
          "command": "./plugins/worker",
          "restart": {
            "policy": "always",
            "min_backoff": "1s",
            "max_backoff": "30s",
            "max_retries": 5
          }
        }
      }
    }
  }
}
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

```json
{
  "plugins": {
    "list": {
      "example.policy": {
        "package": {
          "kind": "http",
          "url": "https://policy.example.com/agena/rpc",
          "auth": {
            "kind": "none"
          }
        }
      }
    }
  }
}
```

Bearer token：

```json
{
  "auth": {
    "kind": "bearer",
    "token_env": "PLUGIN_TOKEN"
  }
}
```

Basic auth：

```json
{
  "auth": {
    "kind": "basic",
    "username": "user",
    "password_env": "PLUGIN_PASSWORD"
  }
}
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
- `tools`
- `commands`
- `plugin_capabilities`
- `ui`
- `config_schema`
- `config_schema_i18n`

Rust SDK 中的最小 plugin 推荐使用 `#[agena_plugin]`：

```rust
use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct EchoPlugin;

#[agena_plugin(
    namespace = "example",
    name = "echo",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Echo text.",
    export = stdio
)]
impl EchoPlugin {
    #[tool(name = "echo", summary = "Echo the supplied text.", read_only, concurrency_safe)]
    async fn echo(&self, #[arg(trim, non_empty)] text: String) -> String {
        text
    }
}
```

如果不使用 `export = ...`，也可以手动导出为 cdylib：

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
- `examples/multi_tool_plugin_stdio`: 推荐的多 tool stdio plugin 写法，覆盖方法级 `#[tool]`、`stream = ...`、字段级权限和 config。

## 多 Tool Plugin 推荐写法

推荐入口是 `#[agena_plugin(...)]`。一个 plugin 暴露多个模型可见 tool 时，把每个 tool 写成 impl 里的一个方法即可；宏会生成隐藏 input/surface 类型、manifest tool definition、静态分发、stream fallback 和 permission 分发。非泛型插件的 manifest/schema 会在生成代码中按类型缓存，避免每次查询 catalog 时重复构造 schema。

如果 tool 输入比较复杂，继续使用独立 input struct。推荐给输入类型派生 `ToolInput`，把字段级清洗和校验写在字段旁边；`#[tool]` 只保留 tool 的语义、权限和运行时策略：

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct FormatInput {
    #[arg(trim, non_empty)]
    text: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct WriteInput {
    #[arg(trim, non_empty, path.write)]
    path: String,
    #[arg(trim, non_empty)]
    text: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FormatOutput {
    rendered: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteOutput {
    message: String,
    path: String,
    bytes: usize,
}

#[agena_plugin(namespace = "example", name = "notes", version = env!("CARGO_PKG_VERSION"), summary = "Notes plugin.")]
impl NotesPlugin {
    #[tool(
        name = "format",
        summary = "Format text.",
        output(FormatOutput),
        read_only,
        stream = format_stream,
        concurrency_safe
    )]
    async fn format(&self, input: &FormatInput) -> Result<FormatOutput> {
        Ok(FormatOutput {
            rendered: input.text.trim().to_string(),
        })
    }

    async fn format_stream(&self, sink: ToolStreamSink, input: &FormatInput) -> Result<ToolStreamEnd> {
        // ...
    }

    #[tool(
        name = "write",
        summary = "Write text.",
        output(WriteOutput),
        mutating,
        filesystem_write
    )]
    async fn write(&self, input: &WriteInput) -> Result<WriteOutput> {
        Ok(WriteOutput {
            message: format!("wrote {}", input.path),
            path: input.path.clone(),
            bytes: input.text.len(),
        })
    }
}
```

方法只有一个输入 struct 参数时，`#[agena_plugin]` 会通过该类型的 `ToolInput` 解析输入；因此该类型应派生 `ToolInput`。schema、trim、non-empty、items/chars 约束和校验逻辑都来自输入类型本身，不需要在 `#[tool]` 上重复声明。方法声明 `output(OutputType)` 后，manifest 会包含该输出类型的 JSON schema，handler 可以直接返回 `OutputType` 或 `Result<OutputType, E>`；泛型输出类型同样使用 `output(Vec<OutputItem>)`。

简单 tool 可以省掉 input struct，由方法参数直接生成隐藏输入类型：

```rust
#[agena_plugin(namespace = "example", name = "echo", version = env!("CARGO_PKG_VERSION"), summary = "Echo plugin.")]
impl EchoPlugin {
    #[tool(name = "echo", summary = "Echo text.", read_only, concurrency_safe)]
    async fn echo(&self, #[arg(trim, non_empty)] text: String) -> String {
        format!("echo: {text}")
    }
}
```

inline `#[arg(...)]` 不只负责 trim/校验和权限语义，也可以直接定义 wire 名和兼容别名。例如 `#[arg(name = "filePath", alias = "path", path.read)] file_path: String` 会让 schema 主字段名变成 `filePath`，同时继续接受旧的 `path` 输入，并自动把 permission jsonpath 扩展到 `$.filePath` 和 `$.path`。Studio 的 `key=value` shorthand 也会把 `path=...` 这类 alias 自动映射回主字段。

对 `ToolInput` struct field，也可以直接写 `#[arg(name = "filePath", alias = "path")]`，不必再分别配 `#[serde(rename = ...)]` / `#[serde(alias = ...)]`。derive 宏会把 `filePath` 作为 schema 主字段名，同时继续接受旧的序列化字段名 `file_path` 和显式声明的 alias，并在反序列化前先把它们统一归一化回真正的 struct field。

如果参数需要默认值，也可以直接写 `#[arg(default = 3)]`、`#[arg(default = String::from("guest"))]` 或 `#[arg(default)]`。inline 参数会把默认值写进隐藏 input struct；`ToolInput` struct field 也支持同样写法，宏会在反序列化前先把缺失字段补上，并同步写入 schema `default`，所以自动推导出来的 command usage 也会复用这个默认值。field 级 `#[arg(default)]` 不要和同一字段上的 `#[serde(default)]` 混用。

如果 typed input 需要一个完整的权威示例，也可以直接在类型上写 `#[input(example = serde_json::json!({...}))]`。这个例子会同时成为 `ToolInput::input_example()` 的返回值，并写入根 schema `examples`，适合数组、嵌套 object 或跨字段一起才有意义的示例。

如果整个 typed input 在缺省时就有一个完整默认值，也可以直接写 `#[input(default)]` 或 `#[input(default = Self { ... })]`。当 host 传进来的 input 是 `null` 时，宏会先把它替换成这个默认值，并同步写入根 schema `default`。它不会自动和一个“部分 object”做 merge；部分字段补默认值仍然建议写 field 级 `#[arg(default = ...)]`。

如果某个普通 struct field 本身也是一个派生了 `ToolInput` 的嵌套输入形状，可以在字段上写 `#[input(nested_shape)]`。这样父级 `ToolInput` 会在外层解析前先对这个嵌套 object 做 inner field alias/default 归一化，解析后再把该字段重新走一遍 inner `ToolInput::parse_input()`，因此内层的 `#[arg(name/alias/default/trim/...)]`、type-level `#[input(...)]` 校验、schema metadata、声明式 `path.*` / `network.*` 权限和 tags 都会保留下来；如果外层字段自己还有 alias，这些 prefix 也会自动扩展到嵌套 jsonpath，比如 `payload.filePath` 和 `body.filePath`。外层 type-level 路径约束也会继续向内解析，所以像 `#[input(trim("body.filePath"), non_empty("payload.filePath"))]` 这类写法会自动对齐到内层真正的 parse key。对 `Vec<Inner>` 或 `Option<Vec<Inner>>` 这类数组字段，item 级路径同样可以直接写成 `payload.filePath` / `body.filePath`，宏会自动补成内部使用的 `payload[].filePath` 再落到每个 item 上。内层 parse/校验一旦报错，错误路径也会自动带上外层前缀；如果字段是数组，还会保留具体 item 下标，比如 `payload[1].filePath`。

如果不想为了一个嵌套 object 单独再包一层 typed input，tool / command 的 inline 参数现在也可以直接写 `#[arg(nested_shape)]`。它会把这个参数当成隐藏 input struct 里的一个嵌套 `ToolInput` 字段处理，所以内层的 `name/alias/default/trim/...`、schema metadata、声明式 `path.*` / `network.*` 权限和 tags 也都会保留下来；如果外层 inline 参数自己还有 `name` / `alias`，这些 prefix 也会继续扩展到内层字段。

如果想把一个现成的 `ToolInput` 片段直接铺平到 inline 参数生成的隐藏 input 里，也可以写 `#[arg(flatten_shape)]`。它等价于在隐藏 struct 上写一个 `#[serde(flatten)] #[input(flatten_shape)]` 字段，所以内层字段会直接提升到当前输入根上，继续复用内层 `ToolInput` 的 alias/default/schema metadata、声明式权限和 tags。这个写法只适合“纯铺平”的场景；额外的 rename/alias/default/trim/permission 配置应继续写在被 flatten 的那个 `ToolInput` 类型上。

如果需要给 inline 参数补 schema 描述，可以直接写 `#[arg(description = "Path to inspect.")]`。同一个属性在 `ToolInput` struct field 上也可以使用，用来覆盖默认来自 doc comment 的 description。

`#[command(...)]` 现在也支持同样的两种输入写法：可以继续接收一个派生了 `ToolInput` 的结构体，也可以像 tool 一样直接在方法参数上写 `#[arg(...)]`，让宏生成隐藏 command input schema 和解析逻辑：

```rust
#[agena_plugin(namespace = "example", name = "notes", version = env!("CARGO_PKG_VERSION"), summary = "Notes plugin.")]
impl NotesPlugin {
    #[command("/note-greet", id = "notes.greet", title = "Greet")]
    fn greet(&self, #[arg(trim, non_empty, example = "Ada")] name: String) -> String {
        format!("hello {name}")
    }
}
```

当 `#[command]` 只有一个普通参数且没有 `#[arg]` 时，宏仍然按“显式 input shape”语义处理。现在常见的 top-level JSON 类型已经直接实现了 `ToolInput`，所以 `fn greet(&self, name: String)`、`fn toggle(&self, enabled: bool)`、`fn ids(&self, ids: Vec<String>)` 这类单参数 command/tool 都可以直接工作；只有在你需要字段级 trim/alias/default/permission/schema metadata 时，才需要给参数加 `#[arg(...)]` 或改成显式 struct。

`#[arg(example = ...)]` 接受任何 `serde_json::json!(...)` 能表示的字面量，所以除了字符串，也可以直接写 `example = 3`、`example = true`、`example = ["a", "b"]` 这类值；它们会同时进入 schema `examples` 和自动推导的 command usage。

如果参数只接受固定枚举值，可以直接写 `#[arg(choices = ["cargo", "git"])]`；对派生 `ToolInput` 的结构体，也可以在类型上写 `#[input(choices("mode", "fast", "slow"))]`。这两种写法都会在运行时校验输入值，并把 schema 对应字段补成 JSON Schema `enum`，所以自动推导的 usage 会优先选第一个枚举值，例如 `/cmd cargo`。

如果字符串参数需要已有语义格式校验，可以直接写 `#[arg(format = "uri")]`、`#[arg(format = "uuid")]` 这类约束；对派生 `ToolInput` 的结构体，也可以写 `#[input(format("endpoint", "uri"))]`。当前内建支持 `uri`、`uuid`、`email`、`hostname`、`ipv4`、`ipv6`，宏会在运行时校验字符串格式，并把同一个值写入 schema `format`，所以像 `#[arg(name = "endpoint", format = "uri")]` 这样的 renamed field 也会在最终 wire key 上得到正确的 schema metadata。自动推导 usage 时，这些格式还会优先生成可执行示例值，例如 `uri` 会得到 `https://example.com`，`uuid` 会得到一个固定 UUID。

如果字符串参数需要正则约束，可以直接写 `#[arg(pattern = "^[a-z0-9-]+$")]`；对派生 `ToolInput` 的结构体，也可以写 `#[input(pattern("slug", "^[a-z0-9-]+$"))]`。宏会在运行时用 Rust `regex` 校验匹配结果，并把同一个模式写入 schema `pattern`，所以像 `#[arg(name = "slug", pattern = ...)]` 这类 renamed field 也会在最终 wire key 上得到正确的 schema metadata。

如果字符串参数还有长度要求，可以直接写 `#[arg(min_chars = 3, max_chars = 32)]`；对派生 `ToolInput` 的结构体，也可以写 `#[input(min_chars("slug", 3), max_chars("slug", 32))]`。这些约束会在运行时按字符数校验字符串，并把 schema 上的 `minLength` / `maxLength` 同步补齐；如果同一个字段同时声明了 `non_empty` 和 `min_chars`，宏会保留更严格的 `minLength`。

如果参数本身是 `Vec<String>` 这类字符串数组，也可以直接给数组项写约束：inline 参数支持 `#[arg(item_min_chars = 3, item_max_chars = 32, item_pattern = "^[a-z0-9-]+$", item_choices = ["cargo", "git"])]`，派生 `ToolInput` 既支持 field 级同名 `#[arg(...)]`，也支持类型级 `#[input(item_min_chars("tags", 3), item_pattern("tags", "^[a-z0-9-]+$"), item_choices("tags", "cargo", "git"))]`。这些 sugar 会自动落到 schema `items.minLength` / `items.maxLength` / `items.pattern` / `items.enum`，等价于手写 `tags[]` 路径约束，但作者不需要自己拼 `[]` 路径。

同样地，字符串数组项也可以直接写 `item_format`：例如 `#[arg(item_format = "uuid")]` 或 `#[input(item_format("ids", "uuid"))]` 会把校验和 schema metadata 落到 `items.format`。这和手写 `ids[]` 路径上的 `format` 约束等价，但错误信息和 schema 都会直接指向数组项。

如果字符串数组项需要就地规范化，也可以直接写 `item_trim` 和 `item_trim_suffix`：例如 `#[arg(item_trim, item_trim_suffix = ".rs")]` 或 `#[input(item_trim("tags"), item_trim_suffix("tags", ".rs"))]` 会把 trim / suffix 去除直接作用在 `tags[]` 上，不需要再手写自定义 normalizer。它们和已有的 `item_min_chars`、`item_pattern`、`item_format` 可以组合使用，所以像 `" cargo.rs "` 这样的输入会先被规范化，再进入后续校验。

如果要求数组项本身非空，也可以直接写 `item_non_empty` 或 `item_non_empty_if_present`：例如 `#[arg(item_non_empty)]`、`#[arg(item_non_empty_if_present)]`，或者 `#[input(item_non_empty("tags"))]`、`#[input(item_non_empty_if_present("tags"))]`。这两种 sugar 会直接校验 `tags[]`，并把 schema `items.minLength` 自动补成 `1`；前者要求匹配到的数组项都非空，后者则只在该路径实际出现值时才拒绝空字符串项。

数组项如果是数字或 object，也可以用同样的写法：`#[arg(item_minimum = 0, item_maximum = 10)]` / `#[input(item_minimum("counts", 0), item_maximum("counts", 10))]` 会把约束落到 `items.minimum` / `items.maximum`；`#[arg(item_min_properties = 1, item_max_properties = 4)]` / `#[input(item_min_properties("entries", 1), item_max_properties("entries", 4))]` 会把约束落到 `items.minProperties` / `items.maxProperties`。这些 sugar 同样等价于手写 `counts[]`、`entries[]` 这类路径约束，但 schema 和运行时错误都会直接指向数组项。

如果字段本身就是一个直接的 `Vec<T>`，而约束语义明显只适用于数组项，宏现在也会自动把不带 `item_` 前缀的写法落到 `[]` 上。例如 `#[arg(trim, trim_suffix = ".rs", min_chars = 3, pattern = "...")] tags: Vec<String>`、`#[arg(choices = ["cargo", "git"])] tools: Vec<String>`、`#[arg(minimum = 1, maximum = 5)] counts: Vec<u32>`、`#[arg(min_properties = 1)] entries: Vec<BTreeMap<_, _>>` 都会自动约束数组项；type-level 的 `#[input(trim("tags"), pattern("tags", "..."))]`、`#[input(choices("tools", "cargo", "git"))]`、`#[input(minimum("counts", 1))]` 也一样。这个自动补 `[]` 只针对那些对整个 array 本身没有直接语义的约束；像 `non_empty`、`non_empty_if_present`、`min_items`、`max_items` 仍然保留“数组本身”的语义，所以元素级非空仍然应该继续写 `item_non_empty` / `item_non_empty_if_present`。

对 `#[serde(tag = "action")]` 这类 enum `ToolInput`，同样可以把这些规则直接写在 variant 的 `#[input(...)]` 上。例如 variant 级 `trim("query")`、`item_trim("tags")`、`item_trim_suffix("tags", ".rs")`、`item_non_empty("tags")`、`item_non_empty_if_present("tags")` 都会只作用于对应 variant 的局部输入形状；如果 variant 字段本身就是直接的 `Vec<T>`，那么 `trim("tags")`、`trim_suffix("tags", ".rs")`、`min_chars("tags", 3)`、`pattern("tags", "...")`、`choices("tools", ...)`、`forbid_substrings("tags", "..")`、`distinct_trimmed("tags")` 这类写法也会像 struct 一样自动落到数组项上。variant 字段本身也支持 struct 同款的 field 级 `#[arg(...)]`，包括 `name`、`alias`、`default`、`trim`、`non_empty`、`distinct_trimmed` 等；这些配置会参与 enum 解析前的 alias 归一化和默认值补齐，并同步写入对应 branch 的 schema metadata。variant 路径还会继续解析 serde 字段名和 field 级 `#[arg(name = ...)]`，所以像 `#[serde(rename_all_fields = "camelCase")]` 下面的 `trim("file_path")`、`requires("file_path", "mode")`、`infer_when_present("file_path")`、`drop_keys("file_path")`，或者 field 上写了 `#[arg(name = "filePath", alias = "path")]` 时的同名路径，都会自动对齐到最终 wire key；`infer_when_present` / `drop_keys` 现在也支持普通 JSON path，比如 `selector.kind`、`items[].kind` 这类嵌套路径，且顶层字段如果用了 `#[arg(alias = ...)]` 或 `#[serde(alias = ...)]`，这些 alias 也会自动扩展到对应的嵌套 path。若 variant 内部有 `#[serde(flatten)] #[input(flatten_shape)]` 的嵌套 `ToolInput` 字段，或者普通字段上的 `#[input(nested_shape)]` 嵌套 `ToolInput` 字段，它的 schema metadata、field-level `#[arg(name/alias/default)]` 归一化结果、声明式 `path.*` / `network.*` 权限和 tags 都会一起提升到该 branch；前者还会继续参与 `infer_when_present` / `drop_keys` 的候选输入键和外层 type-level `trim/non_empty/requires/...` 路径解析，后者则保留自己的对象层级，但它的 inner field `name` / `alias` 仍然会扩展到像 `payload.filePath`、`body.path` 这类候选输入键，而且外层 type-level `trim/non_empty/requires/...` 这类路径约束也会继续向内解析。若这个 `nested_shape` 字段本身还是数组，那么 `infer_when_present("payload.file_path")`、`drop_keys("body.filePath")` 这类省略 `[]` 的写法也会自动落到每个 item。内层 parse/校验错误也会自动回写成 branch 上的完整外层路径；数组 nested shape 会继续保留 item 下标。由于这些权限同样只在对应 branch 生效，宏会把提升到 enum 根上的 jsonpath spec 标记成 optional，避免未选中的 variant 在权限提取阶段被误判为缺字段。宏会先做该 variant 的规范化，再执行后续校验和 schema metadata 补充，因此不需要再把这类规则提升到整个 enum 根上统一处理。

如果 object / map 参数需要限制键值对数量，可以直接写 `#[arg(min_properties = 1, max_properties = 4)]`；对派生 `ToolInput` 的结构体，也可以写 `#[input(min_properties("labels", 1), max_properties("labels", 4))]`。这些约束会在运行时按 object property 数量校验，并把 schema 上的 `minProperties` / `maxProperties` 同步补齐；如果字段同时声明了 `non_empty` 和 `min_properties`，宏会保留更严格的 `minProperties`。

如果数值参数需要上下界，可以直接写 `#[arg(minimum = 0, maximum = 10)]`；对派生 `ToolInput` 的结构体，也可以写 `#[input(minimum("count", 1), maximum("count", 5))]`。这些约束会在运行时校验 numeric JSON value，并把 schema 上的 `minimum` / `maximum` 同步补齐；自动推导 usage 时也会优先选 `minimum` 作为示例值，所以常见单字段输入会直接得到 `/cmd 1` 这类更可执行的默认示例。

如果边界需要严格不等式，也可以直接写 `#[arg(exclusive_minimum = 0, exclusive_maximum = 10)]`；对派生 `ToolInput` 的结构体，则可以写 `#[input(exclusive_minimum("count", 0), exclusive_maximum("count", 10))]`。它们分别落到 schema `exclusiveMinimum` / `exclusiveMaximum`，运行时报错会明确区分 “greater than” 和 “less than”；数组项同理支持 `item_exclusive_minimum` / `item_exclusive_maximum`，等价于手写 `counts[]` 路径上的严格数值约束。

如果约束依赖另一个字段，field 级 `#[arg(...)]` 现在也支持直接写关系规则，不必再在类型上重复当前字段路径：例如 `#[arg(requires = "mode")]`、`#[arg(conflicts_with = "stdin")]`、`#[arg(required_unless_present = "text")]`。对派生 `ToolInput`，这些写法等价于 `#[input(requires("path", "mode"))]` 这类 type-level 规则，但当前字段路径会自动补上；当字段用了 `#[arg(name = ...)]` 或 alias 时，schema 里的关系说明和运行时错误也会继续显示最终 wire key。

如果约束是“当前字段加上一组 peer”的组合关系，也可以直接写 field 级 group sugar：`#[arg(exactly_one_of = ["stdin"])]` 表示“当前字段和 `stdin` 恰好填一个”，`#[arg(at_least_one_of = ["stdin", "text"])]` 表示“当前字段、`stdin`、`text` 至少填一个”。这两种写法分别等价于 type-level 的 `#[input(exactly_one_of("path", "stdin"))]` 和 `#[input(at_least_one_of("path", "stdin", "text"))]`，但当前字段路径会由宏自动补上；如果 peer 字段自己用了 rename/name/alias，宏也会把这些 peer path 解析回最终 wire key。

同样地，field 级 `#[arg(forbid_substrings = ["..", "~"])]` 和 `#[arg(distinct_trimmed)]` 也可以直接用在字符串字段或字符串数组字段上，分别对应 type-level 的 `#[input(forbid_substrings("path", "..", "~"))]` 与 `#[input(distinct_trimmed("tags"))]`。当目标字段本身是 `Vec<String>` 时，这两种 type-level 写法也会自动作用到 `tags[]`，不需要手写 `[]` 路径；前者适合路径、slug、host 之类不允许出现某些片段的输入，后者会在比较前先 trim，再拒绝 `" cargo "` 和 `"cargo"` 这类语义重复值。

如果 command handler 还需要 slash/raw/session/workspace 这类原始命令上下文，可以额外接收一个 `PluginCommandContext<'_>` 参数；它可以放在结构化输入参数之前或之后：

```rust
#[command("/note-greet", id = "notes.greet", title = "Greet")]
fn greet(
    &self,
    input: &ManifestCommandInput,
    context: PluginCommandContext<'_>,
) -> String {
    format!("hello {} via {}", input.name, context.slash.unwrap_or(context.command_id))
}
```

如果直接接收 `PluginCommandInvokeInput`，就已经包含完整原始上下文；这时不要再额外声明 `PluginCommandContext<'_>`。

`#[command]` handler 的返回值会经过 `IntoPluginCommandOutput`。除了直接返回 `PluginCommandOutput`，也可以返回 `String`、`&str`、`()`、`Option<_>` 和 `Result<_, _>`。如果 command 想继续跳转到另一个 plugin command，可以直接返回 `PluginCommandOutput::InvokeCommand { command, input }`；`Option<_>` 形式的返回值在 `None` 时会自动变成 `PluginCommandOutput::None`。

旧的聚合 enum/suite 写法已经移除。插件应统一使用 `#[agena_plugin]` 的方法级写法：每个模型可见 tool 对应一个 `#[tool(...)]` 方法，宏生成隐藏 schema、manifest、静态分发、stream fallback 和 permission 分发。完整可运行版本见 `examples/multi_tool_plugin_stdio`。

## Plugin UI

Plugin UI 是 manifest 的一部分，由 plugin host 统一聚合后提供给 TUI、Studio Web 和 API。它不再是 Studio 前端单独维护的一套 plugin UI registry；前端只消费 runtime 当前 plugin host 给出的 catalog。

Manifest 的 UI surface 明确拆成两个宿主面：

| 字段 | 面向宿主 | 内容 |
| --- | --- | --- |
| `ui.tui.statusline_segments` | TUI | 静态状态栏片段。也可以继续通过 `host/ui.statusline.*` 动态贡献和删除。 |
| `ui.tui.themes` | TUI | TUI theme palette。也可以通过 theme host callback 动态注册。 |
| `ui.tui.content_blocks` | TUI | TUI 文本块，当前默认位置是 `composer_footer`。 |
| `ui.studio.controls` | Studio Web/Desktop | Plugin detail panel 等位置上的按钮、选择器、开关、文本和数字 controls。 |
| `ui.studio.views` | Studio Web/Desktop | Plugin detail 或其他 Studio 位置可渲染的 markdown/link/control 组合视图。 |

Manifest 顶层 `commands` 是 Command Palette 和 slash command 可发现的 plugin commands。它不是 `ui.studio` 的子字段，因为 command 是 plugin 的独立能力描述；Studio/TUI 可以各自决定如何展示。

对带 `input_schema` 的 command，Studio 命令面板和聊天 slash 命令会优先按结构化输入处理参数：

- 如果 schema 不是 object，合法 JSON scalar/array 会直接作为 command/tool input。
- 如果 schema 是 object，合法 JSON object 会直接作为 command/tool input。
- 如果 schema 是多字段 object，也可以使用 `key=value` 空格分隔的轻量写法。
- 如果 schema 是单字段 object，裸文本参数或单个 JSON scalar 会自动映射到该字段，便于 `/plugin-command hello` 或 `/plugin-command 3` 这类简单调用。
- 其他情况会保留 legacy `{ "args": "..." }` 透传，适合自定义解析逻辑或兼容旧 command。

如果 command 没有显式写 `usage`，宏会优先根据 `ToolInput::input_example()` 或 inline `#[arg(example = ...)]` 自动推导一个更适合手输的示例；如果没有显式 example，则继续从 schema 里的 `examples`、`default`、`const`、`enum` 和必填字段类型生成占位值。现在派生出来的 `ToolInput::input_example()` 在没有显式 example 时，也会自动回退到这套 schema 推导结果；而 `input_usage()` 在已经有显式 example 的情况下，也会继续用 schema 补齐缺失字段，尽量避免生成一个本身就缺少必填字段的 usage。这个补齐逻辑同样适用于来自 `flatten_shape` / `nested_shape` 片段的示例，所以内层只提供了部分 example 时，外层必填字段也会继续补齐。这样单字段输入会尽量生成 `/cmd hello` 或 `/cmd <name>`，多字段 object 会尽量生成 `/cmd key=value ...`，只有复杂值才回退成 JSON。

TUI 和 Studio 的 UI contribution 类型不同，是有意的边界：TUI 只处理可在终端稳定渲染的状态栏、主题和文本块；Studio 可以渲染 manifest commands、controls、views，并把用户操作映射为统一的 `PluginUiAction`。

TUI theme palette 使用语义色键，而不是要求插件为每个 widget 指定颜色。支持的键为
`muted`、`accent`、`info`、`success`、`warning`、`danger`、`special`、
`selection_fg` 和 `selection_bg`。颜色值只能使用 `reset`、标准 ANSI 名称、
snake_case 的亮色名称（例如 `light_red`、`dark_gray`）或 `#RRGGBB`。主题格式只接受
上述新 schema，不包含旧键名、别名或兼容映射；未知键和非法颜色会使 theme 反序列化
失败。未提供的键使用 Agena 针对当前亮色/暗色终端选择的高对比度默认值。
`statusline_segments.color`、`content_blocks.color` 和动态 statusline/theme host API 使用
同一个严格颜色类型，不再接受自由格式字符串。

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
| `invoke_command` | 调用当前 plugin 的某个 plugin command；`input` 是默认 JSON object。command handler 返回的 `message` / `submit_prompt` / `invoke_tool` / `open_route` / `open_url` 由宿主继续解释。 |
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
            ToolDefinition::new(
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
        .command(PluginCommandDefinition {
            id: "summarize-workspace".into(),
            title: "Summarize workspace".into(),
            description: "Run the project helper summary tool.".into(),
            category: "Project".into(),
            slash: Some("/project-summary".into()),
            aliases: vec!["workspace summary".into()],
            usage: Some("/project-summary".into()),
            location: "command_palette".into(),
            input_schema: None,
            handler: None,
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

这两个 REST 入口都会经过 plugin host 的 tool registry 和 permission 检查。当前 direct UI invocation 没有交互式 permission confirmation 流；如果调用需要 `ask_permission` 或被 deny，API 会返回 409，Studio 需要把结果作为无法直接执行的操作处理。

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
| permission | `permission.ask_permission` |
| command/shell | `command.execute.before`, `command.execute.after`, `shell.env` |
| session | `session.start`, `session.end` |
| run | `pre_run`, `post_run`, `user.prompt.submit`, `agent.stop` |
| event/notification | `event`, `notification` |
| config | `config` |

Provider-visible prompt content is append-only within a session so upstream prompt caches can match stable prefixes. For that reason `chat.message`, `chat.messages.transform`, and `chat.system.transform` are not applied on the provider request path: they can rewrite, drop, or reorder system/messages. `chat.params` and `chat.headers` still apply because they do not mutate the transcript prefix.

典型用途：

- 在 `provider.list` 中注入 plugin-provided provider。
- 在 `chat.headers` 中给 provider request 增加 header。
- 在 `chat.params` 中调整 temperature、max output tokens 等 request 参数。
- 在 `permission.ask_permission` 中为组织策略提供建议或决策。
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
| `ToolRegistry` | 动态注册或注销 tools |
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

1. Tool manifest 在 `permissions` 中声明 `input_paths`、`input_networks`、`network_access`。
2. Plugin 可以在运行时通过 `permission_paths` / `permission_networks` 补充动态审计项。
3. Permission runtime 检查 path/network/tool policy。
4. `permission.ask_permission` hooks 可以给出建议；拥有 `PermissionDecision` capability 的 plugin 可以返回最终决策。
5. 需要用户确认时，session 状态和 UI/API 会产生 pending permission request。

Manifest 中的权限声明适合 tool 调用前就能知道的资源：

```rust
ToolDefinition::new("download", schema)
    .input_path(InputPathSpec {
        jsonpath: "$.output_path".to_string(),
        kind: PathKind::Write,
        optional: false,
    })
    .path_access(PathAccessSpec {
        path: "~/agena/plugin-cache".to_string(),
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

如果路径或网络目标要先解析输入、读取 plugin 状态、展开 workspace 信息后才能知道，优先把动态权限写在 `#[tool(...)]` 上。宏会先解析 tool input，再在执行 tool body 前返回这些审计项：

```rust
#[tool(
    summary = "Download a file.",
    mutating,
    path(write = self.output_path(input).await?),
    network(connect = input.url.as_str())
)]
async fn download(&self, input: &DownloadInput) -> Result<DownloadOutput> {
    // ...
}

#[tool(
    summary = "Apply a patch.",
    mutating,
    path(requests = self.planned_patch_paths(input)?)
)]
async fn patch(&self, input: &PatchInput) -> Result<PatchOutput> {
    // ...
}
```

`path(read = ...)` / `path(write = ...)` 接收单个路径，也可以直接接收 `Option<_>` 或 `Result<Option<_>>`；`None` 会被跳过。`path(reads = ...)` / `path(writes = ...)` 接收路径集合，支持 `Vec`、数组、slice、`Option<_>` 和 `Result<_>`。`path(requests = ...)` 接收 `PathRequest`、`Vec<PathRequest>`、数组、slice、`Option<_>`、`Result<_>` 或 `()`。网络侧对应 `network(connect = ...)`、`network(connects = ...)` 和 `network(requests = ...)`。这些声明和动态返回项都会在 tool body 执行前进入同一套 path/network policy。

Plugin 内部发起的额外文件或网络操作不能由 host 做强沙箱隔离。需要在 plugin 内部配合权限系统时，manifest 要声明 `PermissionCheck` capability，然后通过 host callback 主动检查：

```rust
host.ensure_path_permission(HostPathPermissionCheckRequest::write(path)).await?;
host.ensure_network_permission(HostNetworkPermissionCheckRequest::connect(url)).await?;
```

也可以使用 `check_path_permission` / `check_network_permission` 拿到 `allow`、`prompt`、`deny` 结果自行处理。Host 会按当前 session、agent、persisted rule、permission hook 和静态 policy 解析该检查。

Tool 权限配置分为 tag、tool name 和 tool-specific rules：

```json
{
  "permission": {
    "tools": {
      "tags": {
        "filesystem_read": "allow",
        "filesystem_write": "ask",
        "network": "ask",
        "internet": "ask",
        "task": "ask",
        "shell": "ask"
      },
      "names": {
        "agena.process.run": "ask",
        "agena.fs.read": "ask",
        "example.echo.echo": "allow"
      }
    }
  }
}
```

`names` 覆盖 runtime-provided 和 user-configured plugin tools。

## MCP

MCP server 配置在 `plugins.list."agena.mcp".config`，并通过 `agena.mcp` plugin tools 对模型暴露。

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

Remote HTTP:

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

Runtime build 时：

1. 从 `plugins.list."agena.mcp".config` 读取 MCP server config。
2. 构建 `McpConnectionManager`。
3. 注册 `agena.mcp` static plugin。
4. `agena.mcp` 从 MCP manager 读取 tool/resource/prompt capabilities。
5. 每个 MCP capability 进入 plugin tool registry。

因此，MCP 的权限、catalog、调用、hook、status 都落在 plugin 体系中。
`agena.mcp` 现在只接受 `stdio` 和 streamable HTTP 两种连接方式；旧的 WebSocket transport 已移除。

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
~/agena/plugin-storage
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
- `config`
- `min_agena_version`
- `archive`
- `dependencies`

安装时，marketplace client 会解析 registry、选择版本、下载 artifact、校验 hash/signature、写入 active config 的 `plugins.list.<id>`，并记录安装元数据。

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

- `--force`: 覆盖已有同名 plugin tool。
- `--dry-run`: 计算结果但不写文件。
- `--allow-unverified`: 允许没有 sha256 的 artifact。
- `--require-signature`: 要求 registry record 带 signature。
- `--refresh`: 安装前刷新 registry index。

Marketplace cache 默认目录：

```text
~/agena/marketplace
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
agena plugin inspect <plugin-id> --format json
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
3. 在 `manifest()` 中声明 hooks、tools、commands、capabilities 和 config schema。
4. 在 `tool_invoke` 或相关 hook 方法中实现行为。
5. 按 transport 导出 plugin。
6. 在 `agena.json` 的 `plugins.list.<id>` 中配置。
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
- package/config/timeouts/env/restart 等 plugin config 变化。
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
- Provided tool ids and bridge: `crates/agena/src/tool/mod.rs`
- Provided plugins: `crates/agena/src/plugins/provided/`
- MCP plugin bridge: `crates/agena/src/plugins/provided/mcp.rs`
- Plugin storage/secrets: `crates/agena/src/plugins/storage.rs`
- Marketplace manifest/install/cache: `crates/agena-plugin-marketplace/`
- CLI plugin commands: `crates/agena/src/cli.rs`
- Backend plugin APIs: `crates/agena-api-server/src/lib.rs` and `crates/agena-api-server/src/rest.rs`
