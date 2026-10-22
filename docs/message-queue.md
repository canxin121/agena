# 服务端消息队列

用户在 AI 工作中发送消息时，服务端立即保存输入并返回提交凭据。当前 execution 的所有模型轮次、工具调用、交互等待及 stop-hook 后续工作结束后，后端依次运行每条待发消息。客户端关闭、切换会话或断线不会阻止队列继续工作。

## 调研依据

2026-10-22 阅读了相邻仓库的实现，以下 commit 是调研时本地 checkout 的版本。

| 项目 | 核心实现 | 队列所有权及调度 | Agena 的取舍 |
| --- | --- | --- | --- |
| Codex `6e5f665fe` | `../codex/codex-rs/ext/queue/src/service.rs`、`ext/queue/src/lib.rs`、`app-server/src/request_processors/thread_queue_processor.rs`、`tui/src/chatwidget/input_queue.rs` | `QueuedItemService` 通过 `QueueStore` 持久化待发输入；每个 thread 串行处理修改和派发，调用 `start_turn_if_idle`，成功接收后再移除队列项；app-server 提供查询、更新、删除和排序接口。TUI 另有输入展示及待提交状态。 | 采用后端持久化、串行派发和服务端条件操作；Agena 复用已有的 user entry 与 queued attempt，不另建一套消息存储。 |
| OpenCode `c10134729d` | `../opencode/packages/core/src/session/input.ts`、`session/runner/llm.ts` | 按 durable admission sequence 保存待处理输入，`promoteNextQueued` 每次提升一条；runner 先完成当前 turn 的 continuation 循环，再提升下一条 queue。`steer` 有独立边界。 | 等待整次 execution 结束后再运行下一条，每条消息保持独立，不在工具后的 follow-up 中提前注入。 |
| Gemini CLI `9b6e026` | `../gemini-cli/packages/cli/src/ui/hooks/useMessageQueue.ts`、`AppContainer.tsx`、`components/InputPrompt.tsx`、`hooks/useGeminiStream.ts` | React `useState<string[]>` 保存队列；stream idle、MCP ready、非压缩状态时，将全部待发文本合并后调用 `submitQuery`。 | 输入时保持可发送这一交互有参考价值；队列所有权与文本合并方式不符合 Agena 的多客户端、后端恢复要求。 |

## Agena 的执行契约

1. `SessionManager::submit` 独立于执行锁完成接收。每个会话的 submission 锁依次覆盖准备附件、提交 user entry / queued attempt 和注册 supervisor；HTTP 响应只等待接收落盘。
2. supervisor 在 spawn 之前同步链接前一个执行，随后等待它结算，再取得会话 execution 锁。Tokio 对任务的首次轮询顺序不会改变 FIFO。不同会话继续并行运行。
3. 当前 round 冻结输入时，以正在执行的用户 EntryId 为上界，排除后续消息及其输出。hydrator 按消息归属排列历史，保证已结束的 assistant 回复在下一条 user 之前。取消且从未开始过的用户输入保留历史记录，但不进入后续 prompt；取消一次尚未开始的重试不会抹去之前已使用的用户输入。
4. 压缩只选取到第一条后续用户消息之前的连续前缀。不能略过待发消息再覆盖其后的旧回复，否则 checkpoint 会永久跳过待发输入。压缩结果成功提交后，后续待发输入仍保留在 prompt window。
5. 正常关闭时取消已开始的执行，未认领的输入保留为 queued。内核打开时将崩溃遗留的 running 标为 interrupted，后端恢复扫描重新注册 queued；扫描覆盖所有工作区。新提交也先恢复同一会话的旧队列，防止扫描尚未到达时发生越序。已经开始过的外部调用不会因队列恢复而重放。
6. 现有 runtime admission 上限为 128 个受监督执行，包含准备、运行及等待中的工作；队列满时拒绝新的接收，客户端保留未提交草稿。状态 shell 继续采用现有有界 control snapshot，队列只返回引用，正文走已有 parts 接口。

`queued_messages` 由同一个持久化 control snapshot 派生，按 `message_id` 排列：

```json
{
  "queued_messages": [
    { "execution_id": "00000000-0000-0000-0000-000000000002", "message_id": 42 }
  ]
}
```

Web 和 TUI 都直接调用发送接口，然后展示这个投影。TUI 的 Enter 和 Ctrl+Enter 都立即提交到后端，HTTP 接收确认之后可继续提交下一条。

## 待发输入与正式对话的展示

待发输入尚未进入 AI 的当前上下文，因此不能作为普通 user 回合插在当前 assistant 回复前面。Web 在输入框上方固定显示独立的待发区域，按 FIFO 编号展示数量及正文预览，明确说明当前回复结束后依次发送。列表高度有上限，较长队列可以滚动；附件输入展示文件名，多行正文保留原文档、仅在预览中压缩空白。待发区域不提供普通回合的 fork / revert 操作。TUI 在对话之外的 footer 显示数量及队首预览。

后端 `/runs` 的会话分页只选择已经开始执行的用户输入及其 assistant 回复。待发与从未执行就被取出的输入不占用首屏或历史分页的消息额度，也不计入会话导航的用户消息总数；原始 entry 仍保留，可通过明确的 parts 身份读取。客户端根据 `queued_messages` 引用补读队列正文，不扫描历史，不调度队列。

每个公开 part 的 `message_id` 指明所属用户输入。会话分页按 `(message_id, boundary_entry_id)` 排列；Web 和 TUI 的展示也使用这个归属顺序。parts 读取及内容展开继续使用原有的物理成员游标。即使后续 user entry 先落盘、上一轮的首个思考或最终回答后落盘，也始终显示为上一条 user、它的完整 assistant 回复、下一条 user。缺少中间用户边界时，不同输入所属的 assistant 回复也不能折成同一块。

用户 part 的 `content.queue_only` 表示该输入从未被任何 attempt 认领。认领前 `run_state=pending`；认领后转为 `completed` 并进入正式对话。取出后仍保留 `queue_only=true`、`run_state=cancelled` 的审计记录，展示端不把它补回正式对话。重试已经使用过的输入不会重新隐藏原来的 user 回合。公开 part 的 revision 同时覆盖内容修订、attempt generation 和执行阶段的推进，避免正文未修改时，刷新或迟到事件让缓存永远保留 pending。

## 编辑与取消待发消息

TUI 的 Ctrl+P 取出队首消息供编辑，Ctrl+X 取消队首消息。两者调用：

```text
POST /api/v1/sessions/{session_id}/queue/{execution_id}/take
{ "request_id": "<CommandId>" }
```

服务端读取不可变原文档，随后在 writer 事务中执行 `CancelQueued`。只有 current attempt 仍为 queued 时才成功；它与 executor claim 共享串行写入边界。成功返回原始 `ComposerDocument`，历史中的 user part 标记为 cancelled。编辑后的发送是一次新的消息提交。

若 claim 先完成，接口返回 HTTP 409 并保持运行中的执行。若另一个客户端先取出消息，也返回冲突。相同 `request_id` 的成功重放保持幂等。客户端收到冲突后重新读取服务端状态。

## 回归验证

- runtime：当前 execution 有多个 provider round 时，后续输入不会提前进入 prompt；三条输入按 FIFO 运行，上一条完整回复先于下一条 user。
- runtime：移除队列中间的消息后，当前执行继续，下一条保留消息照常执行。
- runtime：真实 SQLite 关闭、释放服务所有权并重新打开后恢复 queued；不重放关闭前的 provider 请求。
- runtime：新提交早于后台恢复扫描时，旧队列先执行；其他工作区的待发输入也会被扫描恢复。
- storage：内存与文件 SQLite 都验证 follow-up 输入边界、压缩 checkpoint 不越过队列，以及条件移除不能取消 running。
- HTTP：独立客户端查看同一队列；提交端断开后待发仍在；移除只影响目标 queued 消息，running 返回冲突。
- 真实进程：发送端断开后杀死正在工作的 server，再启动新 server；后台自动恢复两条待发输入，按顺序各执行一次，旧的 running attempt 只标记 interrupted。
- TUI：AI running 或等待权限时，两种发送键都立即发起后端提交。
- Web：服务端队列投影从两条变一条再变为空时正确更新；客户端不自行派发下一条。待发 user part 保持后端 pending 状态。
- 展示回归：三个 user 先落盘、旧回复后落盘时，首屏仍展示当前对话，队列单独预览；认领后移到完整上一轮回复之后；历史翻页保持相同顺序。撤回不产生空 user 回合，queued retry 保留已执行的原始输入，状态更新具有递增版本。
- 连续执行且漏收实时事件时：Web 和 TUI 补读已加载但已退出最近窗口的运行中回复，恢复最终状态与正文；修复后不再重复读取已结束的旧回复。
