# 共享 Query / Command API 开发指南

Agena 的业务操作统一声明在 `agena-session-api` 的 `Query`、`Command` 中，由 `agena-application` 执行。HTTP 客户端、进程内插件、cdylib、stdio 和 HTTP 插件调用同一套业务实现。一般新增操作只需增加请求契约和 Application handler，通用 HTTP 入口与 HostClient 随之支持该操作。

## 1. 代码分层与调用路径

| 位置 | 职责 |
|---|---|
| [agena-session-api](../crates/agena-session-api/src/lib.rs) | `Query`、`Command`、各领域请求、共享 DTO、流和传输句柄、`SessionApiBackend` 低层执行端口 |
| [Application dispatcher](../crates/agena-application/src/dispatch/mod.rs) | 穷尽匹配操作，调用 Application 用例、repository 和 runtime service |
| [agena-api 的 QueryResult](../crates/agena-api/src/queries.rs) / [CommandResult](../crates/agena-api/src/commands.rs) | 成功响应的 `result/data` 包装及公开资源类型 |
| [HTTP operations](../crates/agena-api-server/src/routes/operations.rs) | 解码请求、调用公共 dispatcher、通过公共 JSON 编码池返回响应 |
| [Application backend](../crates/agena-application/src/application_bridge.rs) | 把相同 dispatcher 安装到 runtime 的共享 backend slot；插件调用无需 HTTP listener |
| [HostClient](../crates/agena-plugin-sdk/src/host_api.rs) / [HostClientExt](../crates/agena-plugin-sdk/src/host_api/operations.rs) | 插件侧 `query`、`command`、`query_as`、`command_as`；复用现有 `session_op` 传输 |
| [Rust HTTP client](../crates/agena-client/src/http.rs) | `AgenaClient::query`、`command` 直接转发到两个通用 HTTP 入口 |

HTTP 路径为：

`HTTP → Query/Command → Application dispatcher → 业务服务`

插件路径为：

`HostClient → host/session.op → SessionApiBackend → 同一 Application dispatcher`

stdio、HTTP callback 和 cdylib 的驱动已经传输完整请求枚举。新增一个业务变体不需要为每种插件传输增加方法名、白名单或另一套业务匹配。

## 2. 当前操作覆盖

下面列出当前枚举提供的能力。准确字段、默认值和 serde 标签以链接中的 Rust 契约为准。

### 2.1 顶层 Query / Command

契约：[queries.rs](../crates/agena-session-api/src/queries.rs)、[commands.rs](../crates/agena-session-api/src/commands.rs)。

| 能力 | Query | Command |
|---|---|---|
| 健康与运行状态 | `Health`、`Runtime` | 管理面提供 reload 与任务控制 |
| Provider 与模型发现 | `ListProviders`、`ListProviderModels`、`ListProviderAdapterModels`、`ListSavedProviderAdapterModels` | 管理面提供配置与认证写入 |
| 工作区 | `ListWorkspaces`、`GetWorkspace` | `CreateWorkspace`、`UpdateWorkspace`、`DeleteWorkspace`、`ResolveWorkspace` |
| 会话与树 | `ListSessions`、`GetSession`、`GetSessionState`、`GetSessionCost`、`GetSessionTree`、`GetSessionPlatforms` | `CreateSession`、`UpdateSession`、`UpdateSessionSelection`、`DeleteSession`、`ForkSession`、`ListSessionTree` |
| 运行与消息 | `ReadRuns`、`ReadParts` | `SubmitMessage`、`ContinueRun`、`CompactSession`、`ClearSession`、`CancelRun`、`RewindSession` |
| 内容资源 | `ReadContent`、`ReadContentText` | 扩展会话操作提供订阅和文件传输 |
| 交互回复 | 交互状态包含在会话与内容资源中 | `ReplyPermission`、`ReplyUserInput`、`MarkInteractiveRequestPresented` |
| 权限规则 | `ListPermissionRules`、`GetPermissionRule` | `UpsertPermissionRule`、`ReplacePermissionRule`、`RevokePermissionRule`、`DeletePermissionRule` |
| 后台活动 | `ListActivities`、`GetActivity`、`ActivityLogs` | `StopActivity`、`DismissActivity`、`ClearFinishedActivities` |
| 领域扩展 | `Business`、`Management`、`ExtendedSession`、`Server` | 同名四个领域分支 |

### 2.2 Business：Bot、助手、集成、记忆、通知

契约：[business.rs](../crates/agena-session-api/src/business.rs)。

| 领域 | 读取 | 写入与控制 |
|---|---|---|
| Bot | Bot 列表、详情 | 创建、更新、归档；确保默认或指定 Bot 会话；`CreateBotConversation` 创建新的 Bot 对话 |
| Assistant | 会话助手配置 | 更新助手、控制 reminder、修改助手记忆 |
| Integration | 会话集成列表、指定 binding | 创建集成、控制集成、更新渠道访问策略 |
| Memory | 列表、按名称读取、overview | 保存、删除、批量删除、确保索引 |
| Notification | 按 scope/source/severity/surface 等条件读取 | dismiss、解析 notification action 的目标 |
| Activity | 顶层活动查询 | pause、resume、delete；其余活动控制使用顶层 Command |

### 2.3 Management：控制面

契约：[management.rs](../crates/agena-session-api/src/management.rs)。这一层按 `domain` 分组，各领域内部通常再使用 `method/params`。

| 领域 | 共享能力 |
|---|---|
| `Settings` | effective/file 读取，global/workspace 分层读取与写入，批量 sources，entries 列举，resolved config，validate，set/patch/delete；保留 expected revision、dry run、validate、reload 选项 |
| `Providers` | configured models/adapters、AWS profiles、Provider Studio draft、draft models、model draft、catalog template；保存或删除 provider/adapter/model、draft auth start/continue、client versions refresh |
| `Auth` | provider 认证状态列表/详情，写 API key、删除或刷新凭证，browser start/finish、device start/poll |
| `ModelCatalog` | 搜索、origin 筛选与分页、lookup、match；`ModelCatalogRefresh` 启动刷新 |
| `Mcp` | 外部 MCP connection CRUD、enabled、bridge enabled、导入预览/导入、重连、测试、capabilities、resource、prompt、credential、OAuth start/complete/cancel/logout |
| `Skills` | 指定 workspace 的 catalog/detail、create/update/delete/enabled，resource read/write/delete，validate，install preview/install |
| `Plugins` | list/inspect/settings，architecture/surface，tool registry changes，logs，显式 tool 与 command 调用，Plugin Web settings 与 command |
| `Marketplace` | search、installed、outdated；sync、install、uninstall、upgrade 通过共享后台任务执行 |
| Runtime | `RuntimeTasks`、`RuntimeMetrics`、`RuntimeDiagnostics`；`ReloadRuntime`、`CancelRuntimeTask` |
| Usage | `Usage` 的 period、自定义日期区间、provider/model/session filters、include_subagents、timezone offset |
| Operator | `OperatorTools`、`InvokeOperatorTool`；由 Application 分配调用 ID |
| Permission batch | `PermissionRulesBatch` 按作用域批量删除或修改 permission mode |

Provider/model 的一般发现使用顶层 Query；Provider Studio 和配置管理使用 Management。MCP connection 管理与下节的 MCP server listener 控制分别有独立契约。

`RuntimeMetrics` 共用运行时业务计数；HTTP 请求、延迟及 Prometheus 服务器观测仍由 HTTP 层提供。

### 2.4 ExtendedSession：完整会话操作、流与文件

契约：[extended_sessions.rs](../crates/agena-session-api/src/extended_sessions.rs)。

| 类别 | 操作 |
|---|---|
| 补充读取 | `Overview`、`ExecutionAttempt`、`PromptPreview`、`Plan`、`ToolDetail`、`QueuedMessage`、`InlineUi`、`InlineUiSnapshot`、`FileChanges`、`WorkspaceStats` |
| 元数据与权限 | `UpdateMetadata`、`UpdatePermission`，携带可选 `expected_version` |
| 执行与队列 | `CancelExecutionAttempt`、`EditQueuedMessage`、`DeleteQueuedMessage`、`TakeQueuedMessage`、`PauseQueue`、`ClearQueue` |
| 临时问答 | `StartBtw`，返回可分页读取的流句柄 |
| 流生命周期 | `OpenStream`、`ReadStream`、`CloseStream` |
| 下载 | `Export`、`OpenMedia`、`ReadTransfer`、`CloseTransfer` |
| 上传 | `BeginUpload`、`WriteTransfer`、`FinishUpload`、`CloseTransfer` |

`FileChanges` 是会话中已记录编辑的业务投影。Web 工作台的任意文件树浏览、文件操作、Git、preview、PTY 继续使用各自专用接口。

### 2.5 Server：运行中的 listener 控制

契约：[server_control.rs](../crates/agena-session-api/src/server_control.rs)。

- `ServerQuery::Mcp`：读取 MCP server 状态、公开地址和 OAuth 配置状态。
- `ServerCommand::UpdateMcp`：更新 enabled、server URL、public URL、OAuth issuer URL。
- URL 更新区分“字段省略”“显式 null”“字符串”；对应字段沿用 `camelCase`。
- server composition 安装 `ServerControlPort`，HTTP 和插件读取同一 listener 状态。没有运行 listener 的 Application 调用该领域时，会返回 listener 不可用的服务状态。

服务器登录与 bearer session、授权重定向和握手、WebPush 浏览器订阅、ETag、条件请求、静态资源响应仍由相应传输层处理。

## 3. HTTP 请求与响应

两个通用入口：

| 方法与路径 | 请求体 | 成功响应 |
|---|---|---|
| `POST /api/v1/queries` | `Query` | `QueryResult` |
| `POST /api/v1/commands` | `Command` | `CommandResult` |

入口沿用服务器的认证处理。请求使用 `method/params`，成功响应使用 `result/data`。以下值用于展示完整的协议结构。

### 3.1 顶层操作

发往 `POST /api/v1/queries`：

~~~json
{
  "method": "list_providers"
}
~~~

对应成功响应可以是：

~~~json
{
  "result": "providers",
  "data": []
}
~~~

发往 `POST /api/v1/commands`：

~~~json
{
  "method": "clear_finished_activities"
}
~~~

成功响应示例：

~~~json
{
  "result": "activities_cleared",
  "data": {
    "count": 2
  }
}
~~~

`result` 取值由返回变体决定，例如 `providers`、`session`、`workspaces`、`execution`。`CommandResult::Ack` 是无载荷变体，序列化为：

~~~json
{
  "result": "ack"
}
~~~

### 3.2 领域操作

查询 runtime metrics，发往 `POST /api/v1/queries`：

~~~json
{
  "method": "management",
  "params": {
    "domain": "runtime_metrics"
  }
}
~~~

该领域通过 `QueryResult::Data` 返回：

~~~json
{
  "result": "data",
  "data": {
    "provider_calls_total": 0,
    "provider_calls_error": 0,
    "provider_stream_total": 0,
    "tool_executions_total": 0,
    "tool_executions_error": 0,
    "session_active": 0
  }
}
~~~

读取某个配置字段的请求：

~~~json
{
  "method": "management",
  "params": {
    "domain": "settings",
    "request": {
      "method": "get",
      "params": {
        "path": "session.max_turns",
        "source": "effective"
      }
    }
  }
}
~~~

`Business`、`ExtendedSession`、`Server` 使用各自的 `method/params` 子枚举；`Management` 先使用 `domain/request`，再进入具体领域。保持 Rust serde 契约，调用方无需自行维护操作名称映射。

失败使用既有 Application Failure 与 HTTP 错误响应；失败不会伪装成成功的 `result: "data"`。

### 3.3 HTTP 缓存行为

`AgenaClient::query` 和 `query_revalidated` 都通过通用 POST 执行一次新的业务读取。原有专用 HTTP 客户端方法继续使用它们的 ETag、条件 GET、读取合并与缓存。

新增业务优先通过通用操作发布。已有资源型 GET 可以继续作为薄适配器，保留浏览器缓存等传输行为。

## 4. 插件 SDK 调用

插件通过 `agena_plugin_sdk::operations` 使用共享契约，不需要依赖上层 `agena-api`。

| 方法 | 返回内容 |
|---|---|
| `HostClient::query(Query)` | 完整成功响应 JSON，包含 `result`、可选 `data` 及附加状态 |
| `HostClient::command(Command)` | 完整成功响应 JSON |
| `HostClientExt::query_as<T>(impl Into<Query>)` | 只把 `data` 解码为 `T` |
| `HostClientExt::command_as<T>(impl Into<Command>)` | 只把 `data` 解码为 `T` |

扩展 trait 适用于具体客户端、`&dyn HostClient` 和 `Arc<dyn HostClient>`。无 `data` 的 ack 解码为 JSON null；选择具体 `T` 时，需要它能接收相应载荷。

下面用 `serde_json::Value` 展示两种读取方式：

~~~rust
use agena_plugin_sdk::{HostClient, HostClientExt, Result};
use agena_plugin_sdk::operations::{
    Query,
    management::ManagementQuery,
};
use serde_json::{Value, json};

async fn inspect_runtime(host: &dyn HostClient) -> Result<Value> {
    // 保留 result/data 外层。
    let providers: Value = host.query(Query::ListProviders).await?;

    // 只取得 RuntimeMetrics 的 data。
    let metrics: Value = host
        .query_as(ManagementQuery::RuntimeMetrics)
        .await?;

    Ok(json!({
        "providers_response": providers,
        "metrics": metrics
    }))
}
~~~

用 `command_as` 预览一次配置修改：

~~~rust
use agena_plugin_sdk::{HostClient, HostClientExt, Result};
use agena_plugin_sdk::operations::management::{
    ManagementCommand, SettingsCommand,
    settings::{ConfigSettingsEditOptions, ConfigSettingsSetInput},
};
use serde_json::{Value, json};

async fn preview_setting(host: &dyn HostClient) -> Result<Value> {
    let response: Value = host
        .command_as(ManagementCommand::Settings(SettingsCommand::Set(
            ConfigSettingsSetInput {
                path: "session.max_turns".to_owned(),
                value: json!(12),
                options: ConfigSettingsEditOptions {
                    expected_revision: None,
                    dry_run: true,
                    validate: true,
                    reload: false,
                },
            },
        )))
        .await?;
    Ok(response)
}
~~~

需要完整任务状态时用 `command`：

~~~rust
use agena_plugin_sdk::{HostClient, Result};
use agena_plugin_sdk::operations::management::ManagementCommand;
use serde_json::Value;

async fn request_reload(host: &dyn HostClient) -> Result<Value> {
    host.command(ManagementCommand::ReloadRuntime.into()).await
}
~~~

`ReloadRuntime` 返回包含 `started` 和 `task` 的 data。调用方可以用 `ManagementQuery::RuntimeTasks` 或活动 API 跟踪该任务。

## 5. 启动、可用状态与 reload

业务 backend 由 Application 安装到共享 `SessionApiBackendSlot`。slot 在 runtime snapshot reload 之间保留，runtime 使用弱引用，单次调用取得 backend lease。独立 CLI 或嵌入式程序可使用 `Application::bootstrap` 组成同样的能力，无需启动 HTTP。

插件启动分成两个阶段：

1. `init` 准备配置和注册信息，并保存 HostClient。
2. 订阅 `application.ready`，在 `Plugin::application_ready` 中启动依赖业务 API 的工作。

首次 Application 组装完成后，以及后续已发布 runtime generation，框架都会发送 `application.ready`。保留实例应让后台任务启动保持幂等。backend 尚未挂载或候选 generation 尚未发布时，业务调用会返回明确的 unavailable 状态。

从插件调用中修改 settings、provider、auth 或 MCP 配置，可能需要 reload 当前 runtime。框架把这种 reload 安排到当前插件调用完成后执行，避免在自己的调用栈里等待自己被替换。

这时 Host 响应外层可以增加：

~~~json
{
  "result": "data",
  "data": {
    "saved": true
  },
  "deferred_reload_task_ids": [
    "rtask_example"
  ]
}
~~~

这里 `data` 的实际内容由具体操作决定；上例用于展示附加字段的位置。Settings 的 `reload` 结果还可包含 `deferred_task_id`，MCP mutation 可包含 `reload_task_id`。这些任务 ID 表示 reload 已安排，完成状态通过任务查询读取。

要读取 `deferred_reload_task_ids`，使用 `query` / `command` 保留外层；`query_as` / `command_as` 只返回 data。不要在当前插件回调结束前等待它所安排的 reload 完成。

## 6. 流：OpenStream → ReadStream → CloseStream

共享流是带句柄、可重试的分页读取。所有传输使用相同的操作：

1. `Command::ExtendedSession(SessionCommand::OpenStream(...))` 创建订阅。
2. `Query::ExtendedSession(SessionQuery::ReadStream(...))` 读取事件页。
3. `Command::ExtendedSession(SessionCommand::CloseStream { ... })` 释放订阅。

`StartBtw` 也返回相同的 `StreamHandle`。

### 6.1 可订阅的数据源

| `StreamSource` | 内容 |
|---|---|
| `Changes` | global/workspace/session 范围的会话变更与 runtime signals；session 范围先返回初始 snapshot，可指定 locale |
| `Content` | 指定 session/content resource，从 `ReadContentParams.after` 接续，保留内容资源的 cursor |
| `Notifications` | 按 filter 与 `since_ms` 订阅通知 |
| `InlineUi` | 指定 session/reference 的 versioned snapshot |

创建会话变更流的完整 HTTP 请求体：

~~~json
{
  "method": "extended_session",
  "params": {
    "method": "open_stream",
    "params": {
      "kind": "changes",
      "params": {
        "scope": {
          "kind": "session",
          "session_id": 42
        },
        "locale": "zh-CN"
      }
    }
  }
}
~~~

成功响应示例：

~~~json
{
  "result": "data",
  "data": {
    "stream_id": "11111111-1111-4111-8111-111111111111",
    "after_seq": 0,
    "idle_timeout_ms": 600000
  }
}
~~~

下一次发往 `POST /api/v1/queries`：

~~~json
{
  "method": "extended_session",
  "params": {
    "method": "read_stream",
    "params": {
      "stream_id": "11111111-1111-4111-8111-111111111111",
      "after_seq": 0,
      "limit": 64,
      "wait_ms": 30000
    }
  }
}
~~~

读取结果的 data 为 `StreamPage`：

| 字段 | 语义 |
|---|---|
| `events` | 事件数组；每项包含 `sequence`、`event`、`data` |
| `next_seq` | 当前页最后一个事件的序号；空页沿用请求 cursor |
| `lagged` | 请求位置已经落在保留窗口之外，需要恢复状态 |
| `closed` | 数据源已结束，并且当前页已读完仍保留的事件 |

处理完成一页后，再推进到它的 `next_seq`。响应丢失时，可用原 `after_seq` 重试；按 `sequence` 去重。`ReadStream` 默认最多返回 64 项，参数夹在 1–128；long poll 最多等待 30 秒。

每个句柄保留最多 256 个事件、约 8 MiB 数据。`lagged` 为 true 时，先重新读取对应资源状态，再继续订阅。内容流同时携带 `ContentPage.next_cursor`；恢复持久内容使用它填写 `ReadContentParams.after`。`StreamPage.next_seq` 只属于当前流句柄，不能代替内容 cursor。

关闭请求体：

~~~json
{
  "method": "extended_session",
  "params": {
    "method": "close_stream",
    "params": {
      "stream_id": "11111111-1111-4111-8111-111111111111"
    }
  }
}
~~~

成功返回 `{"result":"data","data":{"closed":true}}`。关闭会停止订阅的生产任务。

## 7. 附件、导入与导出：Transfer

文件操作使用临时文件和可定位的 transfer 句柄，JSON 中只传递有界 base64 分块。每块最多 **64 KiB 原始字节**；单个会话附件的大小上限是 **50 MiB**。导入会话归档使用 `UploadPurpose::Import`，沿用归档校验与 request ID 语义。

### 7.1 下载

1. 用 `SessionCommand::Export { session_id }` 导出会话，或用 `OpenMedia { session_id, path }` 打开会话媒体。
2. 从响应 data 读取 `TransferHandle`：`transfer_id`、`filename`、`size`、`chunk_bytes`、`idle_timeout_ms`。
3. 用 `SessionQuery::ReadTransfer(ReadTransferParams { transfer_id, offset, max_bytes })` 读取。
4. 解码 `TransferChunk.data_base64`，写入 `offset` 对应位置；用 `next_offset` 继续，直到 `eof`。
5. 用 `SessionCommand::CloseTransfer { transfer_id }` 释放句柄。

下载可重复读取同一 offset；响应丢失后重读该块即可。客户端应检查返回的 offset/next_offset，而不是按 base64 字符串长度计算文件位置。

### 7.2 上传

1. 用 `BeginUpload` 指定用途：`Attachment { session_id, filename, mime }` 或 `Import { request_id }`。
2. 从返回的 `TransferHandle` 取得 ID，从 byte offset 0 开始。
3. 用 `WriteTransfer { transfer_id, offset, data_base64 }` 写入每块；响应 data 的 `next_offset` 是服务端当前已存长度。
4. 所有字节确认写入后，调用 `FinishUpload { transfer_id }`。
5. 取得最终附件或归档导入结果后，调用 `CloseTransfer`。

写入按当前尾部追加。同一 offset 的相同字节可以重试；不同字节、跨越未写区域的重叠块、跳过尾部的位置会返回冲突。这样网络重试不会改写已确认的内容。

第一次 `FinishUpload` 启动由 transfer 持有的提交任务，并停止接受新的块。之后即使原调用断连，任务仍会完成；重试同一 handle 会等待或返回同一成功/失败结果。导入的 `request_id` 还会传给归档业务服务。

### 7.3 回收与恢复

Stream/Transfer handle 的空闲时间是 **10 分钟**，后台每 30 秒扫描回收。正在提交的 upload 会保留至提交结束，完成后重新计算空闲时间。主动 Close 可以及时释放资源。

这些句柄属于当前 Application 进程。句柄过期或进程重启后，需要重新打开操作；持久内容可从保存的 content cursor 恢复。流的事件序号和文件传输的 byte offset 分别服务于各自的句柄。

实现见 [operation_io.rs](../crates/agena-application/src/operation_io.rs)、[streams.rs](../crates/agena-application/src/dispatch/streams.rs)、[extended_sessions dispatcher](../crates/agena-application/src/dispatch/extended_sessions.rs)。

## 8. 增加一个业务操作

### 8.1 常规开发步骤

1. 在现有顶层枚举或合适的领域子枚举增加 typed variant。请求 DTO 放在 `agena-session-api` 或已有低层领域 crate；沿用 serde 标签、字段默认值和并发版本字段。
2. 在 `agena-application/src/dispatch` 对应领域增加匹配分支，调用已有 Application/service 方法。需要的新业务逻辑集中放在 Application 或更低层服务。
3. 返回公开资源。已有具体结果使用对应 `QueryResult` / `CommandResult` 变体；领域扩展在边界通过 `Data(Value)` 返回其公开资源。
4. 为新增行为补充必要的契约与业务回归，特别检查 revision、分页、幂等请求 ID、跨 transport 的数据形状。
5. 如需专用 REST URL，再增加薄路由做 path/query/header 到 typed request 的映射，并重新生成路由目录。

通常第 1、2 步就让通用 HTTP、HostClient 和所有现有插件驱动识别并执行新操作。穷尽匹配让缺少 handler 的变体在编译时暴露；不要为普通业务增加另一份 HTTP/Host 操作名单。

需要运行环境独有能力时，增加低层 service port 并在 composition 中安装；例如 MCP server listener 使用 `ServerControlPort`。业务 handler 使用该端口，HTTP 不承载唯一一份实现。

### 8.2 请求和响应设计

- 开放配置值、工具输入、插件设置可以使用 `Value`；操作名称、目标 ID、分页、版本和控制动作仍使用明确字段及枚举。
- 请求中已有 `expected_version`、`expected_revision` 或 `request_id` 时，沿用服务原有并发与幂等语义。
- 保持成功载荷与失败通道分离；不要把“未执行”“不支持”返回成成功资源。
- `agena-plugin-sdk` 已依赖 `agena-session-api`，通过 `operations` re-export 请求。共享请求类型应留在低层，避免让 SDK 反向依赖 `agena-api`。
- 此项目同步更新 server/client 合同；破坏性修改直接更新当前合同和调用方。专用 REST 路径可以保留作薄映射，不增加历史协议世代或降级分支。

### 8.3 路由文档

`route_catalog.rs` 由实际 router 生成：

~~~sh
python3 scripts/agena/generate-route-docs.py
python3 scripts/agena/generate-route-docs.py --check
~~~

它记录 HTTP 挂载信息。业务操作本身的完整清单来自 `Query` / `Command` 及领域子枚举；新增领域操作通常不需要新增 HTTP 路径。
