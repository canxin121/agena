# Git 工作流程与文件恢复

Agena uses the project's ordinary Git repository for checkpoints and recovery. The model runs Git through available shell tools. User-facing sessions use available `agena.interaction` and `agena.plan` workflows for user decisions and plan approval; subagents report these needs to their parent. Agena has no hidden snapshot repository, automatic commit service, workspace undo log, or Git command interception layer.

会话的 rewind（回退）和 fork（分叉）只影响对话，不会还原磁盘文件。Git 也不能保证恢复未记录、未跟踪、被忽略或位于仓库之外的内容；模型不应承诺一个通用的“撤销一切”能力。

This page and [`identity/git_workflow.md`](../crates/agena-runtime-contracts/src/identity/git_workflow.md) provide detailed Git reference material. Shared rules and main/subagent responsibilities are defined in [`identity/mod.rs`](../crates/agena-runtime-contracts/src/identity/mod.rs). Both roles receive [`identity/markdown.md`](../crates/agena-runtime-contracts/src/identity/markdown.md); user-facing sessions also receive [`identity/rich_output.md`](../crates/agena-runtime-contracts/src/identity/rich_output.md). Workflow guidance reflects the current role and available tools. The detailed Git reference remains available through `AGENA_GIT_WORKFLOW_PROMPT` but is not automatically added to sessions. User instructions, conversation boundaries, and root project guidance are appended separately; execution contracts come from live tool help.

## 为什么需要区分三棵树

Git 的恢复能力取决于内容到底在哪一层：

| 状态 | 保存的内容 | 常见误判 |
| --- | --- | --- |
| HEAD | 最后一个提交的树 | 在 HEAD 上创建 recovery 分支只保存已提交内容，不包含未提交文件 |
| index / 暂存区 | 普通 `git commit` 的候选树 | 普通提交会收入整个 index，包括用户此前暂存的内容 |
| working tree / 工作区 | 磁盘上的当前文件 | 普通构建和检查通常读取这里；它可能与候选提交不同 |

同一文件也可能混合多方内容：用户已暂存的部分、用户未暂存的部分、模型本次编辑的部分。模型不能因为“这个文件我改过”就认为整个文件都属于当前任务。保留内容本身和用户已有的暂存边界同样重要。

## 接手或恢复任务

任务开始时，模型应检查实际仓库状态，而不是根据对话记忆猜测：

- 仓库根、当前 worktree 和分支。
- HEAD、任务开始时的提交、近期提交风格。
- `git status`、相关暂存和未暂存 diff。
- 是否存在未完成的 merge、rebase、cherry-pick 或尚未处理的冲突。
- 任务基线、PR 目标分支和 push upstream 是否一致。

仓库可能没有 HEAD，`.git` 也可能不是目录；submodule、嵌套仓库、sparse checkout 和浅历史都需要按实际情况处理。模型不应自动 abort 进行中的 Git 操作，也不应删除 lock 文件。

新 worktree 从明确提交创建，不包含原 worktree 的未提交改动。如果任务依赖源目录里的脏文件，必须先解决依赖和归属。之后每次 shell 调用应指定工作目录；文件工具按 Agena workspace 解析路径，不能假设继承了上一次 shell 的 `cd`。

多个 worktree 有各自的 index，但共享分支 ref 和通常共享配置。同一 worktree 的 Git 修改应只有一个执行者；Git 锁不能保护完整的“读取、暂存、提交”过程，所以在提交、恢复、改写或 push 前，需要重新核对相关状态。

## 保护用户已有工作

无关的脏文件不阻塞任务。模型应继续处理自己的部分，但不能自动：

- 暂存、提交或丢弃用户的其他改动。
- `stash` 用户的未提交工作。
- 取消用户已有的暂存。
- 执行宽泛的 `restore`、`reset` 或 `clean`。
- 删除未合并的分支或脏 worktree。

如果安全分离需要用户决定，模型必须通过 `interaction.ask` 询问，并给出具体选项和后果。已明确授权的范围可以复用，不必在每个普通提交前重复询问。

只读工作不需要 Git。需要修改一个没有 Git 保护的项目时，模型应先通过 `interaction.ask` 让用户选择仓库或初始化范围，或者接受没有恢复点地继续；如果用户已经作出决定，则复用该决定。模型不能自行在父目录、账户 home 或嵌套位置初始化仓库。

## 提交什么

模型应只暂存明确属于当前任务的路径或 hunk，并在提交前审查完整候选内容。

普通 `git commit` 会提交整个 index。用户此前暂存的其他文件也会被收入。`git commit --only -- <paths>` 则取这些路径当前工作区的内容，可能包含未暂存的 hunk，并且不会把其他暂存内容一起提交。它只在所选内容全部属于当前任务、且不会破坏用户暂存边界时才是安全的。路径提交并不能自动区分内容归属。

提交前需要确认构建和检查读取的内容与将要提交的内容一致。常见陷阱是：工作区里的未暂存修复让检查通过，但候选提交缺少该修复；或者只检查了路径列表，没有检查实际 diff。无法验证候选树时，应明确说明验证范围，不能声称“提交已通过”。

提交应按顺序完成：暂存、审查完整 diff、commit、检查新 HEAD 和剩余状态。配置的 hooks 和签名应保留；命令失败或超时后先检查 HEAD、index 和 status 的实际结果，再决定是否重试。

## 阶段提交与交付提交

阶段提交用于在长任务、风险实验或中断前保存有价值的进度。它可以包含尚未完成的工作，但必须诚实记录哪些检查没有运行或仍然失败。阶段提交不表示任务完成，也不能绕过拒绝提交的 hook。

交付提交用于审查、集成和定位问题。一个逻辑变化应尽量包含实现、相关测试或验证、必需的生成文件和 lockfile 更新。按文件类型把实现和测试拆成两个不能独立使用的提交，通常没有帮助。多个独立、可理解的变化可以保留多个提交；提交数量和文件数量本身不是质量标准。

只读任务、每次工具调用、临时输出和没有实际变化的操作不需要制造提交。

## 验证的对象

“测试通过”必须说明测试了什么：

- 普通构建和检查读取 working tree。
- 提交审查读取 index 或工作区中选定的路径。
- 推送和合并读取已提交历史。

当工作区包含用户改动、部分暂存或生成文件时，模型应刷新相关证据，必要时在隔离的干净树中验证候选提交。无法做到时，应在交付报告中说明验证对象与提交树的差异。

## 恢复改动

恢复前先识别改动是已提交、已暂存、未暂存还是未跟踪，再只逆转目标内容。

- 已提交且可能已共享的改动优先使用 `git revert`。
- `git revert --no-commit` 可以叠加在脏 index 上；随后普通 commit 会连同既有的暂存内容一起提交。
- 未提交改动只有在确认全部属于当前任务时，才能做有针对性的反向编辑或路径/hunk restore。
- 模型撤销自己刚做、尚未交付的编辑不需要新的用户批准。

丢弃其他人的工作、删除未合并分支或脏 worktree、宽泛 restore/clean/reset、amend、rebase、squash 和 force-push 都需要针对具体范围的授权。“修好”“清理一下”或“推送”这类泛化请求不足以授权这些历史改写。

在获准改写历史前，应保存旧的已提交 tip，并单独保护脏内容。一个指向 HEAD 的 recovery ref 只保存已提交版本，不能保存未提交文件。reflog 不是永久备份，也不能作为唯一恢复手段。

## 发布与 squash

任务完成并验证后，是否推送取决于任务本身和仓库约定：

- 任务明确面向共享仓库，目标分支清楚，且实现/修复的常规完成方式包含发布时，模型可以在既有授权内推送已审查的提交，不需要额外的“是否 push”批准。
- 任务只要求本地修改、发布目标不明确，或用户/项目要求保留在本地时，模型应停在本地。
- 发布属于任务范围时，不必因为用户没有单独说“push”就停在本地；发布不属于任务范围时，也不要自行发布。
- 推送只发布提交，不授权合并 PR，也不授权改写共享历史。

发布前应检查实际的 push endpoint、目标 ref、任务基线、upstream、待推送提交和最终 diff。远端可能已经前进；模型不应隐式 pull/merge/rebase，而应检查分叉和远端新增内容。多个 push URL、`push.followTags`、mirror 和 submodule 设置也会扩大实际发布范围，需要一并检查。

如果若干本地检查点或 fixup 提交组成一个逻辑变化，应按仓库的集成策略决定是否本地 squash。PR 的 squash merge 可能已经提供所需历史；需要本地 squash 时，模型通过 `interaction.ask` 给出确切提交范围、建议消息以及 squash/保留/延后选项。squash 同意不等于 push 同意，也不能把同一决定在每个提交上重复询问。

历史替换需要单独授权。优先使用显式 refspec 和
`--force-with-lease=<ref>:<observed-old-oid>`。不要自动 force 受保护或默认分支；push 被拒绝时，先重新评估，不把拒绝自动升级为 force。结果不确定时，先核对远端状态再重试。squash merge 后，下一项任务应从更新后的基线开始，避免复用已经集成的旧 topic 分支。

## User decisions and plugin workflows

User-facing sessions use the relevant plugins and their available tools for user decisions and planning. References to questions and approvals in this guide apply to those sessions. Subagents report missing decisions or authorization to the parent and continue independent authorized work; they do not open their own user-interaction or plan-approval workflow.

- `agena.interaction` provides `interaction.ask` for user choices, authorization for specific dangerous Git operations, and squash decisions. Use `interaction.notify` for non-blocking notices when useful.
- `agena.plan` combines tools such as `plan.get`, `plan.set`, `plan.edit`, `plan.review`, and `plan.phase` for plan management, required approval, and phase transitions. Plan approval belongs to this workflow.
- Consult current contracts through `tools_help` and invoke available tools through `tools_call`.
- Questions should show concrete consequences and distinct options, including declining or deferring.
- Timeout, cancellation, an empty answer, or a suggested default is not consent.
- Dependent actions must wait for the answer; independent, authorized work may continue.
- If required tools are unavailable, report the limitation and keep dependent actions pending rather than assuming approval.

## 交付报告

交付时，模型应报告：

- 使用的分支和 worktree。
- 提交 ID，以及它们是阶段提交还是交付提交。
- 实际验证了什么，验证的是工作区还是候选提交。
- 仍未完成的工作和已知限制。
- 是否已 push，以及 push 到了哪个目标。
- 是否保留了 recovery ref、交付分支或 worktree。

用于交付的分支、worktree 和 recovery ref 不应为了“看起来干净”而自动合并回原目录或删除。恢复任务时也应保留任务基线、内容归属、进行中的 Git 操作和已授权的精确范围。

## Validation scope

The model executes Git through ordinary shell tools. The core prompt supplies general task, authorization, and work-protection guidance; detailed Git procedures are reference material, and live tool help describes current contracts. Agena does not validate every Git command against these procedures or provide hidden recovery. The repository has no automated evaluation of model Git behavior; passing compilation, formatting, or lint checks does not demonstrate model compliance.

Update [`identity/mod.rs`](../crates/agena-runtime-contracts/src/identity/mod.rs) for shared rules and session roles, [`identity/markdown.md`](../crates/agena-runtime-contracts/src/identity/markdown.md) for shared presentation, [`identity/rich_output.md`](../crates/agena-runtime-contracts/src/identity/rich_output.md) for user-facing output contracts, and [`session_prompt.rs`](../crates/agena-runtime-session/src/session/manager/session_prompt.rs) for capability-aware assembly. Keep this guide and the detailed Git reference aligned when their guidance changes. Run the prompt, capability, and interaction regression tests and the standard Rust checks:

```sh
cargo fmt --all -- --check
cargo test -p agena-runtime-session -p agena-runtime-tools --lib --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Documentation alone does not change the runtime prompt or tool behavior.
