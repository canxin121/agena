# 后端 API

本文描述 Agena 后端接口。这里的“后端 API”主要指 Studio server 挂载的 Axum 服务，以及底层 `agena-api-server` 提供的 REST、SSE、WebSocket 和 JSON-RPC app-server transport。

实现位置：

- Studio server: `apps/agena-studio-server/`
- API router/handlers: `crates/agena-api-server/src/lib.rs`、`rest.rs`、`sse.rs`、`ws.rs`
- Shared protocol types: `crates/agena-api/`
- Rust SDK: `crates/agena-client/`
- Studio Web API wrapper: `packages/agena-studio-web/src/agena/lib/agenaApi.ts`

## 服务层级

`agena-studio` 启动后会挂载两层路由：

```text
Studio public routes
  GET  /health
  GET  /auth/session
  POST /auth/session

Agena API server routes
  GET  /healthz
  GET  /readyz
  GET  /metrics
  /api/v1/...
  /plugin-rpc/{plugin_id}
```

Studio 只提供 Agena 原生 API。会话、配置、运行时诊断和文件系统能力都走 `/api/v1/*` 与 Studio public routes。

如果传入 `--ui-dir <dist>`，Studio server 还会服务静态 UI：

- `/assets/...`
- fallback 到 `index.html`

如果不传 `--ui-dir`，服务运行在 API-only 模式。

## 启动

```bash
agena-studio \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace-root "$PWD" \
  --config ~/.agena/config.json
```

常用参数和环境变量：

```text
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
--config / AGENA_CONFIG
--set key=value
```

默认监听 `127.0.0.1:3210`。

## 鉴权

Studio UI 鉴权是 Studio server 层的 middleware，不是 `agena-api` wire protocol 的一部分。

如果未设置 `AGENA_STUDIO_UI_PASSWORD` 或密码为空：

- UI auth 关闭。
- `/auth/session` 会报告已认证且 `disabled: true`。
- `/api/v1/...` 不要求 UI token。

如果设置了 UI 密码：

- `POST /auth/session` 用密码创建 session。
- 返回 token，同时设置 `agena_ui_session` HttpOnly cookie。
- token TTL 为 12 小时，访问时刷新 `last_seen`。
- 失败登录按 client key 限速，10 分钟窗口内 8 次失败后锁定 15 分钟。

### `GET /auth/session`

检查 UI session。

响应示例：

```json
{
  "authenticated": true,
  "disabled": true
}
```

需要认证但缺少/过期 token 时返回 401：

```json
{
  "authenticated": false,
  "locked": true
}
```

### `POST /auth/session`

请求：

```json
{
  "password": "..."
}
```

成功：

```json
{
  "authenticated": true,
  "token": "..."
}
```

失败可能返回：

- 400 `auth_disabled`
- 401 `auth_invalid_password`
- 429 `auth_rate_limited`

### 认证方式

API middleware 接受：

```text
Authorization: Bearer <token>
```

也接受 `agena_ui_session` cookie。跨站 cookie 场景下，unsafe methods 会检查 `Origin` 是否允许。

WebSocket 使用同一套 UI auth middleware。浏览器客户端通常通过 REST 和 session SSE 访问 Studio；需要 WebSocket 时使用 `/api/v1/ws`。

## CORS

当设置 `--cors-origin` 或 `AGENA_STUDIO_CORS_ORIGINS` 时，server 会允许指定 origin，并允许 credentials。

当设置 `--cors-allow-all` 或 `AGENA_STUDIO_CORS_ALLOW_ALL=true` 时：

- `Access-Control-Allow-Origin: *`
- credentials 关闭。

允许的方法：

```text
GET
POST
PUT
DELETE
PATCH
OPTIONS
```

允许 header 包括：

```text
Accept
Content-Type
Authorization
If-Match
If-None-Match
Last-Event-Id
```

## 健康检查与指标

### `GET /health`

Studio 层健康检查。响应使用 camelCase：

```json
{
  "status": "ok",
  "generation": 1,
  "loadedAt": "2026-05-13T00:00:00Z",
  "workspaceRoot": "/repo",
  "configPath": "/home/user/.agena/config.json",
  "configFound": true,
  "providerIds": ["anthropic"],
  "sessionRuntimeAvailable": true
}
```

### `GET /healthz`

轻量 liveness probe。返回 `200 OK` 和文本 `ok`，不读取 runtime state。

### `GET /readyz`

readiness probe。runtime snapshot generation 大于 0 时返回 `200 ready`，否则返回 `503 loading`。

### `GET /metrics`

Prometheus-style text metrics，包括：

- runtime generation。
- runtime reload count。
- HTTP request count。
- HTTP duration histogram。
- provider call/error/stream counters。
- tool execution/error counters。
- active session gauge。
- process uptime。
- build info。

## 错误格式

REST handler 有两类错误 envelope。

`agena-api` 统一 transport 错误：

```json
{
  "code": "bad_request",
  "message": "..."
}
```

或在 WebSocket `error` frame 中被 flatten：

```json
{
  "type": "error",
  "id": "req-1",
  "code": "bad_request",
  "message": "..."
}
```

Local REST service 错误：

```json
{
  "error": {
    "code": "bad_request",
    "message": "..."
  }
}
```

Studio UI auth 错误：

```json
{
  "error": "UI authentication required",
  "locked": true,
  "code": "auth_required"
}
```

常见 HTTP status：

- 400 bad request。
- 401 unauthenticated/UI auth required。
- 403 CSRF origin forbidden。
- 404 not found。
- 409 optimistic concurrency conflict。
- 429 auth rate limited。
- 503 service unavailable。
- 500 internal error。

## 分页

多数 list endpoint 使用 cursor pagination：

请求 query：

```text
cursor=<opaque>
limit=<number>
search=<optional>
```

响应：

```json
{
  "items": [],
  "page": {
    "next_cursor": "...",
    "has_more": false,
    "returned": 0,
    "order": "desc"
  }
}
```

不同 endpoint 的 `page` 可能只返回核心字段；前端封装以 `has_more` 和 `next_cursor` 为主。

## Optimistic concurrency

Session update、delete、message submit、continue、fork、permission reply、user input reply、rewind 等写操作可使用：

```text
If-Match: <session.version>
```

版本不匹配时返回 409。

## REST API

以下路由由 `agena-api-server::router` 挂载。

### Runtime

| Method | Path                     | 说明                                                                           |
| ------ | ------------------------ | ------------------------------------------------------------------------------ |
| GET    | `/api/v1/health`         | API server health，返回 status、generation、loaded_at、database_connected      |
| GET    | `/api/v1/runtime`        | runtime status、config path、providers、plugins、reload/janitor、operator 信息 |
| POST   | `/api/v1/runtime/reload` | 手动 reload runtime config                                                     |
| GET    | `/api/v1/git/status`     | workspace 的 git/gh 状态                                                       |

`GET /api/v1/runtime` 响应包含：

- `generation`
- `loaded_at`
- `workspace_root`
- `config_path`
- `config_found`
- `provider_ids`
- `plugin_count`
- `session_runtime_available`
- `watch_paths`
- `reload`
- `janitor`
- `session_cache`
- `automation`
- `operator.mcp`
- `operator.lsp`
- `operator.agents`
- `operator.skills`
- `operator.ui`

### Settings

Settings API 操作当前 runtime 使用的 `config.json`。读接口可以读 resolved effective config，也可以直接读文件；写接口只编辑文件，并在实际变更且请求 reload 时自动触发 runtime reload。

| Method | Path                         | 说明                                                                      |
| ------ | ---------------------------- | ------------------------------------------------------------------------- |
| GET    | `/api/v1/settings`           | 读取一个 setting。query: `path`、`source=effective|file`                   |
| GET    | `/api/v1/settings/entries`   | 列出 setting entries。query: `path`、`source=effective|file`、`recursive` |
| PUT    | `/api/v1/settings`           | 设置一个 TOML path 的值                                                   |
| PATCH  | `/api/v1/settings`           | 深度合并一个 JSON object 到目标 TOML table；`null` 表示删除 key           |
| DELETE | `/api/v1/settings`           | 删除一个 TOML path。query: `path`、`dry_run`、`validate`、`reload`         |
| POST   | `/api/v1/settings/validate`  | 校验当前 `config.json` 能否被 runtime 加载；body 可为空                     |

Path 使用点分语法。带点的 table key 用引号包起来，例如 `mcp.servers.filesystem`。

`source=effective` 读取已经应用默认值、文件、环境变量和 CLI override 后的 resolved config，path 根节点是 resolved config 本身，例如 `runtime.reload.enabled`。`source=file` 读取原始 `config.json`，不会补默认值。

`PUT /api/v1/settings` 请求：

```json
{
  "path": "runtime.reload.poll_interval_secs",
  "value": 2,
  "dry_run": false,
  "validate": true,
  "reload": true
}
```

`PATCH /api/v1/settings` 请求：

`web.search` 是本地 crawl-index 搜索，不再有 API key 设置。要让搜索返回结果，需要先用 `crawl` 工具采集页面并建立本地索引。

写入响应包含：

- `config_path`、`config_found`
- `operation`
- `path`
- `dry_run`
- `changed`
- `created`
- `deleted`
- `validated`
- `reload_requested`
- `reload_required`
- `reload`
- `previous`
- `current`

`reload` 只在实际写入且 reload 成功时出现：

```json
{
  "previous_generation": 1,
  "generation": 2,
  "loaded_at": "2026-05-13T00:00:00Z"
}
```

### Plugins and marketplace

| Method | Path                                    | 说明                                              |
| ------ | --------------------------------------- | ------------------------------------------------- |
| GET    | `/api/v1/plugins`                       | plugin runtime status list                        |
| GET    | `/api/v1/plugins/ui`                    | 当前 runtime 的统一 plugin UI catalog             |
| POST   | `/api/v1/plugins/ui/invoke-tool`        | Studio/TUI 支持面直接调用 plugin tool             |
| GET    | `/api/v1/plugins/{plugin_id}`           | plugin inspect，包含 status、manifest、authority  |
| GET    | `/api/v1/plugins/{plugin_id}/logs`      | plugin retained logs，query: `after_seq`、`limit` |
| POST   | `/api/v1/plugins/{plugin_id}/ui/actions/{action_id}` | 执行 manifest Studio UI action       |
| POST   | `/api/v1/plugins/marketplace/search`    | 搜索 registry                                     |
| POST   | `/api/v1/plugins/marketplace/sync`      | 同步 registry index                               |
| GET    | `/api/v1/plugins/marketplace/installed` | 已安装 marketplace plugins                        |
| GET    | `/api/v1/plugins/marketplace/outdated`  | 可升级 plugins                                    |
| POST   | `/api/v1/plugins/marketplace/install`   | 安装 plugin                                       |
| POST   | `/api/v1/plugins/marketplace/uninstall` | 卸载 plugin                                       |
| POST   | `/api/v1/plugins/marketplace/upgrade`   | 升级一个或全部 plugin                             |

`GET /api/v1/plugins/ui` 返回 plugin host 聚合后的 UI catalog，形状为：

```json
{
  "catalog": {
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
}
```

`operator.ui` 在 `GET /api/v1/runtime` 中提供同一个 catalog，方便 Studio bootstrap 时一次拿到 runtime 状态和 plugin UI。

`POST /api/v1/plugins/ui/invoke-tool` 请求：

```json
{
  "plugin_id": "project-helper",
  "tool": "summarize",
  "input": { "scope": "workspace" },
  "session_id": 42
}
```

`plugin_id` 可省略；省略时 `tool` 按 exposed tool name 查找。带 `plugin_id` 时，`tool` 可以是 plugin manifest 中的原始 tool name，也可以是 registry 暴露名。`input` 必须是 JSON object 或 null。

响应：

```json
{
  "plugin_id": "project-helper",
  "tool": "project-helper/summarize",
  "title": "summarize",
  "output_text": "Summary...",
  "payload": {},
  "metadata": {}
}
```

`POST /api/v1/plugins/{plugin_id}/ui/actions/{action_id}` 执行 manifest 中 `ui.studio.commands`、`ui.studio.controls` 或 `ui.studio.views[*].controls` 对应的 action。请求 body 可传：

```json
{
  "input": { "scope": "current-session" },
  "session_id": 42
}
```

如果 action 是 `invoke_tool`，后端会把 manifest action 自带的 `input` 和请求 `input` 合并，请求字段覆盖默认字段，然后通过 plugin host 执行 tool。`open_route`、`open_url`、`submit_prompt` 和 `none` 不在后端产生副作用，响应会原样返回 action，Studio 前端负责本地行为。

Studio controls 会通过同一个 action endpoint 执行。按钮不额外传值；`select`、`toggle`/`checkbox`/`switch`、`text`、`number` 等输入型 controls 会把当前值放到请求 `input.value`，再按上面的合并规则交给 `invoke_tool` action。

Direct UI invocation 会经过 tool registry 和 permission check。当前没有交互式 permission confirmation 响应通道；如果调用需要 `ask_permission` 或被 deny，接口返回 409。

Marketplace search/install 请求示例：

```json
{
  "registry_id": "default",
  "registry_url": "https://example.com/marketplace.json",
  "query": "lint",
  "refresh": true
}
```

```json
{
  "spec": "plugin-id@1.2.3",
  "registry_id": "default",
  "registry_url": "https://example.com/marketplace.json",
  "config_path": "~/.agena/config.json",
  "force": false,
  "dry_run": false,
  "allow_unverified": false,
  "refresh": false,
  "require_signature": false
}
```

### Auth providers

| Method | Path                                                 | 说明                                       |
| ------ | ---------------------------------------------------- | ------------------------------------------ |
| GET    | `/api/v1/auth/providers`                             | 列出公开 auth providers 和 credential 状态 |
| GET    | `/api/v1/auth/providers/{provider_id}`               | 获取一个 auth provider                     |
| DELETE | `/api/v1/auth/providers/{provider_id}`               | 删除 credential                            |
| PUT    | `/api/v1/auth/providers/{provider_id}/api-key`       | 写入 API key                               |
| POST   | `/api/v1/auth/providers/{provider_id}/refresh`       | 刷新支持的 OAuth credential                |
| POST   | `/api/v1/auth/providers/openai/browser/start`        | OpenAI browser OAuth start                 |
| POST   | `/api/v1/auth/providers/openai/browser/finish`       | OpenAI browser OAuth finish                |
| POST   | `/api/v1/auth/providers/openai/device/start`         | OpenAI device flow start                   |
| POST   | `/api/v1/auth/providers/openai/device/poll`          | OpenAI device flow poll                    |
| POST   | `/api/v1/auth/providers/gitlab/browser/start`        | GitLab browser OAuth start                 |
| POST   | `/api/v1/auth/providers/gitlab/browser/finish`       | GitLab browser OAuth finish                |
| POST   | `/api/v1/auth/providers/atomgit/browser/start`       | AtomGit broker OAuth start                 |
| POST   | `/api/v1/auth/providers/atomgit/browser/poll`        | AtomGit broker OAuth poll                  |
| POST   | `/api/v1/auth/providers/github-copilot/device/start` | GitHub Copilot device flow start           |
| POST   | `/api/v1/auth/providers/github-copilot/device/poll`  | GitHub Copilot device flow poll            |

`PUT /api/v1/auth/providers/{provider_id}/api-key`:

```json
{
  "api_key": "..."
}
```

Browser OAuth start:

```json
{
  "redirect_uri": "http://127.0.0.1:1455/callback"
}
```

GitLab browser start additionally requires:

```json
{
  "instance_url": "https://gitlab.com",
  "redirect_uri": "http://127.0.0.1:1455/callback"
}
```

AtomGit browser poll:

```json
{
  "provider_id": "atomgit",
  "state": "..."
}
```

### Providers

| Method | Path                                     | 说明                  |
| ------ | ---------------------------------------- | --------------------- |
| GET    | `/api/v1/providers`                      | provider summary list |
| GET    | `/api/v1/providers/{provider_id}/models` | provider models       |

Provider summary:

```json
{
  "provider_id": "anthropic",
  "defaults": {
    "adapter": "anthropic",
    "model": "claude-sonnet-4-6"
  },
  "adapters": [
    {
      "adapter_id": "anthropic",
      "enabled": true,
      "configured_model_count": 1
    }
  ]
}
```

Provider models:

```json
{
  "provider_id": "openai",
  "models": [
    {
      "provider_id": "openai",
      "adapter_id": "openai",
      "id": "gpt-5",
      "catalog_model_id": "gpt-5",
      "display_name": "GPT-5",
      "capabilities": {
        "tool_calling": "supported",
        "streaming": "supported",
        "reasoning": "supported",
        "structured_output": "supported",
        "temperature_supported": "unsupported"
      },
      "metadata": {
        "lifecycle": "active",
        "description": "Latest flagship model",
        "limits": {
          "context_window_tokens": 400000,
          "max_output_tokens": 16384
        }
      },
      "thinking_modes": {
        "high": {
          "display_name": "High",
          "description": "Higher reasoning effort",
          "thinking": {
            "type": "effort",
            "effort": "high"
          }
        }
      },
      "speed_modes": {
        "fast": {
          "display_name": "Fast",
          "description": "Priority tier",
          "request_override": {
            "body_patch": {
              "service_tier": "priority"
            }
          }
        }
      }
    }
  ]
}
```

`models` 是 runtime 当前从 provider / adapter 探测到的 live model 列表。`id` 是 backend-visible 的真实 model id，`adapter_id` 会单独返回；Agena 不再把它们拼成 `"<adapter>/<model>"` 这种 route 字符串。Studio Web 和 TUI 用这些 live models 作为 catalog draft 和 provider-local model draft 的来源。

### Model Catalog

| Method | Path                            | 说明                                                                  |
| ------ | ------------------------------- | --------------------------------------------------------------------- |
| GET    | `/api/v1/model-catalog`         | paginated catalog query，支持 `q` / `origin` / `offset` / `limit` |
| POST   | `/api/v1/model-catalog/lookup`  | lookup 一组 `model_id`，返回最匹配的 catalog entries                   |
| POST   | `/api/v1/model-catalog/refresh` | refresh official catalog，按 source 优先级重新拉 public sources 并 merge live provider model lists |

Model catalog list response:

```json
{
  "summary": {
    "last_refresh_at": "2026-05-15T08:00:00Z",
    "last_successful_source": "generated",
    "entry_count": 984
  },
  "total": 2,
  "offset": 0,
  "limit": 50,
  "available_origins": ["Anthropic", "Google", "OpenAI"],
  "items": [
    {
      "model_id": "gpt-5",
      "source": "generated",
      "source_label": "generated catalog",
      "display_name": "GPT-5",
      "lifecycle": "active",
      "context_window_tokens": 400000,
      "max_output_tokens": 16384,
      "description": "Latest flagship model",
      "features": {
        "supported": [
          "tool_calling",
          "streaming",
          "reasoning",
          "structured_output"
        ],
        "unsupported": ["temperature"]
      },
      "thinking_modes": {
        "high": {
          "display_name": "High",
          "description": "Higher reasoning effort",
          "thinking": {
            "type": "effort",
            "effort": "high"
          }
        }
      },
      "speed_modes": {
        "fast": {
          "display_name": "Fast",
          "description": "Priority tier",
          "request_override": {
            "headers": {
              "openai-beta": "fast-mode-2026-02-01"
            },
            "body_patch": {
              "service_tier": "priority"
            }
          },
          "adapter_overrides": {
            "openai": {
              "body_patch": {
                "service_tier": "priority"
              }
            }
          }
        }
      }
    }
  ]
}
```

`items` 只包含官方 catalog 条目。Model catalog 不再保存 default model；默认 provider/adapter/model/agent 应写入配置文件的 `[execution]`。官方 catalog 主要来自公开 online sources，再叠加 live provider model lists；catalog 会把 thinking 和 speed modes 保留在同一个 model entry 下，不会展开成新的模型 id。

### Workspaces

| Method | Path                                      | 说明                     |
| ------ | ----------------------------------------- | ------------------------ |
| GET    | `/api/v1/workspaces`                      | list workspaces          |
| POST   | `/api/v1/workspaces`                      | create workspace         |
| POST   | `/api/v1/workspaces/resolve`              | resolve path，必要时创建 |
| GET    | `/api/v1/workspaces/{workspace_id}`       | get workspace            |
| PUT    | `/api/v1/workspaces/{workspace_id}`       | replace workspace path   |
| DELETE | `/api/v1/workspaces/{workspace_id}`       | delete workspace         |
| GET    | `/api/v1/workspaces/{workspace_id}/files` | list workspace file tree |

List query：

```text
cursor
limit
search
include_session_count
```

Create/replace：

```json
{
  "path": "/path/to/workspace"
}
```

Resolve：

```json
{
  "path": "/path/to/workspace",
  "create_if_missing": true
}
```

Files query：

```text
path=<relative-path>
depth=<0..8>
limit=<1..2000>
```

### Sessions

| Method | Path                                               | 说明                             |
| ------ | -------------------------------------------------- | -------------------------------- |
| GET    | `/api/v1/sessions`                                 | list sessions                    |
| POST   | `/api/v1/sessions`                                 | create session                   |
| POST   | `/api/v1/sessions/import`                          | import JSONL session             |
| GET    | `/api/v1/sessions/tree/{root_id}`                  | list session tree                |
| GET    | `/api/v1/sessions/{session_id}`                    | get session                      |
| PUT    | `/api/v1/sessions/{session_id}`                    | update session title/parent      |
| DELETE | `/api/v1/sessions/{session_id}`                    | delete session                   |
| GET    | `/api/v1/sessions/{session_id}/state`              | execution state                  |
| GET    | `/api/v1/sessions/{session_id}/events`             | session events                   |
| GET    | `/api/v1/sessions/{session_id}/events/stream`      | session event SSE                |
| GET    | `/api/v1/sessions/{session_id}/messages`           | session messages                 |
| POST   | `/api/v1/sessions/{session_id}/messages`              | submit user message                 |
| POST   | `/api/v1/sessions/{session_id}/continue`           | continue blocked/incomplete run  |
| POST   | `/api/v1/sessions/{session_id}/fork`               | fork session                     |
| POST   | `/api/v1/sessions/{session_id}/cancel`             | cancel active run               |
| POST   | `/api/v1/sessions/{session_id}/permission-replies` | reply to permission request      |
| POST   | `/api/v1/sessions/{session_id}/user-input-replies` | reply to host/user input request |
| POST   | `/api/v1/sessions/{session_id}/rewind`             | fork session at message          |
| GET    | `/api/v1/sessions/{session_id}/export`             | export session JSONL             |
| GET    | `/api/v1/sessions/{session_id}/rewind-checkpoints` | list rewind audit checkpoints    |

List sessions query：

```text
cursor
limit
workspace_id
parent_id
roots
search
```

Create:

```json
{
  "workspace_id": 1,
  "title": "New session",
  "parent_id": null
}
```

Update:

```json
{
  "title": "Renamed",
  "parent_id": null
}
```

Run options shared by message submit/continue/replies:

```json
{
  "model": {
    "provider_id": "anthropic",
    "adapter_id": "anthropic",
    "model_id": "claude-sonnet-4-6"
  },
  "thinking_mode": "deep",
  "speed_mode": "fast",
  "agent_profile": "build",
  "system": "optional system prompt override",
  "temperature": 0.2,
  "max_output_tokens": 4096
}
```

Submit message:

```json
{
  "model": {
    "provider_id": "anthropic",
    "adapter_id": "anthropic",
    "model_id": "claude-sonnet-4-6"
  },
  "thinking_mode": "deep",
  "agent_profile": "build",
  "parts": [
    {
      "type": "text",
      "text": "Explain this repo"
    }
  ]
}
```

`parts` 不能为空。响应为 `SessionExecutionResource`：

```json
{
  "session": { "id": 1, "title": "..." },
  "blocked": false,
  "run_state": "idle",
  "latest_event_seq": 123,
  "execution": {
    "model_provider_id": "anthropic",
    "model_id": "claude-sonnet-4-6"
  },
  "pending_permission_requests": [],
  "pending_user_input_requests": [],
  "prompt_usage": {
    "current_tokens": 32000,
    "budget_tokens": 170000,
    "model_context_window_tokens": 200000
  }
}
```

Permission reply:

```json
{
  "agent_profile": "build",
  "reply": {
    "request_id": "...",
    "kind": "allow_once",
    "reason": "optional",
    "scope": "session"
  }
}
```

`kind` 常用值：

```text
allow_once
allow_always
deny_once
deny_always
```

User input submit:

```json
{
  "reply": {
    "request_id": "...",
    "kind": "submit",
    "answers": {
      "choice": ["yes"]
    }
  }
}
```

User input cancel:

```json
{
  "reply": {
    "request_id": "...",
    "kind": "cancel",
    "reason": "cancelled"
  }
}
```

Fork:

```json
{
  "at_message_id": 42,
  "title": "Fork title"
}
```

Fork starts from the selected message id.

Rewind:

```json
{
  "message_id": 42
}
```

Rewind returns a new forked session rooted at the selected message, preserving the source session's provider-visible prompt as append-only.

Import:

```json
{
  "jsonl": "{...}\n{...}\n"
}
```

Export returns `application/x-ndjson` text.

### Messages

| Method | Path                                     | 说明                  |
| ------ | ---------------------------------------- | --------------------- |
| GET    | `/api/v1/sessions/{session_id}/messages` | list session messages |
| GET    | `/api/v1/messages/{message_id}`          | get message           |
| GET    | `/api/v1/messages/{message_id}/parts`    | list message parts    |
| GET    | `/api/v1/message-parts/{part_id}`        | get message part      |

Message list query：

```text
cursor
limit
parts=none|summary|full
```

Message detail query：

```text
parts=none|summary|full
```

Message parts query：

```text
mode=none|summary|full
```

### Permission rules

| Method | Path                                        | 说明               |
| ------ | ------------------------------------------- | ------------------ |
| GET    | `/api/v1/permission-rules`                  | list rules         |
| POST   | `/api/v1/permission-rules`                  | create/upsert rule |
| GET    | `/api/v1/permission-rules/{rule_id}`        | get rule           |
| PUT    | `/api/v1/permission-rules/{rule_id}`        | replace rule       |
| DELETE | `/api/v1/permission-rules/{rule_id}`        | delete rule        |
| POST   | `/api/v1/permission-rules/{rule_id}/revoke` | revoke rule        |

List query：

```text
cursor
limit
search
```

Write request:

```json
{
  "action_key": "...",
  "subject_kind": "tool",
  "tool_name": "bash",
  "qualifier": "git status",
  "path_access_kind": null,
  "workspace_root": null,
  "target_path": null,
  "network_target": null,
  "network_host": null,
  "network_port": null,
  "scope": "workspace",
  "session_id": null,
  "mode": "allow"
}
```

`mode`:

```text
allow
ask
deny
```

Revoke:

```json
{
  "reason": "revoked"
}
```

### Events

| Method | Path                    | 说明                                      |
| ------ | ----------------------- | ----------------------------------------- |
| GET    | `/api/v1/events`        | persisted event list                      |
| GET    | `/api/v1/events/stream` | global/workspace/session notification SSE |

`GET /api/v1/events` query:

```text
scope_kind=global|workspace|session
workspace_id=<required when workspace>
session_id=<required when session>
kinds=comma,separated,event_tags
since_seq_global=<seq>
limit=<1..1000>
```

Response:

```json
{
  "items": [],
  "page": {
    "next_cursor": null,
    "has_more": false,
    "returned": 0
  }
}
```

## SSE

There are two SSE surfaces.

### Session event stream

```text
GET /api/v1/sessions/{session_id}/events/stream
```

Query:

```text
after_seq
limit
poll_interval_ms
idle_timeout_ms
```

The handler first backfills events after `after_seq`, then subscribes to the live session event bus.

Event types:

- `session_event`: data is a domain event JSON object; SSE id is `seq_global`.
- `lagged`: data is the skipped count.
- `error`: data is a string error.

Example frame:

```text
event: session_event
id: 123
data: {"meta":{"seq_global":123},"kind":"...","payload":{}}

```

Studio Web uses this stream in `streamSessionEvents(...)` and reconnects with the last seen seq.

### Notification stream

```text
GET /api/v1/events/stream
```

Query:

```text
since_seq_global
scope_kind=global|workspace|session
workspace_id
session_id
kinds=comma,separated,event_tags
```

Every SSE event has:

```text
event: notification
data: <Notification JSON>
```

Notification shape comes from `agena-api/src/notifications.rs`:

```json
{
  "kind": "event",
  "data": {
    "subscription": "sse",
    "event": {}
  }
}
```

Lagged notification:

```json
{
  "kind": "lagged",
  "data": {
    "subscription": "sse",
    "skipped": 12
  }
}
```

The SSE response sends keepalive every 25 seconds.

## WebSocket

```text
GET /api/v1/ws
```

The WS protocol is defined in `crates/agena-api/src/ws.rs`. On connection, server sends:

```json
{
  "type": "hello",
  "protocol_version": 2
}
```

Client message types:

### Command

```json
{
  "type": "command",
  "id": "cmd-1",
  "method": "cancel_run",
  "params": {
    "session_id": 1
  }
}
```

Server success:

```json
{
  "type": "command_result",
  "id": "cmd-1",
  "result": "ack"
}
```

Exact result shape depends on `CommandResult` variant.

### Query

```json
{
  "type": "query",
  "id": "query-1",
  "method": "runtime"
}
```

Server responds with `query_result`.

### Subscribe

```json
{
  "type": "subscribe",
  "id": "sub-1",
  "scope": {
    "kind": "session",
    "session_id": 1
  },
  "kinds": null,
  "since_seq_global": 100
}
```

Server responds:

```json
{
  "type": "subscribed",
  "id": "sub-1"
}
```

Then sends:

```json
{
  "type": "notification",
  "kind": "event",
  "data": {
    "subscription": "sub-1",
    "event": {}
  }
}
```

### Unsubscribe

```json
{
  "type": "unsubscribe",
  "id": "sub-1"
}
```

### Ping

```json
{
  "type": "ping",
  "nonce": "optional"
}
```

Server replies with `pong`.

### Error

```json
{
  "type": "error",
  "id": "cmd-1",
  "code": "bad_request",
  "message": "..."
}
```

WebSocket frames use text JSON messages.

## JSON-RPC app-server

The CLI can run a stdio JSON-RPC server:

```bash
agena app-server --transport stdio --workspace /path/to/repo
```

Protocol files:

- `crates/agena-api-server/src/jsonrpc/protocol.rs`
- `crates/agena-api-server/src/jsonrpc/server.rs`
- backend implementation in `apps/agena-cli/src/main.rs`

JSON-RPC version is `2.0`; messages are newline-delimited JSON over stdin/stdout.

Supported methods:

```text
session/create
message/submit
permission/reply
sessions/list
messages/list
run/cancel
events/subscribe
```

### `session/create`

Params:

```json
{
  "title": "IDE session",
  "parent_session_id": null
}
```

Result:

```json
{
  "session_id": 1,
  "title": "IDE session"
}
```

### `message/submit`

Params:

```json
{
  "session_id": 1,
  "prompt": "hello",
  "model": "anthropic/claude-sonnet-4-6",
  "temperature": 0.2,
  "max_output_tokens": 1024
}
```

Result:

```json
{
  "session_id": 1,
  "status": "idle",
  "text": "assistant text"
}
```

### `permission/reply`

Params:

```json
{
  "session_id": 1,
  "request_id": "...",
  "decision": "allow",
  "reason": "ok",
  "remember": "workspace"
}
```

`decision`: `allow` or `deny`。

`remember`: `session`、`workspace`、`global`。

### `sessions/list`

Params:

```json
{
  "offset": 0,
  "limit": 20
}
```

### `messages/list`

Params:

```json
{
  "session_id": 1
}
```

### `run/cancel`

Params:

```json
{
  "session_id": 1
}
```

### JSON-RPC errors

Error codes:

- `-32601`: method not found。
- `-32602`: invalid params。
- `-32603`: backend/serialization/internal error。
- `-32004`: not found。

## Plugin RPC

```text
POST /plugin-rpc/{plugin_id}
```

This endpoint forwards `agena-plugin-sdk` JSON-RPC requests to a loaded plugin. It accepts optional bearer token from `Authorization` and returns plugin RPC response JSON.

It is intended for plugin UI/assets or external plugin management surfaces, not for normal chat/session API use.

## Rust SDK

`crates/agena-client` provides:

- `AgenaClient`: REST client for one-shot commands and queries.
- `WsClient`: WebSocket client for live subscriptions.

Example:

```rust
use agena_client::AgenaClient;

let client = AgenaClient::new("http://127.0.0.1:3210")?;
let health = client.health().await?;
```

High-level methods include:

- `health`
- `create_session`
- `submit_message`
- `continue_run`
- `cancel_run`
- `reply_permission`
- `reply_user_input`
- `list_events`
- generic `command`
- generic `query`

## Studio Web client

Studio Web uses `packages/agena-studio-web/src/agena/lib/agenaApi.ts`.

It wraps:

- runtime and health.
- plugin and marketplace APIs.
- provider/auth APIs.
- workspace/session/message APIs.
- permission rule APIs.
- session SSE.
- import/export.
- rewind/fork/tree/checkpoints.

Base fetch helpers in `packages/agena-studio-web/src/lib/api.ts`:

- prepend active backend base URL.
- attach `Authorization` header when active UI token exists.
- fallback to cookie credentials.
- parse structured backend errors.
- emit auth-required event on 401 `auth_required`。

## Test coverage

Relevant tests:

- `apps/agena-studio-server/src/git/regression_tests.rs`: end-to-end git endpoint coverage with real git subprocesses and temp repositories.
- `crates/agena/tests/live_provider_catalog.rs`: live provider catalog checks against external providers and remote public catalog sources. These tests are ignored by default.

## Implementation index

- Router table: `crates/agena-api-server/src/lib.rs`
- REST handlers: `crates/agena-api-server/src/rest.rs`
- SSE global notification stream: `crates/agena-api-server/src/sse.rs`
- WebSocket transport: `crates/agena-api-server/src/ws.rs`
- JSON-RPC app-server: `crates/agena-api-server/src/jsonrpc/`
- Local API DTO/service/errors: `crates/agena-api-server/src/local_api/`
- Command/query dispatch: `crates/agena-api-server/src/dispatch.rs`
- Shared API protocol: `crates/agena-api/`
- Rust client: `crates/agena-client/`
- Studio server auth/CORS/static UI: `apps/agena-studio-server/src/`
- Studio Web wrapper: `packages/agena-studio-web/src/agena/lib/agenaApi.ts`
