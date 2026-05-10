# Agena Web / API 对齐审计

更新时间：2026-05-11

本文档记录 `packages/agena-studio-web` 与当前 `apps/agena-studio-server` / `crates/agena-api-server` 的对齐结果。本文已经从“迁移问题清单”更新为“完成后状态审计”，用于说明哪些问题已被修复，以及还剩哪些仅属设计取舍而非功能缺口的事项。

## 宿主兼容层结论

- `GET /health` 不是错误旧接口。
  - `apps/agena-studio-server/src/app.rs` 暴露 Studio 宿主层健康检查。
  - `GET /api/v1/health` 是 `agena-api-server` 的 v1 API 健康接口。
  - 结论：Web 调用 `/health` 属于宿主层设计，不是迁移残留 bug。

## 已完成对齐

### Runtime / Providers / Plugins

- Runtime 概览与 reload 已接入：
  - `GET /api/v1/runtime`
  - `POST /api/v1/runtime/reload`
- Providers / provider models 已接入：
  - `GET /api/v1/providers`
  - `GET /api/v1/providers/{provider_id}/models`
- Plugins / marketplace 已接入：
  - 插件列表、详情、日志
  - marketplace 搜索 / sync / install / uninstall / upgrade

### Auth provider 管理与登录流

- Auth provider 基础管理已接入：
  - `GET /api/v1/auth/providers`
  - `PUT /api/v1/auth/providers/{provider_id}/api-key`
  - `DELETE /api/v1/auth/providers/{provider_id}`
  - `POST /api/v1/auth/providers/{provider_id}/refresh`
- 迁移前缺失的 browser / device login 流现已补齐，包含后端 REST 暴露层与 Web UI：
  - `POST /api/v1/auth/providers/openai/browser/start`
  - `POST /api/v1/auth/providers/openai/browser/finish`
  - `POST /api/v1/auth/providers/openai/device/start`
  - `POST /api/v1/auth/providers/openai/device/poll`
  - `POST /api/v1/auth/providers/gitlab/browser/start`
  - `POST /api/v1/auth/providers/gitlab/browser/finish`
  - `POST /api/v1/auth/providers/github-copilot/device/start`
  - `POST /api/v1/auth/providers/github-copilot/device/poll`
- Web 侧已支持：
  - OpenAI browser login + device login
  - GitLab browser login
  - GitHub Copilot device login
  - `/auth/callback` 回调页与 `postMessage` 自动回填完成 browser login

### Workspace / Session 管理

- Workspace 管理已补齐：
  - `POST /api/v1/workspaces`
  - `POST /api/v1/workspaces/resolve`
  - `PUT /api/v1/workspaces/{workspace_id}`
  - `DELETE /api/v1/workspaces/{workspace_id}`
- Session 管理已补齐：
  - `GET /api/v1/sessions/{session_id}`
  - `PUT /api/v1/sessions/{session_id}`
  - `DELETE /api/v1/sessions/{session_id}`
  - `POST /api/v1/sessions/{session_id}/fork`
  - `GET /api/v1/sessions/{session_id}/export`
  - `POST /api/v1/sessions/import`
- Web 侧当前可完成：
  - rename / delete workspace
  - rename / delete session
  - fork / export / import session

### Chat 运行控制

- 已接入并暴露：
  - `GET /api/v1/sessions/{session_id}/state`
  - `GET /api/v1/sessions/{session_id}/events`
  - `GET /api/v1/sessions/{session_id}/events/stream`
  - `POST /api/v1/sessions/{session_id}/turns`
  - `POST /api/v1/sessions/{session_id}/continue`
  - `POST /api/v1/sessions/{session_id}/cancel`
  - `POST /api/v1/sessions/{session_id}/permission-replies`
  - `POST /api/v1/sessions/{session_id}/user-input-replies`
  - `POST /api/v1/sessions/{session_id}/rewind`
  - `POST /api/v1/sessions/{session_id}/unrewind`
  - `GET /api/v1/sessions/tree/{root_id}`
  - `GET /api/v1/sessions/{session_id}/rewind-checkpoints`
- Web 侧当前可完成：
  - submit / continue turn
  - SSE 增量更新
  - approve / deny permission requests
  - submit / cancel user input requests
  - rewind
  - cancel 正在运行的 session
  - undo rewind

### Permission rule 生命周期

- 已接入并暴露：
  - `GET /api/v1/permission-rules`
  - `POST /api/v1/permission-rules`
  - `GET /api/v1/permission-rules/{rule_id}`
  - `PUT /api/v1/permission-rules/{rule_id}`
  - `POST /api/v1/permission-rules/{rule_id}/revoke`
  - `DELETE /api/v1/permission-rules/{rule_id}`
- Web 侧已支持 create / update / revoke / permanent delete。

### Message / Event 高级调试入口

- 迁移初期缺失的高级 inspection 入口已补齐：
  - `GET /api/v1/messages/{message_id}`
  - `GET /api/v1/messages/{message_id}/parts`
  - `GET /api/v1/message-parts/{part_id}`
  - `GET /api/v1/events`
- Web 侧当前已支持：
  - Chat 页按需打开 message inspector
  - 查看 message parts 列表
  - 按 part 单独拉取 detail payload
  - Runtime workflow 页查看 recent global event history

### 类型与查询参数修正

- `SessionResource` 前端类型已补齐后端返回字段：
  - `depth`
  - `root_id`
  - `is_subagent`
- `listWorkspaces()` 已改为请求 `include_session_count=true`，避免 workspace 列表的 `session_count` 长期为空。

## 残余说明

以下项目不再视为“当前必须修”的 web/api 不对齐问题：

- `GET /api/v1/workspaces/{workspace_id}`
  - REST 已暴露，但当前 Web 不需要单独 workspace detail 读取，因为页面状态由列表 + 文件树 + git 状态组合完成，未出现功能缺口。
- `GET /api/v1/permission-rules/{rule_id}`
  - REST 已暴露，但 Web 当前权限设置页通过列表资源完成编辑与删除，不依赖单条 detail 读取，属于冗余读取而不是缺功能。
- 部分 session / message 单资源读取接口虽然现在已补 helper 或 UI 入口，但页面主流程仍优先使用批量资源和投影状态。
  - 这是合理实现选择，不再属于迁移遗留问题。

## 本轮完成结论

本轮对齐工作已经完成最初审计里识别出的主要迁移缺口，包括：

1. `workspace/session` 管理动作补齐
2. `session cancel/unrewind` 补齐
3. `permission rule delete` 补齐
4. `listWorkspaces()` 的 `include_session_count` 修正
5. `SessionResource` 前端类型补齐
6. auth provider browser / device login 的后端 REST 与 Web UI 补齐
7. `message detail / part detail / global events` 的高级调试入口补齐

结论：当前 `agena-studio-web` 与 `agena-api-server` 的核心 REST 能力已经完成对齐，不再存在明显的“后端已有但 Web 完全缺失”的关键迁移功能缺口。
