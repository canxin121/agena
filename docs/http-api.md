# Agena HTTP API

## Overview

`agena-api-server` 现在提供统一的 `v1` 服务端 API，重点面向：

- workspace / session / message / permission rule 的标准 CRUD
- 聊天类数据的高性能读取
- 统一的 keyset pagination
- 默认摘要、按需详情的懒加载
- 会话执行类动作接口
- 会话事件的 SSE 增量订阅
- 基于 session `version` 的乐观并发控制
- runtime / auth 的用户管理接口

启动：

```bash
cargo run -p agena-studio-server -- --help
```

常用参数：

- `--listen 127.0.0.1:8765`
- `--database-path ./data/agena.db`
- `--database-url sqlite:///absolute/path/to/agena.db?mode=rwc`
- `--workspace-root /path/to/workspace`

架构说明：

- core 能力保留在 `agena` crate
- HTTP API library 保留在 `crates/agena-api-server`
- HTTP API 可执行入口放在 `apps/agena-studio-server`
- 这样应用如果只需要 runtime / provider / session 等能力，就不必依赖额外的旧 server 入口

## Pagination

所有列表接口都支持：

- `limit`
- `cursor`

返回结构统一为：

```json
{
  "items": [],
  "page": {
    "limit": 50,
    "returned": 50,
    "has_more": true,
    "next_cursor": "opaque-cursor",
    "order": "desc"
  }
}
```

说明：

- `cursor` 是 opaque token，客户端不应自行解析
- `workspaces` / `sessions` / `permission-rules` 返回 `order = "desc"`
- `messages` / `session events` 返回 `order = "asc"`，但其 `next_cursor` 指向更旧的数据窗口，适合聊天记录上翻

## Lazy Loading

消息接口支持 `parts` 参数：

- `parts=none`
  - 只返回 message 元信息和 `part_count`
- `parts=summary`
  - 返回每个 part 的摘要信息，不带 `content`
- `parts=full`
  - 返回完整 part 内容，包含 `detail_json`

推荐前端策略：

1. 会话列表页只拉 `GET /api/v1/sessions`
2. 聊天页先拉 `GET /api/v1/sessions/{session_id}/messages?parts=summary`
3. 需要渲染富内容时，再对特定 message 或 part 发起增量请求

## Execution Model Resolution

执行类接口支持可选的运行参数：

- `model`
  - 结构为 `{ "provider_id": "...", "model_id": "..." }`
- `system`
- `temperature`
- `max_output_tokens`

当请求未显式传入 `model` 时：

- 若 session 历史消息里已有最近一次有效的 `provider/model` 元数据，则自动复用
- 若当前 runtime 只配置了一个 provider，则自动使用它的默认模型
- 若当前有多个 provider 且 session 又没有历史模型，则返回 `400`

## Concurrency

所有会修改 session 的接口都支持 `If-Match`：

- `PUT /api/v1/sessions/{session_id}`
- `DELETE /api/v1/sessions/{session_id}`
- `POST /api/v1/sessions/{session_id}/turns`
- `POST /api/v1/sessions/{session_id}/continue`
- `POST /api/v1/sessions/{session_id}/permission-replies`
- `POST /api/v1/sessions/{session_id}/user-input-replies`

请求头示例：

```http
If-Match: 7
```

或：

```http
If-Match: "7"
```

若版本不匹配，返回 `409 conflict`。

## Endpoints

### Operational probes (unprefixed)

- `GET /healthz` — liveness probe; returns `200 ok` without touching state. k8s-friendly.
- `GET /readyz` — readiness probe; returns `200 ready` once a runtime snapshot is loaded, otherwise `503 loading`.
- `GET /metrics` — Prometheus text format. Currently exposes `agena_runtime_generation`, `agena_runtime_reloads_total`, `agena_http_requests_total`, `agena_http_request_duration_seconds` histogram, provider/tool/session counters and gauges, plus `agena_build_info{version}`.

### Health / Provider

- `GET /api/v1/health`
- `GET /api/v1/runtime`
- `POST /api/v1/runtime/reload`
- `GET /api/v1/auth/providers`
- `POST /api/v1/auth/providers/openai/browser/start`
- `POST /api/v1/auth/providers/openai/browser/finish`
- `POST /api/v1/auth/providers/openai/device/start`
- `POST /api/v1/auth/providers/openai/device/poll`
- `POST /api/v1/auth/providers/gitlab/browser/start`
- `POST /api/v1/auth/providers/gitlab/browser/finish`
- `POST /api/v1/auth/providers/github-copilot/device/start`
- `POST /api/v1/auth/providers/github-copilot/device/poll`
- `GET /api/v1/auth/providers/{provider_id}`
- `PUT /api/v1/auth/providers/{provider_id}/api-key`
- `DELETE /api/v1/auth/providers/{provider_id}`
- `POST /api/v1/auth/providers/{provider_id}/refresh`
- `GET /api/v1/providers`
- `GET /api/v1/providers/{provider_id}/models`

### Workspace

- `GET /api/v1/workspaces`
  - query:
    - `cursor`
    - `limit`
    - `search`
    - `include_session_count=true|false`
- `POST /api/v1/workspaces`
- `POST /api/v1/workspaces/resolve`
  - body:
    - `path`
    - `create_if_missing=true|false`
  - behavior:
    - 先做 path normalization
    - 命中已有 workspace 时直接返回
    - 未命中且 `create_if_missing=true` 时自动创建
- `GET /api/v1/workspaces/{workspace_id}`
- `PUT /api/v1/workspaces/{workspace_id}`
- `DELETE /api/v1/workspaces/{workspace_id}`

### Auth Login Flow

- `POST /api/v1/auth/providers/openai/browser/start`
  - body:
    - `redirect_uri`
  - returns:
    - `authorize_url`
    - `state`
    - `pkce_verifier`
- `POST /api/v1/auth/providers/openai/browser/finish`
  - body:
    - `code`
    - `pkce_verifier`
    - `redirect_uri`
- `POST /api/v1/auth/providers/openai/device/start`
- `POST /api/v1/auth/providers/openai/device/poll`
  - body:
    - `device_code`
    - `user_code`
- `POST /api/v1/auth/providers/gitlab/browser/start`
  - body:
    - `instance_url`
    - `redirect_uri`
- `POST /api/v1/auth/providers/gitlab/browser/finish`
  - body:
    - `instance_url`
    - `code`
    - `pkce_verifier`
    - `redirect_uri`
- `POST /api/v1/auth/providers/github-copilot/device/start`
  - body:
    - `enterprise_domain`（可选）
- `POST /api/v1/auth/providers/github-copilot/device/poll`
  - body:
    - `device_code`
    - `enterprise_domain`（可选）

登录流返回约定：

- browser start 返回授权 URL 和 PKCE / state
- device start 返回 `verification_url`、`user_code`、`device_code`、`interval_seconds`
- finish / poll 返回：
  - `completed=true` 且附带 `provider`
  - 或 `completed=false` 表示仍需继续轮询

### Session

- `GET /api/v1/sessions`
  - query:
    - `cursor`
    - `limit`
    - `workspace_id`
    - `parent_id`
    - `roots=true|false`
    - `search`
- `POST /api/v1/sessions`
- `GET /api/v1/sessions/{session_id}`
- `PUT /api/v1/sessions/{session_id}`
- `DELETE /api/v1/sessions/{session_id}`
- `GET /api/v1/sessions/{session_id}/state`
- `GET /api/v1/sessions/{session_id}/events`
- `GET /api/v1/sessions/{session_id}/events/stream`
  - query:
    - `after_seq`
    - `limit`
    - `poll_interval_ms`
    - `idle_timeout_ms`
- `POST /api/v1/sessions/{session_id}/turns`
- `POST /api/v1/sessions/{session_id}/continue`
- `POST /api/v1/sessions/{session_id}/fork`
- `POST /api/v1/sessions/{session_id}/cancel`
- `POST /api/v1/sessions/{session_id}/permission-replies`
- `POST /api/v1/sessions/{session_id}/user-input-replies`
- `POST /api/v1/sessions/{session_id}/rewind`
- `POST /api/v1/sessions/{session_id}/unrewind`
- `GET /api/v1/sessions/{session_id}/export`
- `POST /api/v1/sessions/import`
- `GET /api/v1/sessions/tree/{root_id}`
- `GET /api/v1/sessions/{session_id}/rewind-checkpoints`

### Message

- `GET /api/v1/sessions/{session_id}/messages`
  - query:
    - `cursor`
    - `limit`
    - `parts=none|summary|full`
- `GET /api/v1/messages/{message_id}`
  - query:
    - `parts=none|summary|full`
- `GET /api/v1/messages/{message_id}/parts`
  - query:
    - `mode=none|summary|full`
- `GET /api/v1/message-parts/{part_id}`

### Permission Rule

- `GET /api/v1/permission-rules`
  - query:
    - `cursor`
    - `limit`
    - `search`
- `POST /api/v1/permission-rules`
- `GET /api/v1/permission-rules/{rule_id}`
- `PUT /api/v1/permission-rules/{rule_id}`
- `POST /api/v1/permission-rules/{rule_id}/revoke`
- `DELETE /api/v1/permission-rules/{rule_id}`

`permission-rules` 已升级为结构化资源，不再只围绕裸 `action_key` 工作。资源包含：

- `subject_kind`
  - `builtin_tool`
  - `path_access`
- `tool_name` / `qualifier`
- `path_access_kind` / `workspace_root` / `target_path`
- `mode`
  - `allow`
  - `ask`
  - `deny`
- `scope`
  - `session`
  - `workspace`
  - `global`
- `session_id` / `workspace_id`
- `source` / `reason` / `operator`
  - `source=permission_reply` 表示规则来自一次 remembered permission reply
  - `source=api` 表示规则由权限规则 API 显式创建或替换
  - `operator` 表示触发该变更的入口，例如 `http_api`
- `revoked_at` / `revoked_reason` / `revoked_by`

创建 / 更新请求示例：

```json
{
  "subject_kind": "builtin_tool",
  "tool_name": "bash",
  "qualifier": "git status*",
  "scope": "workspace",
  "mode": "allow"
}
```

路径规则示例：

```json
{
  "subject_kind": "path_access",
  "path_access_kind": "write",
  "target_path": "src/**",
  "scope": "session",
  "session_id": 42,
  "mode": "deny"
}
```

撤销请求示例：

```json
{
  "reason": "rule no longer needed"
}
```

## Notes

- `PUT` 走完整替换语义，适合生成式客户端和 typed SDK
- 删除接口会返回被删除前的资源快照，便于前端做乐观更新回滚
- `message part` 详情被拆成单独资源，避免列表接口把大块 JSON 全部捞出来
- 执行类接口返回的是轻量的 `session execution resource`，不会把整段 message history 再次塞回来
- `pending_permission_requests` / `pending_user_input_requests` 直接给出待回复对象，前端可按 `request_id` 回调后续接口
- `pending_permission_requests[].scope` 会暴露当前请求允许记忆的作用域（`session` / `workspace` / `global`）；`AllowAlways` / `DenyAlways` 会按 reply 的 scope 落成持久化规则
- `permission-rules/{id}/revoke` 会保留规则审计字段，而不是物理删除；前端应优先使用 revoke 关闭 remembered rule
- `events/stream` 默认从“当前最新事件之后”开始推送；如需补历史增量，请显式传 `after_seq`
- public HTTP API 不暴露 message 写接口；消息写入统一通过 `turns` 和后续 runtime reply 接口完成
- `sessions/{id}/state` 是前端刷新恢复入口，避免重新解析整段消息才能知道当前是否 blocked
- auth 管理接口只返回 credential 摘要，例如 `key_preview`、oauth expiry、account_id，不会回传明文 secret
- public auth API 会过滤内部实现用的凭据键，例如 `gitlab-instance`
