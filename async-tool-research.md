# 后台工具执行(Async Tool Execution)调研报告

> 调研日期:2026-08-13 · worktree:`async-tool-research`
> 对象:① OpenAI Codex CLI(开源,`../codex/codex-rs`)、② 本机 Claude Code v2.1.229(Bun 单文件 Mach-O,复用 `claude-reverse/` 逆向成果 + 本二进制字符串交叉验证)、③ agena 现状(用于对照落地)。
> 问题:它们如何让 AI 调用工具后"不等待工具结束就继续工作",又如何让 AI "知道工具何时结束"(task/subagent 等)?系统提示词/工具定义是如何指导 AI 恰好在需要时把工具放到后台跑的?
>
> **融合落地方案见 [`fusion-design.md`](./fusion-design.md)**(通知即 part / part 即通知;新增 `system_notification` part kind 与 System run kind,`notify_background_completion` 唤醒会话,`# Background execution` 提示词纪律段,`tasks.create`/`shell.run` 文案对齐 Claude)。运行时逆向细节见 [`claude-async-runtime-notes.md`](./claude-async-runtime-notes.md)。

---

## 一、核心结论(先读这个)

两个参考实现采用了**两种互补的异步策略**,而不是单一机制:

| 策略 | Claude Code | Codex | agena 现状 |
|---|---|---|---|
| **A. 协程式让出 + 句柄轮询/被通知**<br>(工具本身跑在后台,先给句柄,完成后被唤醒) | `Bash.run_in_background` / `PowerShell`(后台进程 + `<task-notification>` 唤醒) | `exec_command(yield_time_ms)` → `session_id`,`write_stdin` 空轮询 + `ExecCommandEnd` 事件 | `shell.run(background=true)`(有句柄/日志/stop,但**模型不会被通知**,只能主动 `shell.logs`) |
| **B. 真并发子代理**<br>(每个子任务独立 session/tokio task,完成后把结果注入父会话) | `Agent`(别名 `Task`),默认就是后台,`run_in_background:false` 才同步 | `spawn_agent`(返回 task name)→ 后台跑,完成时 completion watcher 注入消息;`wait_agent` 才阻塞 | `agena.tasks.create`(后台子任务)vs `agena.tasks.run`(同步);`message/output/cancel/wait` |
| **C. 一个响应内并行工具调用** | 系统提示词鼓励独立工具并行调用 | `FuturesOrdered` + `parallel_tool_calls` | `parallel_tool_calls` 契约 + `concurrency_safe` tag,已有 `execute_pending_tools_concurrently` |
| **D. 沙箱内宿主语言长任务**(进阶) | — | code-mode:JS 跑在 V8 isolate,`yield_control()`/`notify()`/`wait` 恢复 cell | — |

**最关键的洞察**:Claude Code 让"工具在后台跑 + AI 继续干别的"成立的关键不是"如何跑后台进程"(进程管理到处都是),而是:

1. **工具调用立刻返回一个"句柄型"的 `tool_result`**(`backgroundTaskId` + 输出文件路径 + "You will be notified when it completes"),这样模型的**当前 turn 能正常结束**,不会被 pending tool 卡住;
2. **后台任务完成时,harness 把一条 `<task-notification>` 当作系统消息重新注入对话**,唤醒一个新的 turn 把结果带给模型——这就是"AI 知道工具结束了";
3. **系统提示词 + 工具描述反复教育模型三条纪律**:不轮询(等通知)、不伪造结果(通知不是自己写的)、需要立即结果才用同步(否则默认后台)。
4. **harness 层强制**(validateInput 拦截前台 sleep 轮询),把"异步契约"从软建议变成硬约束。

下面分别展开。

---

## 二、Claude Code 的运行时机制(逆向 v2.1.223/229)

### 2.1 两种后台面的共同基础设施

**Task Registry + In-flight 追踪**。逆向到核心异步 agent 运行函数 `iMe({taskId, abortController, makeStream, ...})`(见 `claude-reverse/raw/region_agent.txt`):
- 每个 agent/task 在 `taskRegistry` 中登记,状态机:`running → completed | failed | killed`,字段含 `notified`、`quietlyParked`、`keepaliveReasons`、`endTime`、`evictAfter`。
- 每个 agent 维护 `inProgressToolUseIDs`(Set):`assistant` 消息里的 `tool_use` 加入,`user` 消息里的 `tool_result` 移除。`isIdle` = `tool_result 已到数量 == tool_use 已发数量`。
- **`background_tasks_changed`** system 消息子类型(池中原文):*"The full set of live background tasks, emitted whenever membership changes (start, completion, kill, a foreground agent being backgrounded). A level signal... REPLACE semantics: swap your set for this payload."* —— 这就是 harness 内部"知道还有没有后台活"的信号。

**Stall watchdog**:`CLAUDE_ASYNC_AGENT_STALL_TIMEOUT_MS` 默认 600_000ms(10 分钟)。异步 agent 超过该时长无进展(`last message type` 不变、无 in-flight tool)就 abort 并标记失败——防止后台 agent 静默挂死。(agena 内存里有对应经验:`lease-staleness-streaming-abort`。)

**Headless / `-p` 模式的退化**:*"There is no task-notification re-invocation in headless mode, so a backgrounded run is never resumed."* —— 没有重注入能力就没有后台,这是"能不能后台"的充分必要判据。

### 2.2 Bash 的 run_in_background(策略 A)

**工具定义**(`claude-reverse/export/tools-raw/Bash.txt`)里 `mapToolResultToToolResultBlockParam` 在 `backgroundTaskId` 存在时拼出:
```
Command running in background with ID: <id>. Output is being written to: <path>.
You will be notified when it completes. To check interim output, use Read on that file path.
```
(超时自动转后台时文案是:`Command did not complete within its Ns timeout and was moved to the background (ID: ...)`。)

→ 模型当前 turn 立刻拿到这个 tool_result,**turn 正常结束**;命令继续 detached 跑,输出落盘。

**完成唤醒**:进程退出 → 任务 settle → harness 注入一条 `<task-notification>`(见 §2.4),重新唤起模型。

**硬性契约强制**——`validateInput`(Bash.txt 中):
```
Blocked: <sleep-ish command>. To wait for a condition, use Monitor with an until-loop
(e.g. `until <check>; do sleep 2; done`). To wait for a command you started,
use run_in_background: true. Do not chain shorter sleeps to work around this block.
```
即:**在前台轮询等待被直接拦截**,必须走后台+通知或 Monitor。这是"教育"之外的行为兜底。

**Monitor 工具**(独立后台面,事件流):*"Start a background monitor that streams events from a long-running script. Each stdout line is an event — you keep working and notifications arrive in the chat. Events arrive on their own schedule and are not replies from the user..."* —— 明确告诉模型:异步事件会作为"聊天里的通知"到达,不是用户消息。

### 2.3 Agent(别名 Task)的后台子代理(策略 B)

**工具定义**:主名 `Agent`,别名 `Task`;`inputSchema` 里 `run_in_background` 的 describe:
```
Agents run in the background by default; you will be notified when one completes.
Set to false to run this agent synchronously when you need its result before continuing.
```
**默认后台**!`outputSchema` 为 `async_launched`:`{ status, agentId, description, resolvedModel, modelsUsed, outputFile, canReadOutputFile }`。

**运行时**(`region_agent.txt` 的 `iMe` + Agent tool `call`):
- `call` 里 `tokio`/JS 侧:若 `run_in_background`(默认)→ 立即返回 `async_launched`;否则同步 `await`。
- 后台 agent 由 `makeStream` 建独立 stream,`for await` 消费;父会话不受阻塞。
- 完成:stream 结束 → `M3t({taskId, status:"completed", finalMessage, usage, toolUseId, ownerAgentId})` 标记任务完成并唤醒 owner。
- owner 唤醒后,**子代理的 final message 被注入父会话**(通知里含 result),模型直接读到结论。
- `isAsync:true` / `isBackgroundAgent:true` 贯穿 metadata。

### 2.4 `<task-notification>` 如何让 AI 知道"工具结束了"(核心机制)

逆向到注入的消息体是 **`<task-notification>` 包裹的 XML**,作为一条"像用户消息但不是用户消息"的系统侧输入:
```
<task-notification>
  <task-id>...</task-id>
  <tool-use-id>...</tool-use-id>
  <output-file>...</output-file>
  <status>completed</status>
  <summary>...</summary>
  <result>...</result>
</task-notification>
```
(本会话里我自己后台跑的 Monitor/Agent 完成时,注入的就是这条——`<task-id>` 与我拿到句柄时的 id 对应。**这是本报告最直接的实证。**)

系统提示词里有一整段教模型识别并正确对待它(池中原文摘录):
- *"They look like user messages but are not. Distinguish them by the `<task-notification>` opening tag."*
- *"A task-notification fires each time this agent stops with no live background children of its own. The user can send it another message and resume it, so the same task-id may notify more than once."*
- *"Never fabricate or predict a pending agent's results — the notification is never something you write yourself; if the user asks before it arrives, say it's still running."*
- *"If waiting for a background task you started with `run_in_background`, you will be notified when it completes — do not poll."*
- *"For 'tell me when X is ready', use Bash with `run_in_background` ... You get a single completion notification when it exits."* (Monitor 描述里的决策指南)

**这就是"AI 知道工具结束"的全部秘密**:不是轮询,不是心跳,而是 **harness 在任务 settle 后主动往对话里注入一条带结果的通知**,重新触发一个 turn。

---

## 三、Codex 的运行时机制(开源,Rust)

### 3.1 四个协同机制

**① 协程式让出 + 句柄 + 轮询/事件(exec_command)** —— 对应策略 A:
- `core/src/tools/handlers/unified_exec/exec_command.rs:107` → `UnifiedExecProcessManager::exec_command`(`core/src/unified_exec/process_manager.rs:417-663`):
  - 起 PTY 进程后立刻 `tokio::spawn` 两个后台任务:`start_streaming_output`(`async_watcher.rs:44-154`,持续转发 stdout 增量)和 `spawn_exit_watcher`(`async_watcher.rs:159-221`,进程退出时发 **`ExecCommandEnd`**)。
  - 主调用只等 `yield_time_ms`(默认 10s,`collect_output_until_deadline`,process_manager.rs:495-505)就返回,进程还活着则返回 `session_id`。
- 模型继续干别的,之后用 `write_stdin {session_id, chars:""}` 空轮询(`write_stdin.rs:52-94`,空轮询默认等 5-300s)。
- **完成事件** `ExecCommandEnd` → `ToolEmitter::unified_exec(...).emit(Success)` → `emit_exec_end`(`core/src/tools/events.rs:564-593`)→ `TurnItem::CommandExecution` 落地到会话历史 → **下一个模型请求天然看到它**。与 Claude Code 不同:Codex 是"事件进历史,随下轮请求自然可见",而非独立注入一条通知消息。

**② 真并发子代理(spawn_agent/wait_agent)** —— 对应策略 B:
- `core/src/tools/handlers/multi_agents_v2/spawn.rs:39-181`:`spawn_agent` 立即返回 task name/nickname。
- 子代理 = 独立 `Session` + `tokio::spawn` 的 `RegularTask`(`core/src/tasks/mod.rs:276-427`)。
- **完成 watcher**:`AgentControl::maybe_start_completion_watcher`(`core/src/agent/control.rs:459-547`)`tokio::spawn` 订阅子代理 status 直到 `is_final`,然后向父线程注入结果(v2 走 `InterAgentCommunication` 消息 / v1 `inject_user_message_without_turn`)。
- `wait_agent`(`multi_agents_v2/wait.rs:37-196`)才是显式阻塞,用 watch-channel 等到超时。

**③ code-mode(沙箱内宿主语言长任务)** —— 进阶策略 D:
- `core/src/tools/code_mode/execute_handler.rs:30-115`:JS 跑在独立 `codex-code-mode-host` 进程的 V8 isolate;`yield_control()` 让出当前输出同时脚本继续跑;`notify(value)` 即时注入 `custom_tool_call_output`;`wait` 恢复 cell。
- 这让模型能写"并发编排脚本"(多个工具 Promise 并发),是最高级的异步。

**④ 单响应内并行工具**:`try_run_sampling_request` 把每个 tool call 变成 future 塞进 `FuturesOrdered`(`turn.rs:2190,2352-2354`),`drain_in_flight` 统一 await。

### 3.2 Codex 的工具定义如何指导异步

工具描述自带"什么时候后台、什么时候同步"的决策规则(`core/src/tools/handlers/multi_agents_spec.rs`):
- *"While the subagent is running in the background, do meaningful non-overlapping work immediately."*
- *"Call wait_agent very sparingly. Only call wait_agent when you need the result immediately for the next critical-path step and you are blocked until it returns."*
- *"Do not repeatedly wait by reflex."*
- *"Prefer delegating concrete, bounded sidecar tasks that materially advance the main task without blocking your immediate next local step."*
- *"Do not delegate urgent blocking work when your immediate next step depends on that result."*

`exec_command` 描述是 *"Runs a command in a PTY, returning output or a session ID for ongoing interaction."*;`write_stdin` 的 `chars` 参数明确 *"Defaults to empty, which polls without writing."* —— 把"轮询"做成显式工具语义,而不是让模型自己 `sleep`。

对比:**Codex 把 async 指导全部塞进工具描述**(没有大段系统提示词);**Claude Code 同时用工具描述 + 系统提示词 + harness 强制拦截**三层。

---

## 四、对 agena 的启示(对照现状的差距)

agena 现状(见 Explore 报告,文件路径略去前缀,均在 worktree 当前 commit `1a9ca8f8` 内):

- **已有 70% 的基础设施**:
  - `agena.tasks.create`(后台子任务,立即返回)vs `agena.tasks.run`(同步)——策略 B 已有。
  - `shell.run(background=true|monitor)` + `shell.list/logs/stop`——策略 A 的"句柄"已有。
  - `BackgroundOperation{kind,id}` marker + in-progress 部分 + 空 `tool_result` guard + `BackgroundCompletionBridge`(`crates/agena-runtime/src/activity/state.rs:294-570`)把后台完成**异步终态化**到 transcript 部分。
  - `parallel_tool_calls` + `execute_pending_tools_concurrently`——策略 C 已有。

- **关键差距(按重要性)**:
  1. **模型不会被通知**。后台任务完成只是"部分被终态化 + 面板隐藏",**没有把一条"通知消息"注入对话重新唤起模型 turn**。模型若想拿到结果,只能主动 `tasks.wait` / `shell.logs`(轮询)。→ 缺 Claude Code 的 `<task-notification>` 注入,或 Codex 的 `TurnItem` 完成事件入历史。
  2. **无系统提示词层的异步纪律**。没有"你会在后台任务完成时被通知,不要轮询 / 需要立即结果才用同步 / 不要伪造通知"这类指导(agena 的 delegating section 只有一句 "wait for the result")。
  3. **无 harness 强制**(例如前台 sleep 轮询拦截),所以模型即使被教了也可能退化成 sleep-轮询。
  4. `background_operation_from_execution`(`crates/agena-runtime-session/src/session/manager/helpers.rs:186-230`)目前**硬编码只认两种** kind(`shell`、`task`)——新后台工具要扩展这个识别点 + 桥接器。

**落地建议的最小闭环**(把"模型不等待 + 知道何时结束"补全):
1. 后台 launch 返回句柄型 tool_result(已有,保留空 guard)。
2. 新增**完成通知注入**:`BackgroundCompletionBridge` 终态化时,向父会话 `submit` 一条 `system`/`tool_result` 风格的通知消息(带 `task-id`/`kind`/`result` 摘要),并触发一次新的模型 turn(类似 `SessionManager::complete_background_operation` 之后 `submit_user_message`)。
3. 系统提示词新增一节 "Background execution"(复用 Claude Code 措辞:被通知/不轮询/不伪造/需要立即结果才同步/headless 或不可重注入时退化同步)。
4. 工具描述补 `run_in_background` 式字段与"你会在完成时被通知"的说明。
5. (可选强化)校验层拦截前台 `sleep` 轮询,强制走后台。

---

## 五、参考文件索引

**Claude Code(逆向)**
- 工具定义全文:`claude-reverse/export/tools.md`、`tools-raw/Bash.txt`、`tools-raw/Agent(Task)…`(Agent 在 corpus 中,`region_agent.txt`)
- 异步 agent 运行时:`claude-reverse/raw/region_agent.txt`(函数 `iMe`、`M3t`、`MQ`/`Nge`)
- 系统提示词全文:`claude-reverse/export/runtime-main-prompt.md`、`system-prompt-full.md`
- 通知/后台文案语料:`claude-reverse/export/corpus-all-literals.txt`(grep `task-notification` / `run_in_background` / `background_tasks_changed`)

**Codex(开源)**
- 进程/PTY/后台:`codex-rs/core/src/unified_exec/{process_manager,async_watcher,process}.rs`
- 子代理:`codex-rs/core/src/agent/control.rs`、`agent/control/spawn.rs`、`tools/handlers/multi_agents_v2/`
- 工具描述:`codex-rs/core/src/tools/handlers/{shell_spec,multi_agents_spec}.rs`、`code-mode-protocol/src/description.rs`
- 会话/turn 循环:`codex-rs/core/src/session/turn.rs`、`session/inject.rs`、`tasks/mod.rs`

**agena**
- 后台桥:`crates/agena-runtime/src/activity/state.rs:294-570`
- 后台 launch 检测:`crates/agena-runtime-session/src/session/manager/helpers.rs:186-230`
- 终态化:`crates/agena-runtime-session/src/session/manager/mod.rs:1702`
- 系统提示词:`crates/agena-runtime-contracts/src/identity/mod.rs`、`crates/agena-runtime-session/src/session/manager/session_prompt.rs`
- turn 循环:`crates/agena-runtime-session/src/session/manager/replies/replies_execution.rs:257-865`
