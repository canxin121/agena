# Claude Code 异步机制深挖(运行时实现级)

> 逆向对象:本机 `~/.local/share/claude/versions/2.1.229`(Bun 单文件,Mach-O arm64),复用 `claude-reverse/export/corpus-all-literals.txt`(21375 条字符串/内联代码语料)与 `raw/region_agent.txt`。以下是被提取到的**运行时实现代码**(不是提示词文本),全部来自二进制内嵌 JS 源码区。

## 1. 任务注册表(Task Registry)核心

`f3(e,t)` 返回注册表对象(键:taskId → task 记录),能力:
- `register(o)` → 发 `rT({type:"system", subtype:"task_started", task_id, tool_use_id, description, subagent_type, task_type, ...})`
- `update(id, fn)` → 状态变化时 `zyy` 计算 diff,发 `rT({type:"system", subtype:"task_updated", task_id, patch})`
- `updateTranscript(id, fn)` → 独立 transcript(每 task 一套消息)
- `getTranscript/get/remove/evictTerminal/applyOffsetsAndEvict`
- `incrementTotalAgentSpawns / takeConcurrencySlot`(并发配额)
- `evictTerminal`(`Yyy`):终态且已 notified 且非 retain 且非 keepalive 的 task 被移出

task 记录形态(以 `local_agent` 为例,`q4e` 构造):
```
{type:"local_agent", status:"running", agentId, ownerAgentId, parentAgentId,
 spawnDepth, prompt, model, effort, cwd, selectedAgent, agentType,
 abortController, isBackgrounded, isIdle:false, pendingMessages:[],
 retain:false, diskLoaded:false, keepaliveReasons:Set}
```
关键标志:**`notified`**(是否已通知 owner)、**`isIdle`**(in-flight tool 是否清空)、**`keepaliveReasons`**(保持不被 evict 的原因)、**`quietlyParked`**。

## 2. 通知投递链(核心中的核心)

### 完成时(agent):`M3t({taskId, description, status, killedBy, error, taskRegistry, finalMessage, usage, toolUseId, worktreePath, worktreeBranch, ownerAgentId})`
```js
let {claimed, task} = I4e(e, i)          // ① 原子 claim,防止重复通知
let g = hUs({ownerAgentId:h, keepaliveReason:`agent:${e}`, delivering:claimed, taskRegistry:i}) // ② 决定通知发给谁
if (!claimed) return                     // 已通知过 → 跳过
let y = r==="completed" ? "finished" : r==="failed" ? `failed: ${o}` : ...被谁停止
let b = `${qYn}${description}" ${y}"`    // summary 文案
let T = finalMessage ? `<result>${pl(finalMessage)}</result>` : ""     // ③ 注入最终结果
let S = usage ? `<usage>...</usage>` : ""
let w = worktreePath ? `<worktree>...</worktree>` : ""
kd({ value: u8({taskId, toolUseId, outputFile:jb(taskId), status, summary, body:`<note>...${T}${S}${w}`}),
     mode:"task-notification", priority:"next", agentId:g, taskId })
```

### 完成时(shell/monitor):`M9s(taskId, status, exitCode, ...)`
```js
if (!I4e(taskId, registry).claimed) return
let c = H9s(s, t, r, n, l)               // summary:"Command \"x\" completed (exit code N)" 等
kd({ value: u8({taskId, toolUseId, outputFile:jb(taskId), status, summary}),
     mode:"task-notification", priority:"next", agentId:owner })
```

### 原子 claim:`I4e(taskId, registry)`
```js
function I4e(e, t){let r=!1,n;
  return t.update(e, o => { if(o.notified) return o; r=!0; return {...o, notified:!0} }),
         {claimed:r, task:n} }
```
→ **同一条后台任务只会通知一次**,靠 `notified` 标志原子置位。

### XML 构造:`u8({taskId, toolUseId, taskType, outputFile, status, summary, body, trailing})`
按字段序列 `<task-id>`, `<tool-use-id>`, `<task-type>`, `<output-file>`, `<status>`, `<summary>`, + body/note + result/usage/worktree。

### 归属解析:`hUs({ownerAgentId, keepaliveReason, delivering, taskRegistry})`
```js
let o = ownerAgentId ? registry.get(ownerAgentId) : undefined
let s = isAgent(o) && onKeepalive(o) && !isHeadless()  ||  isAgent(o) && o.status==="running"
if (!(delivering && s)) zj(ownerAgentId, keepaliveReason, registry)  // 给 owner 加 keepalive 防 evict
return s && ownerAgentId ? agentId(ownerAgentId) : mainSessionId()   // 通知发给 owner,否则主会话
```

### 转发:`TMo(agentId, registry)`
owner 空闲(keepalive 且非 headless)时,把排队中发给 owner 的通知改投主会话 `Pi()`。

## 3. 通知如何唤醒模型

- `kd({value, mode:"task-notification", priority:"next", agentId})` 把 `<task-notification>` XML 作为一条**高优先级(priority:"next")消息**投进目标 agent 的消息队列,`origin.kind==="task-notification"`。
- 投递后 agent 被唤醒 → 新 turn → `<task-notification>` 以 user 消息形态进入上下文,系统提示词明确告诉模型:*"They look like user messages but are not. Distinguish them by the `<task-notification>` opening tag."*
- `Zxd`(agent observer)显式跳过 `origin.kind==="task-notification"` 的消息做触发统计 → 通知不会污染 observer 的因果链。

## 4. keepalive / evict 生命线

- `zj(agentId, reason, registry)` 加 reason 到 `keepaliveReasons`;`Bce` 同。空集且终态且非 retain → 设 `evictAfter = now + 30s`。
- `Yyy`(evictTerminal):终态 + notified + 无 retain + 无 keepalive → 移出注册表并 `Cbr.emit`(总线通知)。
- `Qyy`:agent 完成且有 keepalive 且已 notified → 若 owner 无同 taskId 的排队通知,则 `zj(owner, "agent:<id>")` 保持 owner 挂起等通知。
- `bMo`:agent 收尾时,对每个 keepalive reason(`agent:`/`workflow:` 前缀),若没有对应排队通知则**继续** `zj`(保持等待);有则移除。
- 效果:后台 agent 完成后 owner 若无其他事,会一直 park 住直到收到通知被唤醒。

## 5. 后台化/同步化控制

- `Jxd`:`autoBackgroundMs` 参数 → 前台 agent 超时(默认阈值)自动 `isBackgrounded:true`(TUI 显示后台化)。
- `$pn`:手动把 running 的前台 agent 标记 `isBackgrounded:true`。
- `Xxd`:非后台、非 keepalive 的终态 agent 从注册表移除。
- **同步 agent 不是"后台标志"**:`run_in_background:false` 时父会话 `await` 子 agent stream 结束(`iMe` 内 `for await` 消费),拿到 `finalMessage` 直接作为 tool_result 返回;后台则立即返回 `async_launched`。

## 6. Stall watchdog

`CLAUDE_ASYNC_AGENT_STALL_TIMEOUT_MS` 默认 600_000ms。`iMe` 内 `A=setTimeout(...)`:超过阈值无新消息且无 in-flight tool → abort → 通知 owner `failed`。`V.size>0`(有 in-flight tool)时**推迟** watchdog。每收到 assistant/user 消息重置。

## 7. 系统事件总线 `rT`(UI/状态同步,非模型)

`task_started` / `task_updated` / `task_progress` / `background_tasks_changed`(REPLACE 语义,成员变化即发:start/completion/kill/前台转后台)。

## 8. 其他后台面

- **Monitor 工具**:`tokio`-风格后台流;完成/失败/kill 都走 `M9s` 通知。
- **远程 agent** `kd({taskType:"remote_agent", ...})`,`Lob` 读取输出。
- **Workflow**:`local_workflow` task 类型,同注册表。
- 每个 task 有独立 `transcripts`(子 agent 自己的消息历史),owner 通知里只带 `<result>` 摘要,不复制全部历史。

## 9. 与 agena 融合的关键映射(设计输入)

| Claude Code | agena 现状 | 融合方案 |
|---|---|---|
| Task Registry(`f3`,`notified` 原子 claim) | `SessionMeta`/`BackgroundOperation` marker | 引入 `taskId`+`notified` 语义到 part |
| `M3t`/`M9s` 完成 → `kd` 通知 | `complete_background_operation`(只终态化 part) | 通知同时注入一条"通知 part"并触发新 turn |
| `<task-notification>` XML 注入对话 | 无对应机制 | 新 part kind `system_notification`/`task_notification`,投影为 system 风格消息 |
| `priority:"next"` 高优先级排队 | `submit_user_message` 普通排队 | 通知走独立高优先级通道 |
| keepalive 防 evict | 无(agena session 是持续实体) | 不需要,但"通知到达前不结束 turn"可借鉴 |
| `run_in_background` 字段(Agent/Bash) | `tasks.create`/`shell.run(background)` | 统一为 `run_in_background` 语义 |
| headless 无重注入 → 退化同步 | agena 无 headless 概念 | 可支持但非阻塞 |
