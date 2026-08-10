# plan / ask / task 时机判定 —— 原文摘录(Claude Code v2.1.223)

以下为从二进制中提取的工具描述原文(英文),供精确参考。

## Plan:EnterPlanMode 描述(节选,`kTb`/`Mkp`)

> Use this tool proactively when you're about to start a non-trivial implementation task. Getting user sign-off on your approach before writing code prevents wasted effort and ensures alignment.
>
> ## When to Use This Tool
> **Prefer using EnterPlanMode** for implementation tasks unless they're simple. Use it when ANY of these conditions apply:
> 1. **New Feature Implementation** ...
> 2. **Multiple Valid Approaches**: The task can be solved in several different ways
> 3. **Code Modifications**: Changes that affect existing behavior or structure
> 4. **Architectural Decisions**: The task requires choosing between patterns or technologies
> 5. **Multi-File Changes**: The task will likely touch more than 2-3 files
> 6. **Unclear Requirements**: You need to explore before understanding the full scope
> 7. **User Preferences Matter**: If you would use AskUserQuestion to clarify the approach, use EnterPlanMode instead
>
> ## When NOT to Use This Tool
> Only skip EnterPlanMode for simple tasks:
> - Single-line or few-line fixes (typos, obvious bugs, small tweaks)
> - Adding a single function with clear requirements
> - Tasks where the user has given very specific, detailed instructions
> - Pure research/exploration tasks
>
> - If unsure whether to use it, err on the side of planning ...

## Plan:ExitPlanMode 描述(`z_p`)

> Use this tool when you are in plan mode and have finished writing your plan to the plan file and are ready for user approval.
> ## When to Use This Tool
> IMPORTANT: Only use this tool when the task requires planning the implementation steps of a task that requires writing code. For research tasks where you're gathering information, searching files, reading files or in general trying to understand the codebase - do NOT use this tool.
> ## Before Using This Tool
> - If you have unresolved questions about requirements or approach, use AskUserQuestion first (in earlier phases)
> - Once your plan is finalized, use THIS tool to request approval
> **Important:** Do NOT use AskUserQuestion to ask "Is this plan okay?" or "Should I proceed?" - that's exactly what THIS tool does.

## Plan:plan mode 激活提示词(节选)

> Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits, run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supersedes any other instructions you have received (for example, to make edits).
> ...
> You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.
> Answer the user's query comprehensively, using the AskUserQuestion tool if you need to ask the user clarifying questions ... ask all clarifying questions you need to fully understand the user's intent before proceeding.

高级 plan 5 阶段档(节选):

> ### Phase 1: Initial Understanding
> Goal: Gain a comprehensive understanding of the user's request by reading through code and asking them questions. Critical: In this phase you should only use the Agent agents IN PARALLEL** (single message, multiple tool calls) to efficiently explore the codebase.
> - Use 1 agent when the task is isolated to known files ...
> - Use multiple agents when: the scope is uncertain, multiple areas of the codebase are involved ...
> ### Phase 2: Design
> **Default**: Launch at least 1 Plan agent for most tasks ... **Skip agents**: Only for truly trivial tasks (typo fixes, single-line changes, simple renames)
> ### Phase 5: Call ExitPlanMode
> NOTE: At any point ... feel free to ask the user questions or clarifications using the AskUserQuestion tool. Don't make large assumptions about user intent.

## Ask:AskUserQuestion 描述

> Asks the user multiple choice questions to gather information, clarify ambiguity, understand preferences, make decisions or offer them choices.
>
> Use this tool only when you are blocked on a decision that is genuinely the user's to make: one you cannot resolve from the request, the code, or sensible defaults.
>
> Reserve this for decisions where the user's answer changes what you do next — not for choices with a conventional default or facts you can verify in the codebase yourself. In those cases pick the obvious option, mention it in your response, and proceed.
>
> Plan mode note: To switch into plan mode, use EnterPlanMode (not this tool). Once in plan mode, use this tool to clarify requirements or choose between approaches BEFORE finalizing your plan. Do NOT use this tool to ask "Is my plan ready?", "Should I proceed?", or otherwise reference "the plan" in questions — the user cannot see the plan until you call ExitPlanMode for approval.

## Task:Agent 工具(别名 Task)描述(`mkp`)

> Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities and tools available to it.
>
> ## When to use
> Reach for this when the task matches an available agent type, when you have independent work to run in parallel, or when answering would mean reading across several files — delegate it and you keep the conclusion, not the file dumps.
> For a single-fact lookup where you already know the file, symbol, or value, search directly. Once you've delegated a search, don't also run it yourself — wait for the result.
>
> ## When not to use
> If the target is already known, use the direct tool: Read for a known path, Grep for a specific symbol or string. Reserve this tool for open-ended questions that span the codebase, or tasks that match an available agent type.
>
> ## Writing the prompt
> Brief the agent like a smart colleague who just walked into the room ...
> **Never delegate understanding.** Don't write "based on your findings, fix the bug" or "based on the research, implement it." ... Write prompts that prove you understood: include file paths, line numbers, what specifically to change.

## Task:Delegating to subagents(主提示词约束段,`Sdu`)

> Subagents multiply cost and time: each one re-establishes context, re-explores, and reports back, and you then re-read its report. Delegate only when the payoff clearly exceeds that overhead. Before spawning, apply these tests:
> - Do the work inline when it is a small, bounded sub-task — a few file reads, one search, a short edit, a single check. Do not spawn a subagent for work you could finish yourself in a handful of tool calls.
> - Do not fan out multiple subagents on a single small task. Parallel subagents are for genuinely independent, sizeable tracks (unrelated modules, a wide multi-file investigation), not for splitting one modest job into pieces.
> - Do not spawn a subagent to review, re-verify, or double-check work you can verify inline.
> - If you delegate, commit to the delegation: do not redo the subagent's work while waiting ...
> - Keep spawn counts low. One well-briefed subagent for a large independent chunk is worth more than several loosely-briefed ones ...
> Delegate for work that is genuinely independent, large enough to justify a fresh context, or naturally parallel. Otherwise, do it yourself.

## 补充:主提示词里对"问"与"做"的平衡规则

- `Xyb` # Delivering work:先做不依赖答案的部分;阻塞性提问只用于"任何假设都不安全/会让工作白做"的情形。
- `Jyb`:When you have enough information to act, act. Do not re-derive facts ...
- `Lyb` 自主模式:用户不在场时禁止 "Want me to…?" / "Shall I…?",仅破坏性或范围变更才停。
- `zyb` Session-specific guidance:需要用户自己跑命令时建议 `! <command>`;超过约 3 次查询的广泛探索才派 Explore 子代理。
- 记忆段(KQr):"When to use or update a plan instead of memory ... When to use or update tasks instead of memory"——plan/task/memory 三者按"跨会话价值 vs 当前会话进度"分流。
