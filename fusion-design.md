# agena × Claude Code 异步融合设计:通知即 part,part 即通知

> 目标:让 agena 采纳 Claude Code 的异步任务机制 —— AI 调用后台工具后继续工作,工具结束后 AI 能**自己知道并处理结果**,而不是去轮询。同时严格守住 agena 的 **everything-is-a-part** 理念:新机制的全部状态、全部可观测物、全部模型可见内容,都落在 `parts` 表里。
>
> 配套材料:`claude-async-runtime-notes.md`(Claude 运行时逆向)、`async-tool-research.md`(三方案总览)。
> 本文件是**融合落地方案**(对应需求 2),并已覆盖需求 3 的两块追加调查:**Monitor 工具**(§7.3)与 **TUI 输入区展示**(§11)。

---

## 0. 一句话结论

agena 已经拥有 Claude Code 异步机制约 70% 的基础设施(`tasks.create`/`shell.run background`/`BackgroundOperation` marker/`BackgroundCompletionBridge`/空 `tool_result` guard 让 turn 能正常结束)。**唯一缺失、也是唯一关键的一环**是:

> 后台操作完成后,Claude 会向父会话注入一条 `<task-notification>` 高优先级消息并**唤醒模型开新 turn**;agena 目前只把 tool_call part 终态化,**从不通知模型,也不触发新 turn**。

本次设计就是在这一环上补齐,并且完全以 part 为载体:

- 新增 part kind `system_notification`(模型可见,**Assistant 角色**);
- **不开新 run**:`settle_background_run` 把通知 part append 进启动 run(不新增 marker),`run_until_stable` 用通知游标重开模型 turn;
- 在 `settle_background_operation`(统一终态化 + 追加 + 唤醒)里一次性完成,idle 走 `execute_registered`,mid-turn 走 `steer_input` 增强;
- 系统提示词新增 `# Background execution` 纪律段;
- `tasks.create` / `shell.run background` 的返回文案改为 Claude 式 "You will be notified… Do not poll";
- **唯一新增工具 Monitor**(§7.3):持续流后台监听,事件逐条通知;
- **输入区展示纯 TUI 化**(§11):`● {count} background` chip 从 10s 轮询改为**事件驱动**(对位 Claude `background_tasks_changed`),并升级为 per-task pill;part 模型零改动。

---

## 1. Claude 机制回顾(要贴齐的部分)

| Claude Code | agena 现状 | 差距 |
|---|---|---|
| `M3t`/`M9s` 完成后 → `kd({mode:"task-notification", priority:"next"})` 注入高优先级消息 | `complete_background_operation` 只终态化 part | **缺通知 part + 唤醒 turn** |
| `<task-notification>` XML 以 user 消息进入上下文,系统提示词说明 "look like user messages but are not" | 无对应机制;`notice` 投影为**空**(wire_message.rs:266) | 缺模型可见的通知载体 |
| `I4e` 原子 claim(`notified` 标志防重复通知) | 无;`SessionMetaUpdated`/`on_finished` 可能重复触发 | **已落库的 `system_notification` part(按 kind+id)即 claim**,settle 入口查重(§4.3) |
| `priority:"next"` 高优先级排队 | steer/submit 都是 next-turn 语义 | 基本等价,无需独立优先级通道 |
| `run_in_background` 字段(Agent/Bash) | `tasks.create` / `shell.run background=true` | 语义对齐,文案未对齐 |
| keepalive 防 evict / stall watchdog | agena 会话是持久实体;child task 有 `timeout_ms` | 不需要 keepalive;watchdog ≈ child subtask 超时 |

**身份语义:AI 发起 → AI 结果。** 后台操作是模型发起的(`tasks.create` / `shell.run background`),所以完成通知的 part 身份也是 **AI(Assistant)**,而不是 System——这是本设计的核心取舍:通知 part 以 `PartRole::Assistant` append 进启动 run,投影成 `Role::Assistant` 消息的一部分(§3)。Claude 把 `<task-notification>` 伪装成 user 消息再靠提示词教模型区分;agena 用 part 自身的 Assistant 角色,比 Claude 更干净、身份语义更真。

---

## 2. 核心数据模型:两个新增,都在 part 体系内

### 2.1 新 part kind:`system_notification`

`Part.kind` 是开放字符串列(types.rs:157),**不需要改 schema**。在 `crates/agena-runtime-contracts/src/part_content.rs` 增加:

```rust
/// `system_notification` — a background-operation completion notification
/// delivered to the model as an Assistant-role part appended to the
/// launching run (no new run). This is the agena analog of Claude Code's
/// `<task-notification>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SystemNotificationContent {
    /// The background operation id (task id or process id).
    #[serde(default)]
    pub operation_id: String,
    /// "task" | "shell" | "workflow".
    #[serde(default)]
    pub operation_kind: String,
    /// The launching tool_call's provider operation id (`agena.operation_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// "completed" | "failed" | "cancelled" | "timed_out".
    #[serde(default)]
    pub status: String,
    /// One-line summary, e.g. `Task "explore" finished`.
    #[serde(default)]
    pub summary: String,
    /// Optional structured detail (failure reason, exit code, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The body the model sees — mirrors Claude's
    /// `<note>…<result>…</result>…</note>` shape (see §3).
    #[serde(default)]
    pub body: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
```

配套改动(都是样板,照着 `NoticeContent`/`HookContent` 抄):
- `TypedContent` 枚举加 `SystemNotification(SystemNotificationContent)` 变体(part_content.rs:577-591);
- `decode()` 加 dispatch arm(:596-613);
- `typed_content_to_value` 加 arm(session/store.rs:656-677);
- `part_summary` 加 arm(session/store.rs:702-739);
- TUI 渲染加 arm(crates/agena-tui-transcript/src/parts.rs)。

### 2.2 通知的 part 身份:Assistant 角色,追加到启动 run(**不**开新 run)

> **2026-08-13 修正**(用户指示):"monitor 后台 shell agent 啥的都是 ai 去发起的,那么结果的 part 身份也应该是 ai 而不是 system,也不开新 run,只在前面的 run 上追加"。
>
> 早期设计的 `system_notification` run kind + System 角色(本行下方历史文字)已**废弃**。通知 part:
>
> - `kind` 仍是 `system_notification`(契约不变,§2.1);
> - **角色是 `PartRole::Assistant`** —— 后台操作由模型发起,结果属于模型自己的 turn;
> - **不开新 run marker** —— 通知作为内容 part 追加到启动 run(tool part 所在的 run marker)之下,`run_id = 启动 run`;
> - 存储层用新的 `settle_background_run` 原子完成:租约刷新(take-over 但不 abort 目标 run)、tool part 终态化、追加 Assistant 通知 part、无子 in-flight 时终态化 run marker —— **一步事务**,不触碰其它 in-flight run。

> 早期决策(已作废):~~`"system_notification" => PartRole::System` run-kind 映射 + 独立 System run~~。该方案让通知成为独立系统消息,但违背 "AI 发起 → AI 结果" 的身份语义,且多出一条 run 噪音。最终采用:通知以 Assistant 角色 append 进启动 run,投影成 `Role::Assistant` 消息的一部分(§3),唤醒逻辑只需"检测新通知 part → 重开模型 turn"(§4)。

---

## 3. 投影:通知如何变成模型看到的消息

`project_persisted`(wire_message.rs:147-271)加一个 arm,仿照 `Hook` arm(:253-265):

```rust
TypedContent::SystemNotification(notification) => {
    if !notification.body.trim().is_empty() {
        wire.push(WirePart::Text { text: notification.body.clone() });
    }
}
```

body 的文本形态(对齐 Claude 的 `<task-notification>` XML,让模型一眼可辨):

```
<agena_notification operation_id="task_abc" kind="task" status="completed">
Task "explore" finished

<result>
…最终文本(来自 child session 的 final_child_text / shell 的 exit text)…
</result>
</agena_notification>
```

- **角色投影**:通知 part 自带 `PartRole::Assistant`(`project_persisted` 用 part 自身角色,wire_message.rs:147-157),挂在启动 run marker 下;`project_completion_input` 读 marker 角色 → 整条 assistant run(含通知文本)以 `Role::Assistant` 消息进入模型 —— 后台结果属于模型自己的 turn,与 Claude 把 `<task-notification>` 伪装成 user 消息但语义上是"任务回来了"一致,而身份是 AI 的;
- `run_has_visible_prompt_payload`(prompt_window.rs:399-411)对非空 Text 天然放行,无需改动;
- **`all_parts_terminal` 约束**(processor/run.rs:621-668):通知 part 生来 `Completed`(终态),不产生 spinner;
- 通知不复制 child 全量历史——它通过 `operation_id` 引用,完整转录仍在 child session 的 parts 里(**everything is a part** 的可组合性,同 Claude 的独立 transcripts)。

---

## 4. 唤醒机制:统一 settle + 通知如何触发新 turn

这是本设计的心脏。旧的 `complete_background_operation`(只终态化 tool part)+ `notify_background_completion`(写独立 System run)两步已合并为**一个原子入口** `settle_background_operation`(manager/mod.rs):在一个 session 互斥 lane 里,终态化 tool part **并**把 Assistant 通知追加到启动 run,然后唤醒。

### 4.1 新管理器入口

```rust
/// Terminalize + append notification + wake, atomically.
pub async fn settle_background_operation(
    &self,
    session_id: i64,
    kind: &str, id: &str,
    terminal: PartState,
    outcome: Result<String, Failure>,
    notification: SystemNotificationContent,
) -> Result<(), AppError> {
    let committed = self.session_mutations.run(session_id, async {
        // 1. Dedup claim: the durable system_notification part (kind+id) is
        //    the claim (Claude I4e analog) — re-delivered signal is a no-op.
        if session.parts().iter().any(|p| part_records_notification(p, kind, id)) {
            return Ok(false);
        }
        // 2. Rewrite the launching tool part to its real final result (the
        //    old complete_background_operation body).
        // 3. run_id = the tool part's run (the assistant run that launched it).
        // 4. One atomic settle: lease refresh + tool-part terminalize + append
        //    the Assistant-role system_notification part onto the launching
        //    run + terminalize the run marker when no child is in-flight.
        self.store.settle_background_run(session_id, run_id,
            Some((tool_part_id, terminal, final_content)),
            vec![new_notification_part(notification, PartRole::Assistant)])?;
        Ok(true)
    }).await?;
    if !committed { return Ok(()); }

    // Wake the model over the settled result, per state:
    if self.execution_registry.is_active(session_id).await {
        self.steer_input(session_id, vec![TypedContent::SystemNotification(notification)]).await?;
        // drain_steer_input 是纯重触发:settle 已把通知 append,它只 reload
        // session;loop 的 newest_notification_part_id 检测到新通知 → 重开 turn。
    } else {
        self.execute_registered(
            session_id, ExecutionSource::User, ExecutionConversationTarget::NewTurn,
            "notification execution",
            move |manager, control, steer_rx| async move {
                manager.notification_run_inner(session_id, control, steer_rx).await
            },
        ).await?;
    }
    Ok(())
}
```

- **幂等 claim 迁移**:从"tool part metadata 上的 `agena.notified` 标志"改为**"已落库的 `system_notification` part 本身"**。理由:通知 part 是持久可观测物,检查它是否已存在即完成去重(§8 不变式 "一切皆 part");不必再写工具 part 的 metadata。`part_records_notification` 检查 `(operation_kind, operation_id)`。
- `notification_run_inner` 是 `submit_user_run_inner` 的瘦身版:跳过 `user.prompt.submit` 插件链(通知不是用户输入),直接 `run_until_stable`(通知 part 已在 store 里,loop 首轮即检测到)。
- 两条路径复用的正是**已验证的既有机制**:
  - idle:`execute_registered` 就是 `submit_subtask_user_message`(runs.rs:382-400)与 scheduler wake(builders.rs:427-449)的入口;
  - active:`steer_input`(history.rs:131)→ `drain_steer_input`(replies_state.rs:526-557)。
- `is_active`(execution_registry.rs:278)给出状态判定,避免 `AlreadyActive` 冲突。

### 4.2 `drain_steer_input` 增强 + turn 触发扩展

`drain_steer_input` 对 `TypedContent::SystemNotification` **不再写任何 run**:settle 已经落库。它只 reload session(把 append 的通知 part 收进投影),然后靠 `run_until_stable` 的新检测触发:

`run_until_stable` 的 turn 触发检测(`replies_execution.rs:363-372` 基于 `last_input_message_id`)**新增一个通知游标** `newest_notification_part_id`:

```rust
let latest_notification = newest_notification_part_id(session.parts());
if latest_notification != observed_notification_id {
    observed_notification_id = latest_notification;
    model_requested = true;      // 通知已在启动 run 里,模型响应开新 assistant run
    turn_run_id = None;
}
```

- 普通用户 steer 仍走 User run(`submit_user_run`),检测沿用 `last_input_message_id`(User marker);
- 通知游标独立于 user-message 游标:通知 part 不是 run marker,不能复用 marker 检测;新通知移动游标 → 重开 turn,模型响应是**新 assistant run**(响应属于新 turn,通知本身属于启动 run —— 语义干净)。
- **turn 身份**:通知不引入新 marker,`message_id_for_turn` / `user_message_id_for_turn`(history.rs:153)的 turn 身份函数**无需改动** —— 这是相对早期 System-run 方案的一大简化。

### 4.3 幂等 claim(对应 Claude 的 `I4e`)

后台完成信号可能重复触发(`SessionMetaUpdated` 每次 meta 变更都发;shell 的 `on_finished` 通常一次但保底)。**已落库的 `system_notification` part 就是 claim**:

- settle 事务开头检查 `part_records_notification(part, kind, id)` 是否已存在;
- 已存在 → 整个 settle 是 no-op(不重写 tool part、不追加、不唤醒);
- 同一条后台任务**只通知一次**。

> Monitor 事件是持续多条,claim 语义不同:按 `agena.monitor_event_seq` 单调递增,每条事件独立通知(见 §7.3)。

---

## 5. 接线点:谁调用 `settle_background_operation`

`BackgroundCompletionBridge` 的两个终态化路径,各自在终态化时调用统一的 `settle_background_operation`(**终态化 + 追加通知 + 唤醒一步到位**,不再分两步):

| 路径 | 现有终态化 | 统一入口 |
|---|---|---|
| shell | `complete_shell`(activity/state.rs:350-431)→ `settle_background_operation(session_id,"shell",&process_id,terminal,outcome,SystemNotification{operation_kind:"shell",status,summary:"Command finished (exit N)",body,…})` | 同函数(替换旧的 `complete_background_operation` + `bridge.notify_settled(...)` 组合) |
| task | `terminalize_task_part`(activity/state.rs:536-647)→ `settle_background_operation(parent_id,"task",&task_id,terminal,outcome,SystemNotification{operation_kind:"task",status:subtask_status,summary,body:<result>final_text</result>})` | 同函数(替换旧的 `complete_background_operation` + `notify_settled` 组合) |

- summary 文案对齐 Claude `M9s`/`M3t` 的 `H9s`:`Command "…" completed (exit code N)` / `Task "…" finished` / `failed: <reason>` / `cancelled`。
- 因为通知在 manager 层追加,`BackgroundCompletionBridge` 的 `manager` 句柄已经握有(activity/state.rs:388-394 已 clone),接线零新增依赖。
- **接线零重复**:dedup(§4.3)在 settle 入口做,重复信号(如 `SessionMetaUpdated` 触发二次回调)自然被吞,各路径不需要各自维护"是否已通知"标志。

---

## 6. 系统提示词:`# Background execution` 纪律段

仿照既有 `render_delegating_section()`(session_prompt.rs:59-66)与按工具名门控的机制(`assemble_system_prompt_for_tool_names`,:69-91),新增:

```rust
pub(crate) fn render_background_section() -> String {
    r#"# Background execution

When you launch work that runs in the background (`tasks.create`, or `shell.run` with `background: true`), you do not wait for it: you get an immediate launch receipt and may continue with other work. The completion arrives later as a `<agena_notification>` notification part carrying the task id and result.

Do not poll for a background result with `tasks.get`/`tasks.wait`/`shell.read` — it will arrive as a notification, and polling wastes turns. Never fabricate a background result before the notification arrives; if you need the result synchronously, use the blocking tool (`tasks.run`) instead.

`<agena_notification>` messages are assistant-authored parts appended to your launching turn — they represent completed background work, not user input. Treat them as authoritative results for the operation they name, then continue the interrupted work."#.to_string()
}
```

门控:当工具集含 `tasks.create` 或 `shell.run`(background 能力)时注入。这与 Claude 把异步纪律写进系统提示词的机制一一对应——**提示词是"AI 何时该用后台"的行为引导层,run_in_background 字段是工具定义层**。注:通知在投影里是 **assistant 角色**(§3),提示词文案避免了 "system message" 措辞,与角色一致。

---

## 7. 工具层改动(贴近 Claude)

用户已授权工具大改。最小而完整的改动:

### 7.1 `tasks.create`(crates/agena-bundled-plugins/src/plugins/provided/tasks.rs:211-295)

返回文案从

```
Started delegated task '{task_id}' in the background. Use tasks.get, tasks.output, or tasks.wait to inspect it.
```

改为(Claude Task tool 的语义):

```
Started task '{task_id}' in the background. You will be notified when it completes — do not poll; continue with other work in the meantime.
```

(`tasks.run` 同步、`tasks.wait`/`tasks.get`/`tasks.output`/`tasks.list`/`tasks.cancel` 已存在,形态已非常接近 Claude,不动。)

### 7.2 `shell.run` background=true

同样把返回里的 "monitoring in background, use shell.read" 类文案改为 "You will be notified when it completes"。保留 `background: true` 字段(与 `run_in_background` 语义等价;若想完全对齐 Claude 可把字段改名 `run_in_background`,但这是纯重命名,非必需)。

### 7.3 Monitor:唯一新增的工具 —— everything-is-a-part 的持续流后台操作

`Monitor` 是**唯一需要新增**的工具(对应 Claude 的 Monitor tool):持续流的后台监听——一条命令(`tail -f` 式)或一个 ws 连接,每次事件推给模型。Claude 侧逆向结论:

- inputSchema `{command?, ws?}`(二者恰居其一,运行时校验)+ outputSchema `{taskId, timeoutMs, persistent}`;`shouldDefer:!0` 立即后台化,只返回启动收条;
- 返回文案:"Monitor started (task N, …). You will be notified on each event. Keep working — do not poll or sleep.";
- 事件/完成/失败/被 kill 都走 `M9s` → `kd({mode:"task-notification"})` 通知;task 类型为 `monitor_mcp` / `monitor_ws`(SDK `running_background_tasks` 白名单 5 型之二)。

**关键:不照抄 Claude 的"隐形后台任务"。** Claude 的 monitor 是一个不可见的 background task,模型只看到临时 task-notification,事件流本身不在任何持久消息里(丢了就没了,只在 `running_background_tasks` 并行元数据里)。agena 的 everything-is-a-part 原则要求:**monitor 就是一个 part,它的每个事件也是一个 part**——Monitor 不是"仿 Claude 的新后台任务类型",而是"又一种后台操作,只不过它的完成是持续的"。

agena 落地方案(全部复用 §2-§5 的既有机制,零新通道;唯一新机制是"settle 但不终态化"):

1. **启动 = 一个 `tool_call` part**:`monitor.start` 调用 → 普通 tool part 盖 `agena.background {kind:"monitor", id: monitor_id}` marker → 保持 `InProgress`(monitor 活着 part 一直转)。这就是 monitor 的**持久身份**,与 shell/task 的后台 marker 完全同一套(`background_operation_from_execution` 认 `monitor.start` 的输出里的 `monitor_id`)。
2. **每个事件 = 一条 `system_notification` part**(`operation_kind:"monitor"`, `operation_id`, `monitor_event_seq:N`, `status:"event"`, summary 为一行):以 **Assistant 角色**追加到启动 run —— 与 shell/task 完成通知**同一个 settle + 唤醒机制**。事件也是 part,所以:
   - 全量事件历史**可回放、可 rewind、可压缩、可续跑**:重启后 parts 还在,模型能看到完整事件历史 —— Claude 的事件流不是消息,做不到;
   - dedup 的 claim 就是**已落库的 part 本身**(按 `kind+id+event_seq` 单调递增,§4.3 的终态 claim 是 `kind+id` 一次性的;事件 claim 是 `kind+id+seq` 单调的,每条事件独立通知、互不覆盖);
   - `/activities` 面板、TUI pill 都是从 part 状态**投影**的(§11 已有事件通道),monitor 没有游离于 part 之外的隐形状态。
3. **唯一新机制:`settle_background_event`** —— "settle 但不终态化":在 session 互斥 lane 里追加一条 `system_notification` 事件 part(走 §4 的租约安全事务,但**不**终态化 tool part、**不**终态化 run marker),然后唤醒模型(复用 `newest_notification_part_id` 重触发)。monitor 继续 in-flight。
4. **终止 = 复用 `settle_background_operation`**:stop(`monitor.stop` / `/activities` 面板 stop,`BackgroundActivity.cancellable`)→ kill 子进程/关 ws → `settle_background_operation(kind:"monitor", id, terminal, outcome, notification{status:"cancelled"/"completed"/"failed"})` —— 终态化 tool part + 追加终态通知 + 唤醒,与 shell/task 完成完全一致。
5. **运行时流缓冲(MonitorRegistry)只是"活投影"**:负责跑命令/收 ws、按 seq 编号、给面板提供实时快照;持久真相在 parts。
6. **防淹**:默认**逐条通知**(贴齐 Claude per-event,也是 everything-is-a-part 最直接的形态);高频场景的"静默期合并"降噪作为可选项(§10-5)——合并只是"几条事件拼进一条 part",**事件仍是 part**,不改变容器。

> 其余不新增:事件与完成都是系统侧注入、以 Assistant 角色追加到启动 run 的 part(§3),模型侧**不需要**新工具来收通知;`shell.logs`/`shell.list` 保留为"主动查流缓冲/审计"用,不再是轮询主通道(monitor 的事件已在转录里,连日志通道都不必为它新增)。

---

## 8. Everything-is-a-part 不变式校验

| 不变式 | 满足方式 |
|---|---|
| 一切持久可观测物都是 part | 通知是 `system_notification` part;结果在 tool_call part content;child 转录在 child session parts |
| 模型可见历史由 parts 派生 | 只新增一个投影 arm;通知按 part 自身角色投影(Assistant),挂在启动 run 下 |
| turn 由 run marker 驱动 | 通知**不开新 run**,追加到启动 run;`run_until_stable` 用独立通知游标 `newest_notification_part_id` 重触发新 turn(模型响应才是新 run) |
| 状态机统一 | 通知 part 终态生;`all_parts_terminal` 不阻塞;settle 事务刷新 lease 但**不 abort 其他 in-flight run** |
| TUI 渲染 parts | 新增一个渲染 arm(通知 chip,仿 Claude task-notification) |
| 无 schema 迁移 | kind 是开放字符串列;content 是 JSON blob(types.rs:155-180) |
| 幂等 | 落库的 `system_notification` part(按 kind+id)即 claim;重复信号被吞,同一条任务只通知一次 |

---

## 9. 实施步骤(按序,含文件锚点)

> **实施状态(2026-08-13)**:Assistant-角色 + 追加启动 run 的重构(**§2.2/§3/§4/§5/§8 对应设计**)已全部落地并通过 workspace 构建 + 全套测试(storage 44+52、runtime-session 130、runtime 65,含重写的通知幂等测试)。早期 System-run 方案(步骤 2/4 的 `submit_system_run` 与 `PartRole::System`)按用户指示**作废**,由 `settle_background_run` + `settle_background_operation` 取代。§11 输入区展示按用户指示**暂缓**(`先不管 输入区显示啥的了`)。**§7.3 Monitor 工具已按 everything-is-a-part 落地**:monitor 本体是带 `agena.background {kind:"monitor",id}` 标记的 `tool_call` part(启动后保持 `InProgress`,配一个空 `tool_result` 守卫 part 使稳定循环不会把它当待执行工具重跑),每个事件 = 一条 Assistant 角色 `system_notification` part(`monitor_event_seq` 单调,逐条 claim 幂等)经 `settle_background_event` 追加到启动 run("settle 但不定终态"),终止复用 `settle_background_operation`;`monitor.start/stop` 工具 + `agena.monitor` 插件 + `ToolPayloadOutput::Monitor` 均已注册,workspace 全量检查 + 全套测试通过(runtime-session 133、bundled-plugins 含 capability 身份快照)。

1. **契约层**:`SystemNotificationContent` + `TypedContent::SystemNotification` + `decode` arm
   - `crates/agena-runtime-contracts/src/part_content.rs`(:577-591 枚举, :596-613 decode)
   - `crates/agena-runtime-session/src/session/store.rs`(:656-677 `typed_content_to_value`, :702-739 `part_summary`)
2. **存储层 settle**(取代作废的 `submit_system_run` + `PartRole::System`):
   - `engine.rs` trait 加 `settle_background_run(session_id, owner_id, run_id, tool_part: Option<(i64, PartState, Value)>, new_parts, now_ms)`;
   - `agena-storage-sqlite/src/engine.rs:1171-1346` 与 `store/in_memory.rs:989-1102` 实现:刷新 lease(own/stale/None 均不 abort 其他 in-flight run)→ 终态化 tool part → 追加 Assistant 通知 part(挂启动 run)→ 无 in-flight child 时终态化 run marker;
   - `store/facade.rs:1169-1224` 门面:仅清 tool part 的流式 buffer(`clear_streaming_part`,绝不 `clear_streaming_session` 以免吞并发流)。
3. **投影**:`wire_message.rs` `project_persisted` 加 arm(:253-265 Hook 为模板);通知 part 自带 `PartRole::Assistant`,挂在启动 run 下。
4. **唤醒**(取代作废的 System-run steer):
   - `manager/mod.rs:1707` 起合并为统一 `settle_background_operation`(session 互斥 lane 内 settle + dedup,然后按 idle/active 唤醒);
   - `replies_state.rs:526-598` `drain_steer_input` 对 `SystemNotification` 只 reload session(settle 已落库),不再写 run;
   - `replies_execution.rs` `run_until_stable` 加 `newest_notification_part_id` 游标,新通知重触发 fresh model turn;
   - `runs.rs` `notification_run_inner`(idle 唤醒用,load 即见已落库通知)。
5. **接线**:`activity/state.rs` `complete_shell`(:350-431)与 `terminalize_task_part`(:536-647)各自调用 `settle_background_operation`(替换旧的 `complete_background_operation` + `notify_settled`);dedup 由入口的 `part_records_notification` 承担。
6. **提示词**:`session_prompt.rs` 加 `render_background_section()`,按背景工具门控注入(:69-91)。(文案里"system message"措辞改为"通知消息"——通知在投影里以 assistant 角色呈现,见 §3。)
7. **工具文案**:`tasks.rs:287-294` 与 `shell.run` background 返回改 "You will be notified… Do not poll";**新增 Monitor 工具**(§7.3,`BackgroundOperationKind::Monitor` + 输入/输出 schema + 子 session 事件桥)。
8. **TUI 输入区(§11)**:
   - `app_backend/live_events.rs:192-208` 在 `ActivityChanged` 分支更新 `background_activity_summary`(事件驱动,替代 10s 轮询 —— 这是 Claude `background_tasks_changed` 的 level-signal 对位);
   - `view_main.rs` `composer_background_activity_part`(:669)从 `● {count} background` 升级为按 `BackgroundActivity` 渲染的 pill 列表;
   - `crates/agena-tui-transcript/src/parts.rs` 渲染 `system_notification` part 的转录内通知 chip(仿 Claude task-notification,§8 不变式第 5 行)。
9. **测试**(已落地):
   - 后台完成 → Assistant 通知 part 落库(挂启动 run)+ 会话被唤醒(idle 与 mid-turn 两态);
   - 通知角色 == `PartRole::Assistant`;通知 `run_id == Some(launching_run_id)`;启动 marker 终态化;
   - 重复 settle 幂等(同一 kind+id 只落一条通知);
   - 通知不阻塞 `all_parts_terminal`;
   - turn 身份/rewind 在通知出现后仍正确(通知不开 run,身份函数零改动);
   - Monitor(已落地):`monitor.start` 返回的 `monitor_id` 经 `background_operation_from_execution` 标记背景操作;事件逐条通知(每事件一条 `system_notification`,`monitor_event_seq` 单调,重复信号按 seq 幂等)、事件不终态化 tool part/run marker、cancel → 终态通知复用 `settle_background_operation`、`monitor.stop` 不标记背景操作;
   - TUI(暂缓):后台成员变化**事件驱动**更新输入区 pill(无 10s 轮询延迟)。

---

## 11. TUI 输入区展示:Claude 的真相与 agena 的贴齐方案

> 用户关切:"输入框附近的显示"。"我们的 part 啥的还是 everything is a part 的良好设计就可以了" —— 本节**只动 TUI 展示层,零 part 模型改动**。

### 11.1 逆向结论:Claude 输入区到底显示什么

| 现象 | 逆向结论(2.1.229) |
|---|---|
| `#[warning:◐] agent-1 · scanning api` 这类 pill | **只是 onboarding "powerup" 课程模板示例帧**(语料里的 `P6_` statusline-setup 提示词与 `#[` 语法教学),**不是内部机制**。 |
| 真实的输入区状态 | **用户配置的 statusLine shell 命令**:stdout 成为 `statusLineText` store 状态,由 `_HT` 组件渲染在输入框附近。statusline 的 JSON stdin 上下文(`gHT`)字段为 `{session_id, cwd, model, workspace, tool, output_style, session_start_unix, ...}` —— **没有 backgroundTasks 字段**,statusline 命令**拿不到**后台任务数据。 |
| 输入区**不**显示 live 后台任务 | 后台任务集合只经 `background_tasks_changed`(REPLACE 语义)系统事件流动,由 `/tasks` 面板与 SDK 元数据(`running_background_tasks`,白名单 5 型 `local_bash/monitor_mcp/monitor_ws/local_agent/local_workflow`)消费,statusline 与输入区**不显示**它。 |

一句话:**Claude 输入区并没有"实时后台任务列表"**。它只有 (a) 用户自配的 statusline(命令上下文不含后台数据),以及 (b) `/tasks` 面板。`#[warning:◐]` 那套花哨 pill 只存在于教学示例里。

### 11.2 agena 现状盘点(输入区四角)

`view_main.rs` `render_composer`(:426):状态 chip 活在 composer 边框行上 —— 主状态左上、history/approval 右上、**背景活动左下**、plan 进度右下。

| 现有元素 | 位置 | 机制 | 与 Claude 对位 |
|---|---|---|---|
| `composer_status_parts`(:609) | 左下 | `status_line.rs` 纯呈现 + `run_status_line_command`(:2-17 `StatusLineUpdated`) | Claude statusline(用户自配命令,含插件 `StatusLineText`/`FooterBlock` 贡献) |
| `composer_background_activity_part`(:669) | **左下** | `background_activity_summary` → `● {count} background`;来源 `refresh_background_activity_summary_if_due`(**10s 轮询** `list_activities` active_only)→ `BackgroundActivitySummaryLoaded`(dispatch.rs:5) | 无直接对位 —— 这是 agena 独有的实时后台计数,Claude 输入区没有 |
| `/activities` 面板 | 独立视图 | `list_activities`/`activities_row_from_resource`(:680) | Claude `/tasks` 面板(消费 `background_tasks_changed`) |
| footer 状态行 | 底部 | status_line 文本 + plugin FooterBlock + banner 覆盖 | Claude statuslineText(另有 `#[warning]` banner 覆盖) |
| 事件通道 | — | `RuntimeLiveSignal::Activity`(live_signal.rs)→ `live_events.rs:192-208` `ActivityChanged` → `transcript_state.rs:221` 失效投影重载 | `background_tasks_changed`(REPLACE) —— **通道已存在,但没接进 chip** |

**关键差距**:`RuntimeLiveSignal::Activity` 事件通道**已经存在**且每笔 `BackgroundActivityChangedEvent{activity_id, reason, ts_ms}` 都带 reason(启动/完成/失败/取消……),但 `● {count} background` chip 只被 10s 轮询刷新 —— 事件到达时 chip 最多滞后 10s,且轮询 `active_only` 只给 count,不给成员明细。

### 11.3 贴齐设计(纯 TUI,事件驱动)

1. **chip 改事件驱动**:`live_events.rs:192-208` 的 `ActivityChanged` 分支(已对选中会话/祖先生效)直接维护 `background_activity_summary` 与一个成员 map `background_activities: BTreeMap<id, (title, kind, status)>`,发 `AppMessage::BackgroundActivitySummaryLoaded`(或新 `ActivitiesMembershipChanged`)给 `dispatch.rs`。**保留 10s 轮询作冷启动/兜底**,事件到达即覆盖 —— 事件驱动是主路径,轮询只是兜底。这就是 Claude `background_tasks_changed` REPLACE 语义的 level-signal 对位。
2. **chip 从计数升级为 pill 列表**:`composer_background_activity_part` 按 `BackgroundActivity.title/description/status` 渲染,贴齐 Claude 教学示例的 pill 语法(仅借其视觉语言,非其机制):
   - 运行中:`◐ shell · tail api.log` / `◐ task · explore` / `◐ monitor · ws://…`(◐ spinner);
   - 成功:`✓ task · explore · found reject`;
   - 失败:`[warning:✕] task · crawl · exit 1`;
   - 可选按 kind 配图标:`shell ⌁`、`task ▸`、`monitor 👁`、`browser 🌐`。
3. **statusline 命令上下文(可选增强,agena 对 Claude 的**改善**)**:`run_status_line_command` 的 stdin JSON 增加 `background_tasks: [{task_id, kind, title, status}]` 字段。Claude 的 statusline **拿不到**后台数据(设计缺陷);agena 事件通道现成,把成员快照喂给自配 statusline 命令,让用户自写 `jq`/`grep` 决定显示什么 —— 这是贴齐 Claude 的同时做得更好的一点。
4. **转录内通知 chip**:`system_notification` part 在转录里渲染为一次性的通知行(§9-8 第二项),输入区 pill 只管"活着/刚结束"的成员,pill 消失后完整通知仍在转录里可查(一切皆 part,可回溯)。

### 11.4 不变式校验(§8 延伸)

- 输入区 pill **不是 part** —— 它只是 `BackgroundActivity` 资源(持久于 background_activities 表)的投影,part 模型零改动;
- 转录内通知 chip 是 `system_notification` part 的渲染 arm(§8 第 5 行既定);
- 事件驱动后,10s 轮询降级为兜底,不删(冷启动时序:live 通道在 connect 前的事件由首轮 poll 补齐)。

---

## 10. 待你拍板的决策点

1. ~~System-role 注入 vs user 伪装注入~~ —— **已定**(2026-08-13,用户指示):**Assistant-role 追加到启动 run**(§2.2),不再开 run。Claude 的 user 伪装与早期 System-run 方案均已否决。
2. **新 kind `system_notification`(推荐)** vs 扩展现有 `notice` —— 我推荐前者:projection 规则简单(notice 目前投影为空,加条件分支有泄漏风险),kind 是开放字符串列,加一个零成本。
3. **`shell.run` 字段改名 `run_in_background`(可选)** —— 纯对齐 Claude 的重命名,非必需。
4. 通知的**唤醒是否带优先级**(严格"下一条输入" vs 普通排队)—— 现有 steer/submit 已是 next-turn 语义,默认不加优先级通道。
5. **输入区 pill 粒度**:`● {count} background`(现状,保留计数)vs **per-task pill 列表**(§11.3-2,贴齐 Claude 教学示例的视觉)—— 我推荐 per-task pill:事件通道已带成员明细,成本低,信息量大;计数作为折叠态。
6. **statusline 命令上下文是否喂后台数据**(§11.3-3,agena 对 Claude 的改善)—— 我推荐喂:`background_tasks` 字段可选(`skip_serializing_if`),不破坏既有 statusline 命令。
7. **Monitor 事件粒度**:逐条通知(贴齐 Claude `M9s`,推荐)vs 静默期合并降噪 —— 默认逐条;高频场景再议合并策略。
