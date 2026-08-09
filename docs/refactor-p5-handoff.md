# v2 重构进度 — P5 中途交接（2026-08-09）

> 目标：按 `docs/refactor-prompt.md` 彻底完成 v2 持久化/执行数据层重构，
> 设计依据 `docs/database-design-v2.md`（"everything is a part" 会员优先模型）。
> 硬性约束：无向后兼容、无迁移、无死代码、无死数据、门面封闭（外部只走 `SessionStore`）、
> 无事件概念（`NotificationBus` 只发 `SessionChange` 实时通知，永不持久化）、
> 单一会话状态由 parts + leases 推导。

## 阶段状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| P0 | 侦察并删除 v1 遗留层 | ✅ |
| P1 | v2 schema（DDL + sequences + leases + triggers） | ✅ |
| P2 | PersistenceEngine（SQLite）+ InMemoryEngine | ✅ |
| P3 | SessionStore facade + MemoryLayer + NotificationBus | ✅ |
| P4 | 执行引擎改线（P4a–P4d） | ✅ |
| **P5** | **查询/UI 表面迁移到 parts** | **🔄 进行中** |
| P6 | 死代码清扫 | ⬜ |
| P7 | 测试 + 基准（验证门 3–10） | ⬜ |

最近提交（已提交部分）：`P4a`–`P4d`（`scope(db):`），加上更早的
`feat(db): v2 schema / persistence engines / SessionStore facade`。

## 当前 git 状态

分支 `research/db-design-audit`，工作树 `/Volumes/Rc20/Projects/agena/.agena/worktrees/db-design-audit`。

未提交改动（P5 进行中的全部工作）：
- **新增文件**：`crates/agena-runtime-session/src/session/store.rs`、
  `crates/agena-runtime-session/src/session/transcript.rs`、
  `crates/agena-runtime/src/live_signal.rs`
- **已删除**：`crates/agena-application/src/event_projection.rs`（D11 全局事件投影废弃）
- **已修改**：`agena-storage`(store/{types,engine,facade,in_memory,mod})、`agena-storage-sqlite/engine.rs`、
  `agena-runtime`(lib/application_services/runtime/builder/live_signal)、`agena-runtime-session`(大量)、
  `agena-domain`(session_summary/execution_status)、`agena-cli`(两处 SessionListRequest 默认值)、
  `agena-application`(application/dispatch/service 全部改线)

## 当前编译状态（关键基准）

- `cargo check -p agena-storage -p agena-storage-sqlite -p agena-runtime-session -p agena-runtime -p agena-application` **全部干净，0 error 0 warning**。
- `cargo check --workspace` 仅剩 **3 个 crate** 报错：
  - `agena-api-server`（32 处）
  - `agena-tui-backend`（8 处）
  - `agena-cli`（2 处）
- 下游 `agena-client`、`agena-tui-app`、`agena-e2e`、`agena-web` 因依赖上述 crate 暂未暴露错误，
  属被阻塞状态（grep 已验证它们引用已删除的 `RuntimeEventStreamService`/`EventFilter`/`RuntimeEvent` 等）。

## P5 已完成部分（本阶段核心成果）

### 存储层扩展（已提交工作的一部分）
- `SessionListQuery` 增加 `workspace_id / parent_id / roots_only / search / limit / before` 过滤字段。
- `PersistenceEngine` + `SessionStore` trait 增加：
  `get_session_summary(session_id)`（单行投影，存在性/lifecycle/version 检查）与
  `session_counts_by_workspace(&[i64])`（13.5 workspace_counts）。
- SQLite / InMemory 双实现齐全；`SessionFacade` 全委托。

### 应用层改线（本次会话，全部未提交）
- `agena-runtime/src/application_services.rs`：`RuntimeApplicationServices` +
  组合输入 + 组装函数增加 `session_store: Option<Arc<dyn agena_storage::store::SessionStore>>`。
- `agena-runtime/src/runtime/builder.rs`：`application_services_with_manager_option` 通过
  `session_manager.session_store()` 注入门面。
- `agena-application/src/service/mod.rs`：
  - 删除 `publisher` 字段与 3 个已删仓库（`session_stats/session_summary/session_mutation_repository`），
    删除 `session_queries` 字段（query service 始终按参数传入）。
  - 新增 `session_store: Arc<dyn SessionStore>` + `session_store_facade()` 访问器。
  - `ApplicationService::new` 新签名：
    `(workspace_root, memory_repo, workspace_repo, permission_rule_repo, session_store)`（5 参数）。
  - `ensure_session_model` → `session_store.get_session_summary`（返回 `store::SessionSummary`，检查 Ready）。
  - `workspace_session_counts` → `session_store.session_counts_by_workspace`。
  - 删除死代码 `EventCursor`。
- `agena-application/src/service/sessions.rs`：
  - `list_sessions` → `session_store.list_session_summaries`（新过滤字段；cursor 类型转换 + limit 转 i64）。
  - `get_session` → `get_session_summary`。
  - `create_session` → `session_store.create_session(NewSession{relation_kind: Child, ...})`。
  - `replace_session` → `session_store.rename`。
  - `delete_session` → `session_store.delete`。
  - **删除 `list_session_events`**（D11）。
  - 新增 `session_resource_from_storage_summary(&store::SessionSummary)`（counts 来自 summary 本身，
    不再单独统计查询；`subtask_access` v2 恒 None）与 `session_resource_from_storage_meta(&SessionMeta)`
    （create/rename 后 counts 为 0）。
  - 测试重写：v1 `EventStore` fixture → 直接在 facade/engine 上建 session + `submit_user_message`
    造 run marker；新增 create/rename/delete 往返测试。
- `agena-application/src/service/execution.rs`：删除 `list_session_events_after` + `runtime_event_query_error`（D11）。
- `agena-application/src/service/permissions.rs`：删除 `publish_permission_rule_event` +
  `permission_rule_record_event` + `publisher` 用法（无事件发布概念）；测试 `new` 调用同步更新。
- `agena-application/src/application.rs`：
  - 删除 `event_queries/event_stream` 字段，新增 `live_signals`；
    destructure 新 `RuntimeApplicationServices`（`session_store`/`live_signals`）。
  - `spawn_notification_aggregator` 重写：`facade.subscribe_all(SessionChange)` → 仅 notice part
    投影为 banner 通知（14.3）+ `RuntimeLiveSignal::Activity` → activities 通知。
  - 删除 `event_stream_service()/event_query_service()` 访问器，新增 `session_store_facade()`。
  - `notification_from_runtime_event` → `notification_from_session_change(SessionChange)`。
  - 测试重写为 `SessionChange::PartAdded`（notice）投影 + 非 notice 不投影。
- `agena-application/src/dispatch/queries.rs`：删除 `ListEvents` 查询臂 → 显式 D11 错误
  （"global runtime event history was dropped in v2 (D11)"）；清理无用 import。
- `agena-application/src/event_projection.rs`：**已删除**（D11）；`lib.rs` 相应移除模块声明。

## P5 剩余工作（下一步 AI 的任务清单）

### 1. agena-tui-backend（8 处，先行）
`crates/agena-tui-backend/src/backend_session.rs` 使用 v1 事件表面，需改线：
- `list_session_timeline`：`RuntimeEventQueryService.list_timeline_events_before` → 改为
  `SessionQueryService` 提供的 v2 表面（`transcript_snapshot` / `list_projected_messages` /
  或 facade 的 `session_view`/parts），返回新时间线条目类型。
- `refresh_session`：`list_events_before`（最新 seq）+ `list_events`（计数）→ 用 `latest_event_seq`
  （v2 = session version 高水位）+ parts 计数。
- `subscribe_session_events`：`RuntimeEventStreamService.subscribe_presentation_events(EventFilter)`
  → 改为 `SessionStore::subscribe(session_id, SessionChange)`（part 补丁）+ 需要时
  `RuntimeLiveSignalService`（活动/插件信号）。注意 `RuntimePresentationEvent`/`RuntimeLivePresentationSubscription`
  **仍然存在**（`presentation_event.rs`，经 agena-runtime lib.rs:207-208 重导出）——执行计划说
  "presentation_event.rs (rewired to part patches)"，即保留 TUI 订阅协议、把底层数据源换成 parts。
- `lib.rs:45` 导入 `agena_domain::EventFilter/EventScope` → 删除/替换。

### 2. agena-api-server（32 处）
- `rest/events.rs`（`list_events` 端点）：**整文件删除**（D11/19.7）。
- `rest/sessions.rs`：`list_session_events` + `stream_session_events`（19.4 行）→ 设计 19.4：
  "wire format changes from event envelopes to part patches"；用 facade `session_state`/parts
  或 `SessionQueryService.transcript_snapshot` 重写。
- `sse.rs`、`ws.rs`、`ipc.rs`：`RuntimeEventStreamService`/`RuntimeLiveEventSubscriptionItem`/`EventFilter`
  → `SessionStore::subscribe_all/subscribe(SessionChange)` + `RuntimeLiveSignalService`。
- `state.rs`：`event_stream_service()/event_query_service()` 访问器 → 移除。
- 所有 `event_projection::event_resource_from_runtime` 引用 → 随端点删除。
- `lib.rs` 路由 `/api/v1/sessions/{id}/events` + `/events/stream` → 移除。

### 3. agena-cli（2 处）
`crates/agena-cli/src/cli/mod.rs:1608/1687`：`RuntimeEventPublishService`/`RuntimeEventPublishRequest`
（权限规则事件发布）→ 删除该发布路径（v2 无事件发布概念）。

### 4. 下游（依赖上述，逐个解锁）
- `agena-client`：`http.rs` `list_events`/`events_url`/`events_stream_url`/`append_event_query`、
  `ws.rs` `Event(EventResource)` 订阅变体、`QueryResult::Events` 映射 → 随 API 契约删除/改线。
- `agena-tui-app`：`app_types.rs` LiveEvent、`app_session_events/requests.rs`、`interactive.rs`
  `subscribe_session_events`。
- `agena-e2e`：`dsv4f_tool_api_suite.rs` + `support/mod.rs` 中
  `RuntimeEventStreamService.subscribe_events(EventFilter::Session)` + `message_part_checkpointed`
  等待逻辑 → 改 facade `subscribe(session_id, SessionChange)`（等待 `PartAdded`/`PartUpdated`）。
- `agena-web`（如存在引用）。

### 5. P5 收尾
- `cargo check --workspace` 全绿。

## P6 死代码清扫（其后）

- grep 遗留标识符 → 零命中（v1 事件/仓库命名：`RuntimeEvent`、`RuntimeEventStreamService`、
  `RuntimeEventQueryService`、`RuntimeEventPublishService`、`EventFilter`、`EventScope`、
  `EventEnvelope`、`EventStore`、`SessionSummaryRecord`、`SessionSummaryRepository`、
  `SessionStatsRepository`、`SessionMutationRepository`、`SessionEventStats`、
  `event_projection`、`EventCursor`、`list_session_events`、`list_session_events_after`、
  `list_events`、`ListEvents`、`append_event_query`、`events_url`、`events_stream_url` 等）。
- 无迁移代码/文件（v1 schema 目录、`migration`、`schema_lifecycle` 中残留）。
- `cargo clippy --workspace -- -D warnings` 全干净。

## P7 测试 + 基准（验证门 3–10）

- `cargo test`（含 v1 测试模块在 `SessionStore` 上的重新表达，如
  `session_list_materializes_message_and_child_counts` 已重写）。
- 并发测试（SQLite engine_tests 已有跨连接池/多进程模拟）、resume、retry、usage、
  JSONL 往返（engine_tests 已覆盖）——补齐 facade 层与 TUI 数据面的等价测试。
- `EXPLAIN` 查询计划验证派生 SQL 走索引（13.5）。
- 无裸 SQL 在 `PersistenceEngine` 之外（门 10）。

## 关键设计事实（接手者必读）

- **parts 是唯一聊天内容实体**；消息 == `kind='run'` marker part；排序恒 `ORDER BY created_at_ms, part_id`（4.1/4.2）。
- **`SessionSummary`（storage）自带** `message_count`（=run markers，D9）、`child_session_count`、
  `last_message_at_ms` —— 应用层列表不再需要单独统计查询（13.5 派生 SQL 在 engine 内）。
- **`SessionChange`（PartAdded/PartUpdated/PartRemoved/SessionMetaUpdated）** = 唯一实时更新概念；
  `SessionObserver = Arc<dyn Fn(SessionChange) + Send + Sync + 'static>`；`subscribe_all` 返回 `GlobalSubscription`。
- **`RuntimeLiveSignal`（Activity/Plugin/ToolRegistryChanged）** = 非 parts 的瞬态运行时信号
  （broadcast，可 Lagged），与 facade 的 `SessionChange` 并行（14.3）。
- **门面封闭**：`SessionStore` trait + `SessionFacade<E>`；`engine()` 不在 trait 上；外部禁止裸 SQL。
- **无事件概念**：v1 的 `RuntimeEventStreamService`/`RuntimeEventQueryService`/`RuntimeEventPublishService`/
  `EventFilter`/`EventScope`/`RuntimeEvent` 已全部删除。
- `latest_session_event_seq` 保留（`SessionQueryService.latest_event_seq`，v2 映射为 session version 高水位）。
- `SessionManager` 有 `pub fn session_store(&self) -> Arc<dyn SessionStore>` 访问器。
- 提交规范：每阶段 `scope(db):` 前缀；提交要可评审。
- 不要动 `claude-reverse/`、`demo.md`、`demo.txt`、`hello.txt`、`sample.txt`。
- 若设计文档与 prompt 冲突，以 prompt 为准（`docs/refactor-prompt.md`）。
