# Claude Code v2.1.223 系统提示词逆向报告

逆向对象:本机 `/Users/canxin/.local/share/claude/versions/2.1.223`(macOS arm64, bun 编译的单文件 Mach-O,约 272MB,构建于 2026-08-05,GIT_SHA 4535f697)。

## 一、逆向方法

1. `strings` 定位 "You are Claude Code" 等标记,确认提示词文本在 bun 字节码字符串池与可读 JS 源码区(约 235M–263M)各有一份。
2. 解析字节码字符串池条目格式:12 字节头(`10 00 00 00 00 00 00 00 09 00 00 00`)+ 4 字节小端长度 + UTF-8 文本 + 16 字节对齐,全量索引出 40 万个字符串条目。
3. 在 JS 源码中定位主提示词组装函数 `X9(...)`(位于 251.32M)与各段函数,逐段提取模板字面量原文。
4. 单独提取 plan mode 提示词(池 118.78–118.83M)、EnterPlanMode/ExitPlanMode/AskUserQuestion/Agent(别名 Task)四个工具的描述与时机判定文本。

## 二、主系统提示词最终结构

首行问候语由 `Wys` 常量给出:`You are Claude Code, Anthropic's official CLI for Claude.`(SDK/非交互变体为 "running within the Claude Agent SDK" 或 "You are a Claude agent")。之后由 `tue()` → `X9()` 按以下顺序拼装(非 brief 模式):

1. `Fyb` — 身份段:You are an interactive agent that helps users with software engineering tasks...+ 安全准则(`Noa`)+ 禁止猜测 URL。
2. `Uyb` — `# System`:文本输出/权限模式/system-reminder 提示/提示注入警告/hooks 说明/上下文自动压缩。
3. `qyb` — `# Doing tasks`:不过度设计、不写多余注释、UI 必须实测、探索性问题先给建议等。
4. `jyb` — `# Executing actions with care`:按可逆性与爆炸半径决定要不要先问用户。
5. `Wyb` — `# Using your tools`:优先专用工具、并行调用、用任务工具拆解跟踪。
6. `Vyb` — `# Tone and style`。
7. 动态段(按 `X9` 内 `m` 数组顺序,按条件注入):
   - `kyb` `# Communicating with the user` / `# Text output`(反冗长)
   - `Dyb` 代词规则(未声明一律 they/them)
   - `xyb` 谨慎行动(难撤销/对外动作先确认)
   - `Iyb` 任务连续性(已同意任务端到端覆盖)
   - `Pyb` Claude Fable 5 模型身份(可选)
   - `zyb` `# Session-specific guidance`(含 Agent 工具何时用、fork 何时用)
   - `KQr` `# Memory`(记忆系统,含 memory/plan/task 三者取舍)
   - `ebb/rbb/tbb` `# Environment`(工作目录/git/模型/知识截止)
   - `Nyb` `# Language`(按用户语言回复)
   - `$yb` `# Output Style`(可选)
   - `nbb` `# Background Session`(后台任务工作树/提交/PR 策略)
   - `eGo` `# Scratchpad Directory`
   - `obb` `# Context management`(自动压缩说明)
   - `ibb` brief 模式 / `lbb` `# Focus mode`
   - `Jyb` 有足够信息就行动;`Xyb` `# Delivering work`;`Qyb` `# Corrections`
   - `Sdu` `## Delegating to subagents`(counter-steer 时启用,见下文)
   - `Myb` heron_brook 覆盖文本(默认:`Do not call the AgentTool unless the user requested it`...)
   - `Lyb` 自主运行模式(用户不在场时不问 "Shall I…")
8. 收尾:`I0p` 附件相关提示、`vTe`(如有)、`rdu` 自动记忆提醒。

## 三、plan / ask / task 三件事的时机判定(核心)

这三件事各自有独立的工具描述 + 主提示词交叉约束,共同构成 "恰当�时机":

### 1) Plan —— EnterPlanMode / ExitPlanMode + plan mode 提示词

**EnterPlanMode 工具描述(`kTb`/`Mkp`)明确列出何时用、何时不用**:
- 用:新功能实现、存在多种可行方案、改动现有行为/结构、需要架构决策、会碰 2-3 个以上文件、需求不清晰、用户偏好会影响实现方向(若本来想用 AskUserQuestion 澄清方案,直接改用 EnterPlanMode)。
- 不用:单行/几行修复(typo、明显 bug、小改动)、需求非常明确的单函数、纯研究/探索任务。
- 边界:不确定时宁可先规划;工具要求用户批准才能进入 plan mode。

**Plan mode 激活后注入的系统提示词(两档)**:
- 基础档(池 118.826M):"Plan mode is active... you MUST NOT make any edits, run any non-readonly tools... This supersedes any other instructions"+ 只能编辑 plan 文件,其余全部只读 + 用 AskUserQuestion 澄清需求。
- 完整 5 阶段档(Opus/高级 plan):Phase 1 Initial Understanding(并行派 Agent 探索代码库,明确 1 个还是多个 agent 的判据)、Phase 2 Design(默认至少派 1 个 Plan agent 设计实现方案,琐碎任务才跳过)、...、Phase 5: Call ExitPlanMode。

**ExitPlanMode 工具描述(`z_p`)**:只在 plan mode 且写完 plan 文件后使用;只用于"需要写代码的实现步骤规划",研究/读代码类任务禁用;有未解决问题先用 AskUserQuestion,定稿后用它请求批准;**禁止用 AskUserQuestion 问 "Is this plan okay?"**。

### 2) Ask —— AskUserQuestion 工具描述

- 一句话定义:Asks the user multiple choice questions to gather information, clarify ambiguity, understand preferences, make decisions or offer them choices。
- 时机判定(`aTs`):"Use this tool only when you are blocked on a decision that is genuinely the user's to make: one you cannot resolve from the request, the code, or sensible defaults."
- 反例(`DFu`):"Reserve this for decisions where the user's answer changes what you do next — not for choices with a conventional default or facts you can verify in the codebase yourself. In those cases pick the obvious option, mention it in your response, and proceed."
- Plan mode 内:用于定稿前澄清需求/在方案间选择;禁止用来问计划是否 OK。

### 3) Task —— Agent 工具(别名 Task)描述(`mkp`)

本版本子代理工具主名 **Agent**,别名 **Task**(`var ti="Agent",MU="Task"`)。描述含:
- `## When to use`:任务匹配某个可用 agent 类型、有可并行独立工作、或回答需要跨多个文件阅读时——委派出去,你只留结论。
- `## When not to use`:目标已知时直接用 Read/Grep;已委派后不要自己重复做。
- `## Writing the prompt`:像给刚进门的聪明同事交代一样写 brief;禁止 "based on your findings, fix the bug" 这类把理解推给子代理的写法。
- fork 子类型:需要保留上下文/开放式问题用 `subagent_type: "fork"`。

**主提示词对 task 的克制约束(两处)**:
- `Sdu` `## Delegating to subagents`(counter_steer 策略启用时):子代理贵且慢,只有收益明显超过开销才委派;小任务自己做、别为小任务扇出多个子代理、能在自己循环里验证就别派去验证、派了就别重复做、控制数量。
- `zyb` Session-specific guidance:探索超过一定查询数才派 `Agent(Explore)`;
- `C0p`(heron_brook 默认):"Do not call the AgentTool unless the user requested it / Do not use workflows or deep-research unless the user requested it"。

## 四、三者如何互相约束(为什么"时机"把握得好)

- 通道隔离:澄清用 AskUserQuestion,批准计划只能用 ExitPlanMode,切换规划用 EnterPlanMode——工具描述互相点名,禁止串用(plan mode 提示词原文:"End turns with AskUserQuestion (for clarifications) or ExitPlanMode (for plan approval). Never ask about plan approval via text or AskUserQuestion.")。
- 默认收敛:有默认值/可自查的就直接做(Ask 反例);能自己干完的别派 agent(Delegating 段);简单改动别进 plan mode(EnterPlanMode 反例)。
- 兜底规则:Xyb/`Delivering work` 说"先做完不依赖答案的部分;阻塞性提问只留给任何假设都不安全的情形";Jyb 说"信息足够就行动"。
- 自主模式(Lyb)更进一步:用户不在场时连 "Want me to…?" 都不准问,只有破坏性/范围变更才停。

## 五、原始素材文件

- 各段函数源码提取:`/tmp/sections_raw.txt`、`/tmp/region251c.txt`、`/tmp/region250b.txt`、`/tmp/region_agent.txt`、`/tmp/region_agent2.txt`、`/tmp/zp.txt`、`/tmp/planmode_entries.txt`、`/tmp/mem_sec.txt`

## 更新:完整提示词全文

- `system-prompt-full.md` — 主系统提示词全文(逐段逐字重建,动态占位用 `{...}` 标注)+ 附录 A 收录 EnterPlanMode / ExitPlanMode / AskUserQuestion / Agent(别名 Task)/ TaskCreate / TaskList 工具定义与 plan mode 提示词全文。

## 更新:完整导出(export/)

- `export/corpus-all-literals.txt`(20MB)— JS 源码区全部 21,375 条模板字面量/长字符串的去重全集(提示词语料完整清单)。
- `export/tools.md` + `export/tools-raw/`(58 个原始定义块)— 全部 56 个内置工具:searchHint / description / prompt(已解析)+ 完整原始定义代码。
- `export/subagents.md` + `export/agents.json` — 全部 10 个内置 agent 类型(Explore / Plan / general-purpose / main / claude / main-session / subagent / workflow-subagent / teammate / statusline-setup)的 whenToUse 与完整 system prompt。
- `export/runtime-main-prompt.md` — 按本机环境(工作目录 /Volumes/Rc20/Projects/agena、git、macOS/zsh、模型 deepseek-v4-flash(max))渲染的运行时成品主提示词。
- `export/corpus-index.json`、`export/tools.json` — 机器可读索引。

另:`system-prompt-full.md`(主提示词全文+附录A plan/ask/task 工具与 plan mode 全文)、`plan-ask-task-original.md`(三决策原文摘录)、`raw/`(逆向过程原始提取)。
