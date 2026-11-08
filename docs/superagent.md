# SuperAgent + Chatbot：持久任务与可选工作台
> **主要入口已升级为自然聊天。** 直接在原 Web/TUI 会话交办、追问、补充或叫停，助手在后台管理 Task 并主动回报。无需使用本页的手动任务步骤；完整使用说明见 [自然聊天持续工作](conversation-work.md)。本页主要保留底层接口和可选工作台文档。

本实现把长期任务放在现有 Agena Runtime 上方，不替换 Session Kernel、工具权限或 Subagent。当前是单实例、单操作者、串行任务版本；没有接通 QQ、飞书、GitHub，也不会主动向外发送消息。

## 已实现

- 独立于聊天会话的持久任务，以及独立 Root Worker Session。
- 普通聊天中的明确工作请求可由助手转交后台；闲聊仍直接回答，命令式入口仅作补充。
- 自然交办、查询、取消、续办和自动回报；仅明确要求审核的工作需要确认，也可在原聊天确认。
- 创建幂等键、聊天消息去重、版本冲突检查。
- 执行中断后保留任务身份与 Worker，不自动重放未知工具副作用。
- 复用原会话的工作区、显式提示片段、权限边界和能力禁用集合；不复制聊天历史。

代码入口：`crates/agena-superagent/` 是领域与服务契约；`crates/agena-runtime/src/runtime/superagent.rs` 是控制器；`crates/agena-storage-sqlite/src/superagent_repository.rs` 是持久化适配器。

## 使用边界

Web 与 TUI 现在都通过共享命令目录提供 `/tasks` 和 `/task [目标]`，并调用同一套类型化 Task API。原来的 `/status` 仍显示会话状态，`/activities` 仍显示后台活动；原 `/tasks` 的后台活动别名改为长期任务工作台，避免两种“任务”混淆。

原始 `/api/v1/chatbot/messages` 保持面向连接器的独立入口，其 `/status <UUID>` 和 `/cancel <UUID>` 语义不覆盖 Web/TUI 原有命令。

## 可选 Web 任务工作台

1. 在导航中选择“长期任务”，或点击聊天页标题旁的“长期任务”。也可以在聊天框输入 `/tasks`。
2. 点击“新建任务”，填写目标并确认。`/task 检查测试失败原因` 会预填目标并打开确认表单，不在你确认前启动执行。
3. 列表支持当前会话/所有任务、状态筛选和分页。选中任务可查看目标、执行次数、更新时间、提示和补充要求。
4. “打开执行会话”显示真实模型回复、工具结果与审批；在执行会话中再次点“长期任务”会回到原任务。
5. 等待输入时进入执行会话回答问题。结果进入“待验收”后，可以补充要求继续，也可以检查产物后确认完成。取消需要明确确认。

手机窄屏使用列表/详情切换；桌面使用左右面板。表单有焦点约束、键盘关闭和加载禁用状态。页面仅在可见时做有界刷新；失败退避，离开后停止读取。

未提交或结果不确定的 Web 表单在当前浏览器标签页保存短期草稿，以服务器来源和来源会话区分。刷新后恢复，但不会自动重发请求。文本仅用于该标签页恢复，不是服务端长期记忆或跨设备同步；主动关闭表单会清除草稿。

## 可选 TUI 任务工作台

输入 `/tasks` 打开任务工作台；命令面板选择 `/task` 或输入 `/task [目标]` 可以打开交办表单。命令使用同一份服务端目录，并有八种终端语言文案。

| 按键 | 操作 |
|---|---|
| `↑/↓`、`j/k` | 选择任务 |
| `Enter` | 打开执行会话 |
| `o` | 打开原始会话 |
| `n` | 新建任务 |
| `u` | 输入补充要求并继续 |
| `a` | 验收确认 |
| `c` | 取消确认 |
| `r` | 刷新 |
| `g`、`f` | 切换范围、循环状态筛选 |
| `[`、`]` | 上一页、下一页 |
| `PageUp/PageDown` | 滚动详情 |
| `Esc` | 返回，或关闭尚未提交的表单 |

编辑时 `Enter` 换行，`Ctrl+Enter` 或 `Alt+Enter` 提交；确认式操作也可用 `Enter`。中文、Unicode 和粘贴使用既有编辑器。TUI 关闭任务页再回来会保留该进程内的面板状态；不承诺终端进程退出后仍恢复未提交草稿。

任务页会按终端宽度切换横向/纵向布局，窄屏列表使用单行项目，避免项目高度超出可见区域而整行消失。

## 失败与冲突保护

Web 与 TUI 的创建/继续请求首次提交后固定请求键和正文。网络断开或回执丢失时，“重试”重发同一请求，避免重复开工。不能在不确定的回执上悄悄改成另一项工作。

取消、恢复和验收绑定已读取的任务版本。状态变化返回 409 后，界面保留输入、刷新状态，但不会自动替用户作新决定。Web 点击“使用最新版本重新确认”，TUI 使用 `Ctrl+R`，核对后再次提交。

读取列表、切换任务、切换会话都有请求代次检查，旧响应不能覆盖新的选择；已观察到的较新任务版本不能被迟到的旧列表回退。

API 继承 Agena 实例现有访问控制。对外开放时必须启用实例认证；它不是接收任意 QQ/飞书用户消息的公共 Webhook。外部适配器仍需独立实现验签、身份映射、会话归属和授权，不能把显示名或消息内容当作权限。

普通消息仍走原有会话执行流程，其工具行为继续受原权限系统约束。“不会自动建任务”不等于“普通会话禁止所有工具”。人格、工作区长期记忆、提醒与有限次数的主动跟进现已接入默认 `agena.companion`，普通聊天与 Web/TUI 可选管理面板共用同一服务，详见 [助手使用说明](assistant.md)。

## 任务接口

| 方法 | 路径 | 用途 |
|---|---|---|
| POST | `/api/v1/tasks` | 显式创建任务 |
| GET | `/api/v1/tasks` | 查询任务；支持 source_session_id、workspace_id、state、limit、offset |
| GET | `/api/v1/tasks/{id}` | 当前任务、Worker 与执行回执 |
| POST | `/api/v1/tasks/{id}/cancel` | 取消排队或运行中的任务 |
| POST | `/api/v1/tasks/{id}/resume` | 用新指令显式继续原任务 |
| POST | `/api/v1/tasks/{id}/accept` | 人工验收当前结果 |

创建请求：

```json
{
  "source_session_id": 123,
  "request_key": "client-request-001",
  "goal": "检查当前项目的测试失败原因，给出验证结果"
}
```

`source_session_id` 必须是已存在的普通会话，不能是 Subagent。Task 请求不会授予比原会话更宽的工具权限。`request_key` 在重试时保持不变；同一来源会话复用键却改变目标会返回 409。

返回记录包含 `id`、`state`、`revision`、`worker_session_id`、`execution_id`、`attempt_generation`。Worker 的正文、工具结果、权限审批和提问继续通过现有 Session API / 界面查看和处理。

手动 Task API 创建的任务仍在执行成功后进入 `review`；自然聊天交办的工作默认自动完成并回报。显式手动验收请求：

```json
{"expected_revision": 5}
```

中断、失败、取消或待验收任务可显式恢复：

```json
{
  "expected_revision": 5,
  "request_key": "resume-request-001",
  "instruction": "先核对之前是否已经生成结果，确认后补充缺少的测试"
}
```

恢复要求 Worker 没有仍在执行或排队的旧请求；会复用同一 Worker，但建立新的任务 attempt 和执行输入。重复提交相同恢复请求不会创建更多 attempt。不要修改 `expected_revision` 后盲目重试冲突；先读取当前状态。

## 聊天接入

```http
POST /api/v1/chatbot/messages
Content-Type: application/json
Authorization: Bearer <existing Agena operator token>
```

```json
{
  "session_id": 123,
  "message_id": "chat-event-001",
  "text": "/task 检查当前项目的测试失败原因"
}
```

支持：

- `/task <目标>`：创建长期任务。
- `/tasks`：列出当前来源会话的最近 20 个任务；完整分页使用 Task API。
- `/status <任务 UUID>`：查询当前会话创建的任务。
- `/cancel <任务 UUID>`：取消当前会话创建的任务。
- 其他文本：投递给原有聊天会话，返回执行回执。

引用中的 `/task` 不会被解析为交办；没有通过模型猜测消息意图。聊天命令不能查看或取消其他来源会话的任务。原生 Task API 是实例操作者接口，保持现有单操作者管理范围，不声称提供团队 RBAC。

每条新消息使用新 `message_id`，重试同一条消息保留原 ID。事件内容会先持久化，之后才能产生效果；相同 ID 更换文本会返回 409。已完成请求的重放返回原始回执，因此查询最新进度应发送新的 `/status` 事件或 GET Task。

普通聊天返回 `kind: conversation` 和执行身份；答案仍通过现有会话读取/流接口获取。任务命令返回 `kind: task` 或 `kind: tasks`。本版不包含外部渠道的结果推送。

## 持久化与恢复

任务和聊天回执使用现有实例 SQLite；不另建数据库，也不使用内存后台任务表作为权威。

- 数据库所有者统一声明并安装表。只接受已识别的完整旧基线进行追加迁移；不完整或外来的 Schema 被拒绝。
- Worker 创建使用由 Task ID 派生的稳定命令身份。
- 模型输入使用 Task ID + attempt 的稳定投递键，保护提交成功但回执丢失的窗口。
- 控制器通过持久执行身份核对状态，不依靠通知总线不丢消息。
- 原执行在重启后为 interrupted 时，Task 也进入 interrupted；需要明确恢复，而不是从头重跑。
- 原有普通会话排队恢复语义保持不变。

升级真实实例前应备份数据。追加 Schema 后，旧二进制可能拒绝打开新库，不承诺直接降级。本轮开发测试仅使用临时数据库，不应把测试记录理解为真实服务已经升级。

## 当前限制

仅串行运行一个顶层 Task；等待权限/输入时也保留这一保守并发限制。普通聊天会话不受这个 Task 队列限制。Worker 仍可使用原有 Subagent 能力，其既有角色限制保持不变。

尚未提供专属 Git worktree、跨平台强沙箱、任务级成本/时间预算、语义 Watchdog、外部副作用 Outbox、外部连接器、团队身份、外部渠道推送或回执保留清理策略。共享工作目录不构成安全隔离。任务中断后仍需核对外部副作用，不能宣称任意工具操作严格只执行一次。

## 验证

```sh
cargo test -p agena-superagent --lib --offline --locked
cargo test -p agena-storage-sqlite --test superagent_tasks --offline --locked
cargo test -p agena --test system scenarios::superagent --offline --locked -- --test-threads=1
cargo fmt --all -- --check
```

系统测试启动临时实例、脚本化的本地模型服务和独立工作区，覆盖明确交办、普通聊天、幂等、会话范围、验收、取消、真实进程崩溃与显式恢复。它们不使用真实外部模型或用户的日常会话数据库。

## 第一版后端验证记录（2026-11-07）

- 新增领域测试：3 通过。
- 新增 SQLite 存储、聊天回执与 Schema 迁移测试：6 通过。
- 新增真实 HTTP / 独立产品进程测试：6 通过；最终补丁已复跑。
- 既有 `agena-storage-sqlite`、`agena-runtime-session` 的 library / integration 回归通过；普通聊天与消息队列系统场景回归通过。
- 相关 crate 的 Clippy（`-D warnings`）、`cargo fmt --all -- --check` 和 `git diff --check` 通过。
- 新存储测试已配置到 CI；领域测试与系统场景进入既有对应入口。未声称 GitHub Actions 已实际运行。

没有重启用户现有服务，没有迁移真实会话数据库，没有提交或推送 Git。当前结果是本地工作区中的后端与 Web/TUI 实现，不是已上线的外部聊天平台机器人。

## 原生客户端代码与验收入口

- `crates/agena-api/src/tasks.rs`：共享协议；Web 类型由 Rust Schema 生成，不手抄第二份任务状态。
- `crates/agena-client/src/http/tasks.rs`：TUI 使用的认证 HTTP 客户端。
- `crates/agena-tui-app/src/app_tasks.rs`：终端任务状态、异步消息归属、表单和渲染。
- `packages/agena-web/src/features/tasks/`：Web 控制器、草稿和实际 Vue 组件。
- `packages/agena-web/src/pages/TasksPage.vue`：可导航、可链接的任务页。
- `apps/agena/tests/scenarios/superagent_ui.rs`：真实产品进程上的浏览器/终端验收。

```sh
# 状态与协议回归
cargo test -p agena-tui-app app_tasks --lib --offline --locked
cargo test -p agena --test system scenarios::superagent --offline --locked -- --test-threads=1
cd packages/agena-web
bun test ./tests
bun run build
cd ../..

# 需要本地 Chrome、tmux 和已安装的 Web 依赖；使用临时实例与本地模型
cargo test -p agena --test system scenarios::superagent_ui --offline --locked \
  -- --ignored --test-threads=1 --nocapture
# 浏览器驱动使用 Node；CI 按仓库 .node-version 安装，不用 Bun 运行 Vite 探针。
```

浏览器验收真正加载 Vue 应用并调用真实 Agena 后端，覆盖接受请求后丢失回执、重复点击保护、执行会话跳转、原 Worker 反馈续办、验收、375px 手机布局、弹窗焦点和刷新恢复草稿。终端验收通过私有 tmux socket 驱动真实 TUI，覆盖命令目录、中文输入、继续、执行会话往返、验收与窗口缩放；不会操作用户的全局 tmux。

这些 UI 验收默认标记为需要额外系统依赖；CI 显式安装 Web 依赖和 tmux，要求 Chrome 可用，并运行 `--ignored`，不是仅声明用例。CI 运行结果以实际流水线为准，本次只验证本地相同测试入口。

## Web/TUI 本轮验收结果（2026-11-07）

| 验证 | 实际结果 |
|---|---|
| Web 全量 Bun 行为测试 | 311 通过、0 失败（含 8 项任务竞态/重试测试） |
| Web 类型、模块边界、图标和语言目录 | 通过；7 种 Web 语言目录完整 |
| Web 生产构建 | 通过 |
| TUI App library 回归 | 175 通过、0 失败；最终任务专项 5 项再跑通过 |
| HTTP 客户端 / 内置插件 library 回归 | 26 / 32 项通过 |
| 持久任务 HTTP 系统回归 | 7 通过，包括新增的过期取消保护 |
| 真实 Chrome + Vue + Agena | 通过：真实执行结果、回执丢失重试、续办、验收、375px 手机布局、焦点和草稿恢复 |
| 真实 TUI + 私有 tmux + Agena | 通过：中文输入、续办、Worker 往返、验收、42×18 缩放 |
| 相关 crate Clippy / Rust 格式 / diff 检查 | 通过；Clippy 使用 `-D warnings` |

真实终端测试发现窄屏列表的多行项目会因可见高度不足而消失，已经改为窄屏单行列表并新增内容断言，不只是断言程序没有崩溃。浏览器测试主动丢弃服务端已经接受的创建回执，验证重试仍只有一个任务和一次初始模型请求。

浏览器驱动必须使用 Node。尝试使用 Bun 直接运行 Vite/CDP 探针时，应用页面未就绪；CI 已明确安装仓库指定 Node 并用已通过的 Node 路径执行。Bun 仍用于 Web 依赖、行为测试和生产构建。

最终真实 UI 验收输出保留在本机 `/tmp/agena-tasks-ui-node-final.log`；浏览器截图与终端文本的具体目录在该日志中。临时验收只使用脚本化本地模型和独立数据目录，没有调用真实付费模型；这些结果不能代替长期负载、第三方渠道、多用户或多平台终端的测试。

本轮未提交或推送 Git，也未重启现有用户服务、替换用户的运行实例或迁移真实会话数据库。要使用新入口，需要运行包含本工作区改动的新构建。

## 后续：自然聊天已成为主要入口

任务工作台的手动流程现保留为辅助入口。`agena.work` 默认插件已接通普通 Web/TUI 对话、同 Worker 续办、原会话提问/权限卡片和可靠主动回报。普通工作不再默认要求手动验收；明确要求验收时也能在聊天中确认。最新行为、验证结果与边界见 `docs/conversation-work.md`，不应把本页前述历史“手动任务版本”的限制当作当前自然对话入口的行为。

## 人格、记忆与主动交互已接通

当前完整助手链路由 `agena.companion`、`agena.work` 和现有 Runtime 组合实现。普通聊天可以修改人格、保存/纠正/删除工作区记忆、安排受免打扰与频率限制的提醒和有限周期跟进，并持续处理后台工作。Web 与 TUI 都有可选 `/assistant` 管理入口。最新行为与验证记录统一见 `docs/assistant.md`；QQ/飞书不在本轮范围内。
