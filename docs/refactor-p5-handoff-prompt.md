# 交接提示词 — 给下一个接手 v2 重构的 AI

你在 `/Volumes/Rc20/Projects/agena/.agena/worktrees/db-design-audit`（git 分支 `research/db-design-audit`）。
任务：**继续并完成 v2 持久化/执行数据层重构**，依据 `docs/refactor-prompt.md`，设计依据
`docs/database-design-v2.md`。**先读 `docs/refactor-p5-handoff.md`**（详细进度、关键事实、验收门都在里面），
再继续下面的工作。不要问我问题，直接干活。

## 铁律（不可违背）

1. 无向后兼容、无迁移、无死代码、无死数据。
2. 门面封闭：外部代码只允许通过 `agena_storage::store::SessionStore` 访问会话数据，
   裸 SQL 只允许出现在 `PersistenceEngine` 实现里。
3. 无事件概念：`NotificationBus` 只发 `SessionChange`（实时通知，永不持久化、永不重放）。
4. 单一会话状态由 parts + leases 推导（17.3）。
5. 若设计文档与 `docs/refactor-prompt.md` 冲突，以 prompt 为准。
6. 不要动 `claude-reverse/`、`demo.md`、`demo.txt`、`hello.txt`、`sample.txt`。
7. 提交按阶段、用 `scope(db):` 前缀，保持可评审。
8. 每完成一步运行一次 `cargo check -p <crate>`，全绿再提交；不要一次改太多再一起编。

## 当前基准（已验证）

- `cargo check -p agena-storage -p agena-storage-sqlite -p agena-runtime-session -p agena-runtime -p agena-application`
  → **0 error 0 warning**。
- `cargo check --workspace` 只剩 3 个 crate 报错：`agena-api-server`（32）、`agena-tui-backend`（8）、`agena-cli`（2）。
- 下游 `agena-client`/`agena-tui-app`/`agena-e2e`/`agena-web` 被上述阻塞。

## 继续 P5（按顺序）

1. **agena-tui-backend**（8 处）：`backend_session.rs` 的 `list_session_timeline` /
   `refresh_session` / `subscribe_session_events` 从 v1 事件表面改到
   `SessionQueryService`（transcript_snapshot / latest_event_seq）+ `SessionStore::subscribe(SessionChange)`
   + `RuntimeLiveSignalService`；`lib.rs:45` 删掉 `EventFilter/EventScope` 导入。
   `RuntimePresentationEvent`/`RuntimeLivePresentationSubscription` 仍保留（见进度文档），数据源换成 parts。
2. **agena-api-server**（32 处）：删 `rest/events.rs`（D11）；`rest/sessions.rs` 的
   `list_session_events`/`stream_session_events` 改成 part patches（19.4）；
   `sse.rs`/`ws.rs`/`ipc.rs` 改 `subscribe_all(SessionChange)` + `RuntimeLiveSignalService`；
   `state.rs` 删 event 访问器；`lib.rs` 删 `/events` 路由。
3. **agena-cli**（2 处）：`cli/mod.rs` 删 `RuntimeEventPublishService`/`RuntimeEventPublishRequest`
   权限规则事件发布路径。
4. **agena-client / agena-tui-app / agena-e2e**：逐个解锁——删 `list_events`/`events_url`/
   `events_stream_url`/`append_event_query`/`Event` 订阅变体/`QueryResult::Events`；
   e2e 的 `subscribe_events(EventFilter::Session)` + `message_part_checkpointed` 等待改为
   `subscribe(session_id, SessionChange)`（等 `PartAdded`/`PartUpdated`）。
5. `cargo check --workspace` 全绿 → **提交 P5**（`scope(db):`）。

## 然后 P6（死代码清扫）

- grep v1 遗留标识符（清单见进度文档）→ 零命中。
- 无迁移代码/文件残留。
- `cargo clippy --workspace -- -D warnings` 全干净（先 `-p` 逐个，再全量）。

## 最后 P7（测试 + 基准，验证门 3–10）

- `cargo test` 全绿（v1 测试模块在 `SessionStore` 上重新表达——`sessions.rs` 的计数测试已重写做样板）。
- 并发 / resume / retry / usage / JSONL 往返 / `EXPLAIN` 索引验证 / 无裸 SQL 出界。

## 完成条件（全部满足才算完成，不要提前收工）

- 10 道验证门全过（见 `docs/refactor-prompt.md` / 进度文档）。
- 每阶段有 `scope(db):` 提交。
