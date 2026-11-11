# API v1 路由终态设计

本文是 `agena-api-server` 路由重构的目标形态。它记录**为什么**这样划分，以及每条路由的最终形态。

## 三条定位原则

1. **文件夹用绝对路径定位，会话用 id**
   文件夹的身份就是它的路径；会话有独立生命周期（可重命名、可分叉、可归档），所以保留 id。URL 里不再出现 `workspace_id`。

2. **HTTP 与工具用同一套语义**
   `agena.fs` 的十个工具（`stat/read/read_many/read_media/document/glob/grep/write/apply_patch/replace`）已经有完整定义。`/fs` 命名空间与它们一一对应，前端的文件浏览和模型的文件操作是同一件事的两种入口。

3. **资源用复数名词，动作用 HTTP 方法**
   不再出现 `POST /x/cancel`、`POST /x/install` 这类动词路径。状态转换统一为 `PUT .../state` 带 body。

## 路径参数约定

| 参数 | 含义 |
|---|---|
| `path` | **绝对路径**。服务端拒绝相对路径，避免依赖进程工作目录 |
| `relative` | 相对于 `path` 的子路径，用于目录内定位 |
| `pattern` | glob/grep 模式 |
| `{id}` | 会话、机器人、插件等有独立生命周期的资源 |

`/fs` 与 `/git` 不做额外的路径权限检查：本地操作者持有实例令牌，本就拥有全部权限。会话级的权限边界服务于渠道成员，不适用于这些路由。

---

## 系统

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/healthz` | 存活探针 |
| GET | `/readyz` | 就绪探针 |
| GET | `/metrics` | Prometheus |
| GET | `/api/v1/health` | 结构化健康信息 |
| GET | `/api/v1/runtime` | 运行时状态 |
| POST | `/api/v1/runtime/reloads` | 原 `runtime/reload` |
| GET | `/api/v1/usage` | 用量统计 |
| GET | `/api/v1/settings/resolved` | 原 `config/resolved` |

## 流

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/streams/global` | 原 `changes/stream?scope_kind=global` |
| GET | `/api/v1/streams/sessions/{id}` | 原 `sessions/{id}/changes/stream` |
| GET | `/api/v1/streams/notifications` | 原 `notifications/stream` |
| GET | `/api/v1/streams/revisions` | 原 `changes/revisions` |
| GET | `/api/v1/ws` | WebSocket 升级 |

---

## 文件系统

与 `agena.fs` 工具一一对应。

| 方法 | 路径 | 对应工具 |
|---|---|---|
| GET | `/api/v1/fs?path=` | 列一层目录 |
| GET | `/api/v1/fs/stat?path=` | `stat` |
| GET | `/api/v1/fs/content?path=&max_bytes=` | `read` |
| PUT | `/api/v1/fs/content?path=` | `write`（可选 `move_to=` 表示移动） |
| DELETE | `/api/v1/fs?path=` | 删除 |
| POST | `/api/v1/fs/read-many` | `read_many` |
| GET | `/api/v1/fs/media?path=` | `read_media` |
| GET | `/api/v1/fs/document?path=` | `document` |
| POST | `/api/v1/fs/patch` | `apply_patch` |
| POST | `/api/v1/fs/replacements` | `replace` |
| POST | `/api/v1/fs/directories?path=` | 创建目录 |
| GET | `/api/v1/fs/globs?pattern=&path=` | `glob` |
| GET | `/api/v1/fs/greps?pattern=&path=` | `grep` |
| POST | `/api/v1/fs/uploads?path=` | 上传二进制 |
| GET | `/api/v1/fs/downloads?path=` | 下载文件 |
| GET | `/api/v1/fs/archives?path=` | 整目录打包下载 |

`GET /fs/content` 与 `PUT /fs/content` 是同一资源（路径 × 内容）的读写两端，因此共用路径名。

## 文件夹登记（收藏与最近）

文件夹登记**不是权限边界**。任何路径都可直接通过 `/fs` 访问；登记只用于"最近使用"和收藏列表。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/folders` | 列表，含会话计数 |
| GET | `/api/v1/folders?path=` | 按路径查单条 |
| POST | `/api/v1/folders` | 登记路径 |
| PUT | `/api/v1/folders?path=` | 更新（`new_path=` 表示移动登记） |
| DELETE | `/api/v1/folders?path=` | 取消登记（不动磁盘） |

## Git

git 属于目录，不属于会话，也不要求目录已被登记。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/git?path=` | 仓库状态（原 `git/status`，修正了读进程 cwd 的缺陷） |
| POST | `/api/v1/git?path=` | 初始化仓库（原 `project/git/init`） |
| GET | `/api/v1/git/diff?path=` | 原始 diff（原 `vcs/diff/raw`） |
| POST | `/api/v1/git/stages?path=` | 暂存 |
| POST | `/api/v1/git/commits?path=` | 提交 |
| POST | `/api/v1/git/pull-requests?path=` | 创建 PR |

---

## 会话

| 方法 | 路径 |
|---|---|
| GET, POST | `/api/v1/sessions` |
| GET | `/api/v1/sessions/overview` |
| GET, PUT, DELETE | `/api/v1/sessions/{id}` |
| GET | `/api/v1/sessions/{id}/state` |
| GET | `/api/v1/sessions/{id}/parts` |
| GET | `/api/v1/sessions/{id}/parts/{part_id}/sections/{section}` |
| GET | `/api/v1/sessions/{id}/runs` |
| GET | `/api/v1/sessions/{id}/plan` |
| GET | `/api/v1/sessions/{id}/cost` |
| GET | `/api/v1/sessions/{id}/tree` |
| GET | `/api/v1/sessions/{id}/export` |
| POST | `/api/v1/sessions/imports` |
| GET | `/api/v1/sessions/{id}/prompts/preview` |
| GET | `/api/v1/sessions/{id}/edits` |
| GET | `/api/v1/sessions/{id}/folder` |

**会话列表与详情直接返回 `folder_path`**，调用方无需再解析文件夹 id。`GET .../edits` 是原 `file-changes`：它读的是 Agena 记录的"本会话编辑过哪些文件"，与 git 状态无关，改名以澄清。

**内容**

| 方法 | 路径 |
|---|---|
| GET | `/api/v1/sessions/{id}/content/{resource_id}` |
| GET | `/api/v1/sessions/{id}/content/{resource_id}/text` |
| GET | `/api/v1/sessions/{id}/content/{resource_id}/stream` |
| GET | `/api/v1/sessions/{id}/inline-ui/{reference}` |
| GET | `/api/v1/sessions/{id}/inline-ui/{reference}/live` |
| GET | `/api/v1/sessions/{id}/inline-ui/{reference}/stream` |

**执行**

| 方法 | 路径 | 原 |
|---|---|---|
| POST | `/api/v1/sessions/{id}/executions` | `messages` |
| GET, DELETE | `/api/v1/sessions/{id}/executions/current` | `cancel` |
| GET, DELETE | `/api/v1/sessions/{id}/executions/{eid}/attempts/{gen}` | 不变 |
| POST | `/api/v1/sessions/{id}/executions/{eid}/continuations` | `continue` |
| POST | `/api/v1/sessions/{id}/executions/{eid}/forks` | `fork` |
| POST | `/api/v1/sessions/{id}/compactions` | `compact` |
| POST | `/api/v1/sessions/{id}/rewinds` | `rewind` |
| POST | `/api/v1/sessions/{id}/btws` | `btw` |

**队列**

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/sessions/{id}/queue` | 新增 |
| PUT | `/api/v1/sessions/{id}/queue/state` | `queue/pause` |
| GET, PUT, DELETE | `/api/v1/sessions/{id}/queue/{execution_id}` | 三个方法合一 |
| POST | `/api/v1/sessions/{id}/queue/{execution_id}/promotes` | `queue/{id}/take` |

**交互**

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/sessions/{id}/interactions` | 新增 |
| POST | `/api/v1/sessions/{id}/interactions/{request_id}/responses` | `permission-replies` + `user-input-replies` |
| POST | `/api/v1/sessions/{id}/interactions/{request_id}/presentations` | `present` |

**附属**

| 方法 | 路径 |
|---|---|
| GET, PUT | `/api/v1/sessions/{id}/assistant` |
| POST | `/api/v1/sessions/{id}/assistant/reminders/{rid}` |
| POST | `/api/v1/sessions/{id}/assistant/memory` |
| PUT | `/api/v1/sessions/{id}/selection` |
| PUT | `/api/v1/sessions/{id}/permission` |
| GET, POST | `/api/v1/sessions/{id}/attachments` |
| GET | `/api/v1/sessions/{id}/media` |

**集成**

| 方法 | 路径 |
|---|---|
| GET, POST | `/api/v1/sessions/{id}/integrations` |
| GET | `/api/v1/sessions/{id}/integrations/{binding_id}` |
| PUT | `/api/v1/sessions/{id}/integrations/{binding_id}/state` |
| PUT | `/api/v1/sessions/{id}/integrations/{binding_id}/access` |
| GET | `/api/v1/sessions/{id}/platforms` |

---

## 机器人

| 方法 | 路径 | 原 |
|---|---|---|
| GET, POST | `/api/v1/bots` | |
| GET, PUT | `/api/v1/bots/{id}` | |
| DELETE | `/api/v1/bots/{id}` | `archive` |
| POST | `/api/v1/bots/{id}/sessions` | `{id}/session` |
| POST | `/api/v1/bots/default/sessions` | `bot/session` |

---

## 提供方

合并原 `providers`、`provider-studio`、`auth/providers` 三个前缀。

| 方法 | 路径 |
|---|---|
| GET, POST | `/api/v1/providers` |
| GET, DELETE | `/api/v1/providers/{id}` |
| GET | `/api/v1/providers/{id}/models` |
| GET | `/api/v1/providers/{id}/configured-models` |
| GET | `/api/v1/providers/{id}/configured-local-models` |

**草稿**（编辑中的状态，保存前不落盘）

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/providers/draft` | `provider-studio/draft`（新建） |
| GET, PUT | `/api/v1/providers/{id}/draft` | `GET draft?provider_id=` / `save` |
| POST | `/api/v1/providers/{id}/draft/adapters` | `save-adapter` |
| POST | `/api/v1/providers/{id}/draft/adapters/{aid}/models` | `save-model` |
| DELETE | `/api/v1/providers/{id}/draft/adapters/{aid}` | `delete-adapter` |
| DELETE | `/api/v1/providers/{id}/draft/adapters/{aid}/models/{mid}` | `delete-model` |
| POST | `/api/v1/providers/{id}/draft/models/lookups` | `draft/models` |
| POST | `/api/v1/providers/{id}/draft/models/preview` | `draft/model` |
| POST | `/api/v1/providers/{id}/draft/models/template` | `draft/model/catalog-template` |

**认证**

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/providers/{id}/auth` | `auth/providers/{id}` |
| PUT, DELETE | `/api/v1/providers/{id}/auth/credential` | `api-key` |
| POST | `/api/v1/providers/{id}/auth/refreshes` | `auth/providers/{id}/refresh` |
| POST | `/api/v1/providers/{id}/auth/device-sessions` | `{vendor}/device/start` |
| GET | `/api/v1/providers/{id}/auth/device-sessions/{sid}` | `{vendor}/device/poll` |
| POST | `/api/v1/providers/{id}/auth/browser-sessions` | `{vendor}/browser/start` |
| PUT | `/api/v1/providers/{id}/auth/browser-sessions/{sid}` | `{vendor}/browser/finish` |
| POST | `/api/v1/providers/{id}/draft/auth-sessions` | `provider-studio/auth/start` |
| PUT | `/api/v1/providers/{id}/draft/auth-sessions/{sid}` | `provider-studio/auth/continue` |

厂商名（openai / gitlab / github-copilot）从路径中移除，改为 `{id}`；新增认证方式不需要新增路由。

**目录**

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/providers/catalog` | `model-catalog` |
| POST | `/api/v1/providers/catalog/lookups` | `model-catalog/lookup` |
| POST | `/api/v1/providers/catalog/matches` | `model-catalog/match` |
| POST | `/api/v1/providers/catalog/refreshes` | `model-catalog/refresh` |
| POST | `/api/v1/providers/models/resolutions` | `providers/models` |
| GET | `/api/v1/providers/aws-profiles` | 不变 |
| POST | `/api/v1/providers/client-versions/refreshes` | 不变 |

---

## 工具

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/tools` | `operator/tools` |
| POST | `/api/v1/tools/invocations` | `operator/tools/invoke` + `plugins/tools/invoke` |
| GET | `/api/v1/tools/changes` | `plugins/tools/changes` |

## 插件

| 方法 | 路径 |
|---|---|
| GET | `/api/v1/plugins` |
| GET | `/api/v1/plugins/surface` |
| GET | `/api/v1/plugins/architecture` |
| GET | `/api/v1/plugins/{id}` |
| GET, PUT | `/api/v1/plugins/{id}/settings` |
| POST | `/api/v1/plugins/{id}/commands/{cid}/invocations` |
| GET | `/api/v1/plugins/{id}/logs` |
| POST | `/api/v1/plugins/marketplace/search` |
| POST | `/api/v1/plugins/marketplace/syncs` |
| GET | `/api/v1/plugins/marketplace/installed` |
| GET | `/api/v1/plugins/marketplace/outdated` |
| PUT, DELETE | `/api/v1/plugins/marketplace/installations/{id}` |
| POST | `/api/v1/plugins/marketplace/installations/{id}/upgrades` |
| POST | `/plugin-rpc/{plugin_id}` |

## MCP 连接

| 方法 | 路径 |
|---|---|
| GET, POST | `/api/v1/mcp-connections` |
| POST | `/api/v1/mcp-connections/imports` |
| POST | `/api/v1/mcp-connections/imports/preview` |
| PUT | `/api/v1/mcp-connections/bridge` |
| GET, PUT, DELETE | `/api/v1/mcp-connections/{id}` |
| PUT | `/api/v1/mcp-connections/{id}/state` |
| POST | `/api/v1/mcp-connections/{id}/reconnects` |
| POST | `/api/v1/mcp-connections/{id}/tests` |
| GET | `/api/v1/mcp-connections/{id}/capabilities` |
| POST | `/api/v1/mcp-connections/{id}/resources` |
| POST | `/api/v1/mcp-connections/{id}/prompts` |
| PUT, DELETE | `/api/v1/mcp-connections/{id}/credential` |
| POST | `/api/v1/mcp-connections/{id}/auth-sessions` |
| PUT, DELETE | `/api/v1/mcp-connections/{id}/auth-sessions/current` |

---

## 技能、记忆、权限规则

| 方法 | 路径 |
|---|---|
| GET, POST | `/api/v1/skills` |
| POST | `/api/v1/skills/validations` |
| POST | `/api/v1/skills/installations` |
| POST | `/api/v1/skills/installations/preview` |
| GET, PUT, DELETE | `/api/v1/skills/{id}` |
| PUT | `/api/v1/skills/{id}/enabled` |
| GET, PUT, DELETE | `/api/v1/skills/{id}/resource` |
| GET, POST | `/api/v1/memories` |
| GET | `/api/v1/memories/overview` |
| POST | `/api/v1/memories/index` |
| GET, PUT, DELETE | `/api/v1/memories/{name}` |
| GET, POST | `/api/v1/permission-rules` |
| POST | `/api/v1/permission-rules/batch` |
| GET, PUT, DELETE | `/api/v1/permission-rules/{id}` |
| DELETE | `/api/v1/permission-rules/{id}/grants` |

## 活动与通知

| 方法 | 路径 | 原 |
|---|---|---|
| GET | `/api/v1/activities` | |
| PUT | `/api/v1/activities/{id}/state` | `stop` / `pause` / `resume` / `dismiss` |
| DELETE | `/api/v1/activities/{id}` | `delete` |
| GET | `/api/v1/activities/{id}/logs` | |
| GET | `/api/v1/notifications` | |
| DELETE | `/api/v1/notifications/{id}` | `dismiss` |
| POST | `/api/v1/notifications/{id}/actions/{action_id}` | |

`PUT .../state` 的 body：

```json
{ "state": "paused" }
```

取值：`running`、`paused`、`stopped`、`dismissed`。

## 设置

| 方法 | 路径 | 原 |
|---|---|---|
| GET, PUT, PATCH, DELETE | `/api/v1/settings` | |
| GET, PUT, DELETE | `/api/v1/settings/{layer}` | `settings/layers/{layer}` |
| GET | `/api/v1/settings/entries` | |
| GET | `/api/v1/settings/sources` | |
| POST | `/api/v1/settings/validations` | `settings/validate` |

---

## 规模对比

| 指标 | 重构前 | 重构后 |
|---|---|---|
| 路由条目 | 177 | 约 140 |
| 顶层命名空间 | 27 | 13 |
| 含动词的路径 | 31 | 0 |
| 路由表所在文件 | 1 个（737 行） | 按域拆分 |

消失的前缀：`workspaces`、`provider-studio`、`auth`、`git`（顶层）、`vcs`、`project`、`changes`、`operator`、`bot`（单数）、`config`、`model-catalog`、`notifications/stream`。

## 迁移顺序

1. **拆文件**：`routes/*.rs` 按域组织，行为零变化
2. **`/fs` 与 `path` 参数**：新增命名空间，git 修正为按路径查询
3. **合并 `provider-studio` 与 `auth`**
4. **统一动词**：activities、notifications、插件市场
5. **`changes` → `streams`** 等纯改名

前两步不动既有路径，可独立发布；第 3 步起是破坏性变更，需要前端同步。
