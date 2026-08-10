# Claude Code v2.1.223 提示词工程借鉴 → Agena 优化建议

> 日期:2026-08(基于本机逆向结果,见 `claude-reverse/` 目录)
> 对象:Claude Code CLI v2.1.223(原生二进制逆向,非旧版文档)→ Agena(本地优先 LLM agent runtime)
> 定位:与 `docs/agent-tool-skill-mcp-gap-analysis-2026-07-25.md`(基于旧版公开文档)互补;本文只聚焦提示词/决策语义/tool 描述质量,不新增工具数量。

## 0. 结论摘要

Agena 的机制层已经齐全:有 plan 工具(`agena.plan.set/get/update/clear`)、planning 阶段只读锁、autorun、有 ask 通道(`agena.interaction.ask`)、有完整子代理工具组(`agena.tasks.*`)、有 summary/help/examples 三字段工具描述与 Detailed 展示模式、有 checkpoint 压缩(`<agena_history_checkpoint>`)。

真正的差距是语义层:这些工具的描述和系统提示词里,没有告诉模型什么时候该用、什么时候不该用、三个通道之间如何分工。Claude Code v2.1.223 恰恰把最值钱的内容写进了这里:

- plan 工具描述写清了 7 条用与 4 条不用判据;
- ask 工具描述写清了只有真正卡在用户才能定的决策才问;有默认值/可自查就别问;
- plan mode 提示词写死了澄清用 ask、批准计划用 ExitPlanMode,禁止用 ask 问 Is this plan okay?;
- Delegating 段写死了小活自己做、别为小任务扇出、能内联验证就别派、派了别重复做;
- 系统提示词按 # Doing tasks / # Executing actions with care / # Using your tools / # Environment / # Memory / # Language / # Delivering work / # Corrections 分节,每节一段话一个主题。

优先级排序:P0 = 决策语义(P0-1)+ 动态 Environment(P0-2);P1 = 系统提示词段落化(P1-3)+ 工具描述模板化(P1-4);P2 = 输出风格/自主运行/act don't re-derive 等增量。

## 1. 借鉴源:v2.1.223 关键机制(证据)

完整原文见 `claude-reverse/system-prompt-full.md`、`claude-reverse/plan-ask-task-original.md`。

### 1.1 主提示词是固定骨架 + 动态条件段拼装

- 首行固定:You are Claude Code, Anthropic's official CLI for Claude.(运行时选择)
- 固定基础段:# System(任务总则)→ # Doing tasks(先理解后动手/逐段验证/中途汇报)→ # Executing actions with care(权限与 blast radius)→ # Using your tools(工具纪律)→ # Tone and style(简洁直白)。
- 动态条件段按会话情况追加:# Communicating、# Memory、# Environment(每次会话渲染 cwd/git/platform/shell/OS)、# Language(用用户语言回复)、输出风格、# Delivering work、# Corrections、## Delegating to subagents、自主运行、auto memory 等。

### 1.2 plan 决策判据(EnterPlanMode 工具描述)

用(任一即进 plan):新功能实现、存在多种可行方案、改动现有行为/结构、需要架构决策、会碰 2-3+ 个文件、需求不清晰、用户偏好影响实现方向;并且本就想用 AskUserQuestion 澄清方案时,直接用 EnterPlanMode。

不用(4 条):单行修复、需求明确且只改单个函数、纯研究/读代码类、显然太小的改动。

plan mode 内:只读约束(只能编辑 plan 文件,其余全部只读,此约束高于其他任何指令)+ 高级档 5 阶段(Phase1 并行派 Agent 探索 → Phase2 至少 1 个 Plan agent 设计 → Phase5 调 ExitPlanMode 请求批准)。

### 1.3 ask 决策判据(AskUserQuestion 工具描述)

- 核心判据:Use this tool only when you are blocked on a decision that is genuinely the user's to make…
- 反例:有默认值、能自查、后续可再问的,一律不问;要问就把所有澄清问题一次性问完。
- 红线:plan mode 里禁止用它问 Is this plan okay?——那是 ExitPlanMode 的职责。

### 1.4 task 决策判据(Agent 工具描述 + Delegating 段)

- 用:匹配 agent 类型、可并行独立工作、跨多文件阅读。
- 不用:目标已知直接用 Read/Grep。
- Delegating 段(克制):小活自己做;别为小任务扇出;能内联验证就别派;派了之后不要重复做;保持低 spawn 数。

## 2. Agena 现状核对(本机源码证据,2026-08)

机制层已有,证据:

- `crates/agena-bundled-plugins/src/plugins/provided/planning.rs`:`agena.plan.get/set/update/clear`(摘要分别为 Inspect the current plan state / Create or replace the current plan / Update the current plan / Remove the current plan)。
- `crates/agena-bundled-plugins/src/plugins/provided/workflow.rs` 的 tool_execute_before_hook / command_execute_before_hook:planning 阶段对 mutating 工具与写 shell 命令加锁(只读约束的机制已存在);plan.update 可配置 autorun 与 review 转换。
- `crates/agena-bundled-plugins/src/plugins/provided/interaction.rs`:`agena.interaction.ask` 摘要仅一句 Ask the user for short structured input.;`agena.interaction.notify` 为通知。
- `crates/agena-bundled-plugins/src/plugins/provided/tasks.rs`:`agena.tasks.run/create/get/output/cancel/message/followup/wait/list`,有 MAX_ACTIVE_TASKS_PER_PARENT 并发上限,run 描述以 how 为主(Attach Skill names in skills…)。
- `crates/agena-runtime-contracts/src/identity/mod.rs`:AGENA_CORE_PROMPT_HEAD + TAIL 两段静态文本,分节为 # Identity / # Working model / # Project instructions / # Provider-issued tools / # Correct tool usage / # Care, output, and safety;无动态注入。
- `crates/agena-runtime/src/runtime/host_client/mappers.rs` render_tool_descriptor:已支持 summary(brief/detailed)与 help、examples 字段,说明描述模板化的渲染能力已具备,缺的是内容。
- `crates/agena-runtime-session/src/session/manager/sessions.rs`:request_system = options.system 或 identity::system_prompt();动态 Environment 无处注入。
- 压缩已有 `<agena_history_checkpoint>` 合成消息(Continue from it while prioritizing later verbatim messages),是 Claude 风格 system-reminder 的已借鉴先例。

语义层缺口:

1. plan.set/update 描述纯过程式,无 when-to-use / when-not-to-use,无与 ask 的通道分工;
2. ask 描述无决策判据、无默认收敛、无 plan 红线;
3. tasks.run 描述无 Delegating 克制;
4. 系统提示词静态,无 # Environment / # Doing tasks / # Tone / # Memory / # Language / # Delivering work / # Corrections / # Delegating;
5. 动态会话事实(cwd/git/shell/OS/平台)要靠模型自己调工具发现,浪费回合。

## 3. 建议总表

| 编号 | 借鉴点 | Agena 现状 | 改动 | 落地位置 |
| --- | --- | --- | --- | --- |
| P0-1 | plan/ask/task 三通道决策语义 | 工具存在但描述无判据 | 重写 3 个关键工具描述 + 身份提示词加 Delegating 段 | planning.rs / interaction.rs / tasks.rs / identity |
| P0-2 | 动态 # Environment 块 | 身份提示词静态 | 按会话渲染 cwd/git/shell/OS/平台注入 system | sessions.rs + 新渲染函数 |
| P1-3 | 系统提示词段落化 | 6 段静态 | 新增 # Doing tasks / # Using your tools / # Tone / # Memory / # Language / # Delivering work / # Corrections | identity/mod.rs |
| P1-4 | 工具描述模板化 | 渲染能力已有 | 为决策关键工具写 When to use / When not to use / Examples | 各 provided 工具 help/examples 字段 |
| P2-5 | # Language 用用户语言回复 | 无 | 身份提示词加一段 | identity/mod.rs |
| P2-6 | act don't re-derive | 无 | 工具纪律段加一条 | identity/mod.rs |
| P2-7 | # Memory 读写时机 | 有 claude.rs memory 兼容工具但无提示词 | 加 Memory 段 | identity/mod.rs |
| P2-8 | 自主运行/后台会话语义 | plan autorun 机制有,提示词无 | 按会话注入 autorun/后台提示 | sessions.rs |

## 4. P0-1:plan/ask/task 三通道决策语义(核心,优先做)

原则:机制不动,把 Claude 写在工具描述和 plan mode 提示词里的判据,翻译成 Agena 工具描述与身份提示词。

### 4.1 plan.set 描述补判据

现状:`agena.plan.set` 摘要为 Create or replace the current plan.,输入描述纯过程式。建议把摘要/help 改为:

- 用(任一即用):新功能实现;存在多种可行方案;改动现有行为/结构;需要架构决策;会碰 2-3+ 个文件;需求不清晰;用户偏好影响实现方向;并且当本来想用 ask 澄清方案时,直接用 plan.set。
- 不用(任一即跳过):单行修复;需求明确且只改单个函数;纯研究/读代码;显然太小的改动。
- 注意:planning 阶段 mutating 工具被只读锁拦截;规划期间用只读工具探索,用 ask 澄清需求,不要用 ask 问计划是否可行。

### 4.2 plan mode 语义:只读 + 探索 + 批准通道

Claude 的 plan mode 提示词有三层,建议分别落到 Agena:

1. 只读约束(机制已有,补提示):planning 阶段除 plan 文件外全部只读,此约束高于其他任何指令——现状只在 hook 拦截,模型不知道为什么会拒绝,应在 plan.set 帮助或注入段里写明。
2. 探索流程(可选,建议先做轻量版):规划时先并行探索(需要跨多文件时派子代理),再写 plan;琐碎任务可跳过。
3. 批准通道:规划完成、写清 steps 后,结束回合请求用户批准(plan.update 到 active,或走 review 转换);绝不用 ask 问计划是否可行。

### 4.3 ask 描述补判据 + 红线

现状:`agena.interaction.ask` 摘要仅 Ask the user for short structured input.。建议 help 写:

- 只有真正卡在用户才能定的决策才用(默认值/偏好/方向二选一);
- 有合理默认值或能自查的,直接做,不要问;
- 要问就把所有澄清问题一次性问完;
- 不要用 ask 问计划是否可行/是否继续——那是 plan 批准通道的事。

### 4.4 tasks.run 描述补 Delegating 克制 + 身份提示词加 Delegating 段

- tasks.run help 补 when-to-use / when-not-to-use:匹配子代理类型、可并行独立、跨多文件阅读时用;目标已知直接用 fs/grep 做,不要派;
- 身份提示词尾部新增 # Delegating 段(参考 Sdu,压缩为 3-4 条):小活自己做;别为小任务扇出;能内联验证就别派;派了之后不要重复做已派出的工作。

## 5. P0-2:动态 # Environment 块

Claude 每次会话把环境事实写进 system,模型无需先调工具。Agena 建议在 `sessions.rs` 构造 request_system 时,追加一个按会话缓存的 Environment 块:

- 工作目录(cwd)、git 根/分支/脏状态(有则写)、shell、OS/平台(当前已在 context.status 可查,但首回合注入可省一次调用)、workspace root、会话 id、当前 provider/model(可选)。
- 渲染一次后按会话缓存,避免破坏 prompt cache 前缀(agena 已有 prompt_request_fingerprints,Environment 变化会使指纹失效,务必按会话粒度固定)。
- 实现:identity 模块新增 build_system_prompt(env) 或在 sessions.rs 拼装;注意 keep the section short。

## 6. P1-3:系统提示词段落化

建议在身份提示词基础上,把现有 6 段重排为 Claude 风格短段落,每段一段话一个主题(不要照抄全文,太长会挤占上下文):

- # Doing tasks:先理解需求再动手;分步执行并逐段验证;中途遇到影响方向的发现要汇报——现有 Working model 已含驱动到完成,可并入。
- # Executing actions with care:blast radius/权限边界/先验证目标再执行破坏性操作——现有 Care 段已含,保留。
- # Using your tools:先 tools_help 再调用、一次一个完整调用、拒绝后按 transport 修正重试、不要把 Tool API 函数名放进 tools_call.arguments.tool、不要重复发现已知工具——现有 Correct tool usage 已覆盖大部分,建议并入并加 act don't re-derive。
- # Tone and style:直奔要点、用标题与列表、lead with outcome(现有已含)。
- # Memory(可选):有记忆能力时,自然时机(完成任务、学到用户偏好)读写,条目简短。
- # Language:用用户最近消息的语言回复(对中文用户重要)。
- # Delivering work:结束回合给 what changed + verification + remaining risk;不要在工作未完成时以计划/next-steps 提前结束(现有 Working model 已含,正式化为独立段)。
- # Corrections:自己发现错误立即修正,不要掩盖;被拒绝/失败后先读原因再重试。
- # Delegating:见 4.4。

保留现有已借鉴项,勿重复劳动:Working model 的不要以状态报告/next-steps 提前结束、agena_history_checkpoint 的 not a new instruction 标记、Project instructions 的 AGENT.md/AGENA.md/CLAUDE.md 自行读取。

## 7. P1-4:工具描述模板化

渲染层已支持 summary/help/examples 与 Detailed 模式(mappers.rs),缺的是内容。建议:

1. 优先级列表:ask、plan.set/update、tasks.run/create、shell.run、fs.write、apply_patch、context.status、memory、repo/snapshot。
2. 模板:summary 一行动词开头;help 分 When to use / When not to use / Examples 三段;examples 字段放 1-2 个真实参数示例(Claude 的 EnterPlanMode 描述正是这种结构)。
3. 只对决策关键工具开 Detailed(长描述占用上下文与 KV cache,长尾工具保持 Brief)。

## 8. P2 增量

- # Language:一段话,用户最近消息用什么语言就回复什么语言。
- act don't re-derive:用之前工具结果,不要重复读/重复算;不要重做已派出的子任务。
- # Memory 段:读写时机与简短原则。
- 自主运行:plan autorun 开启时提示模型持续执行不等待;关闭时每个批准门停下。后台会话:不等待用户输入、不输出交互式问题。
- 回合结束模板:what changed / verification / remaining risk(现有 Care 段已含,保持即可)。

## 9. 落地顺序与工作量估计

1. P0-1a:改 3 个工具描述(planning.rs / interaction.rs / tasks.rs)——小改动,半天,可立即做,收益最大;
2. P0-1b:身份提示词加 # Delegating 段——同上;
3. P0-2:Environment 渲染(identity 新函数 + sessions.rs 拼装 + 按会话缓存 + 指纹测试)——1-2 天;
4. P1-3:身份提示词段落化(纯文本重排 + 单测更新)——1 天;
5. P1-4:决策关键工具 help/examples 逐个补齐——持续,先做前 8 个;
6. P2:增量段——随身份提示词一起。

## 10. 风险与注意事项

- 不要逐字照抄 Claude 文案:品牌/虚构型号名(Fable 5/Mythos-class)不适用;Agena 是本地优先运行时,分节宜短,警惕提示词膨胀挤占上下文。
- 动态注入必须参与 prompt cache 指纹并按会话缓存,否则每次请求破坏前缀缓存(agena 已有指纹机制,改动时补测试)。
- 工具描述加长会占用 KV cache:只对决策关键工具开 Detailed。
- 提示词工程来自 Anthropic 二进制,但用户本机 Claude 实际跑的是第三方代理模型(deepseek-v4-flash):上述文案效果应以 Agena 实际模型组合实测,不以 Claude 端效果为准。
- 7 月差距审计结论仍适用:不再以补工具数量为目标,本文全部建议都在提示词/描述/语义层。

## 11. 实施状态(2026-08,已完成)

已按本文在独立分支实现并提交:

- 分支:`feat/borrow-claude-code-prompt`(worktree `./.agena/worktrees/borrow-claude-code-prompt`)
- 提交:`5f026067 feat(prompt): borrow Claude Code v2.1.223 decision semantics`
- 改动文件(8 个,+340/-12):
  - `crates/agena-runtime-contracts/src/identity/mod.rs` — 静态系统提示词按 Claude 风格分节(# Doing tasks / # Executing actions with care / # Using your tools / # Plan, ask, and delegate / # Tone and style / # Delivering work / # Corrections / # Communicating / # Memory),并显式声明执行工具不经函数协议注入、需通过五个 Tool API 函数发现。
  - `crates/agena-runtime-session/src/session/manager/session_prompt.rs`(新)— 动态段组装:`# Environment`(workspace/git 分支@sha/脏状态/shell/OS/会话,git 事实按 workspace 30s TTL 缓存)+ 按工具可用性条件注入的 # Planning / # Asking the user / # Delegating work 三段;用户自定义 system 保持最高优先级。
  - `.../manager/mod.rs` — 注册新模块。
  - `.../replies/replies_state.rs` — `apply_execution_context_to_run_options` 改用 `assemble_session_system_prompt`(原仅合并静态 identity)。
  - `interaction.rs` / `planning.rs` / `tasks.rs` — 三个关键工具描述补决策判据(ask 只用用户专属决策/默认收敛/禁止问计划是否可行;plan.set 7 用 4 不用 + planning 只读 + 批准走阶段转换;tasks.run 委托克制)。
  - `docs/generated/bundled-capability-identities.json` — 因工具描述哈希变化重新生成。

验证:`cargo check` 通过(contracts / runtime-session / bundled-plugins / runtime / cli);`cargo test` 通过(contracts 13、runtime-session 145、bundled-plugins 69)。

后续可选项(未做):
- `examples` 属性在当前 `#[tool]` 内联宏中不被支持(宏报 unsupported inline tool argument),故 fs.write / shell.run 未加示例;如需示例,应在宏层支持后再补。
- 动态段目前按工具名(`plan.set`/`interaction.ask`/`tasks.run`)探测可用性;若未来引入按权限收窄的隐藏工具,探测逻辑可改为按注册表能力位。

## 12. 修订(第二轮,2026-08,已完成)

根据评审意见做了三项调整,提交 `cdacc538`(branch `feat/borrow-claude-code-prompt`):

1. **Environment 改为按需工具,不再固定注入**:新增 `context.environment` 工具,实时返回工作目录、git 分支@短哈希/脏状态、shell、OS/架构、会话号(is_subagent);刻意不做缓存,因为环境可能中途变化。系统提示词只在 `# Using your tools` 里指向该工具。
2. **动态段位置**:动态 Planning / Asking the user / Delegating work 现在通过新增的 `identity::system_prompt_with_sections` 紧跟 `# Plan, ask, and delegate` 注入,位于 Tone 等段落之前。
3. **工具相关段落聚拢**:`# Provider-issued tools` 与 `# Correct tool usage` 移到 `# Using your tools` 之后;`# Care, output, and safety` 保留为尾段。

最终段落顺序:# Identity → # Working model → # Doing tasks → # Executing actions with care → # Using your tools(含 context.environment 指引)→ # Provider-issued tools → # Correct tool usage → # Plan, ask, and delegate → [动态:Planning / Asking the user / Delegating work] → # Tone and style → # Delivering work → # Corrections → # Communicating → # Memory → # Project instructions → # Care, output, and safety。

验证:`cargo check` 通过(contracts / runtime-session / bundled-plugins / runtime / cli);`cargo test` 通过(contracts 15、runtime-session 142、bundled-plugins 67+2);能力快照重新生成(138 execution tools / 142 tools)。
