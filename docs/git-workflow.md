# Git 工作流程与文件恢复

Agena 通过统一系统提示词指导模型使用项目自己的 Git 仓库保存和恢复文件改动。模型通过现有 shell 工具执行 Git，通过 `interaction.ask` 获取用户决策。没有新增 Git 自动提交服务、隐藏仓库、文件快照日志或命令拦截器。

提示词的唯一来源是 [`git_workflow.md`](../crates/agena-runtime-contracts/src/identity/git_workflow.md)，由 `identity::system_prompt_with_sections` 注入所有会话。动态询问提示词、询问工具帮助和超时结果采用相同的授权语义。上下文压缩提示词要求保留分支、提交、已有改动和授权范围。

## 默认行为

- 修改前检查仓库、分支、worktree、起始提交，以及暂存和未暂存差异。无关的已有改动不阻塞任务；重叠且无法安全分离的改动通过询问工具处理。
- 已授权的文件修改任务，在完成有意义的工作单元、通过相关验证后主动创建本地提交。用户要求不提交或项目另有提交策略时遵循该要求。只读任务不创建提交。
- 只暂存和提交当前任务的明确文件或改动块，不自动提交、撤销、stash 或取消暂存用户的工作。提交失败后检查原因，修复后重新提交，不把失败当作可以 amend 前一个提交的理由。
- 不主动 push。用户明确授权发布某个分支后，检查目的地和全部待推送提交。多个未发布的 WIP/fixup 提交属于同一逻辑改动时，通过询问工具展示范围和提交消息，让用户选择 squash、保留或暂缓。独立且有意义的提交保持分开。
- 普通任务、计划批准和 push 请求都不隐含授权破坏性操作或改写历史。确认必须对应具体路径、提交范围或远端分支；同一范围已明确授权时不重复询问。
- 已提交改动优先用 `git revert` 撤销；未提交改动先检查当前内容再做精确反向编辑。不得为撤销局部改动而恢复整个工作区。已发布历史默认通过后续提交修正。
- 没有 Git、没有仓库或目标不在仓库内时，通过询问工具决定初始化范围或接受没有 Git 恢复能力。禁止悄悄初始化用户主目录、父目录或嵌套仓库。

## 询问必须走工具

已知工具先通过 `tools_help` 读取实时契约，再通过 `tools_call` 调用：

| 决策 | 工具和语义 |
| --- | --- |
| 需求、方向或偏好不清楚 | `interaction.ask` |
| 具体危险 Git 操作缺少授权 | `interaction.ask`，说明目标、影响和安全替代方案 |
| 推送前是否 squash | `interaction.ask`，显示精确范围和建议消息 |
| 计划审批 | `plan.review` |
| 同一操作和范围已经获得授权 | 直接继续，无须重复询问 |
| 询问超时、取消或没有答案 | 不构成批准；只继续独立且已授权的工作 |
| 询问工具不可用 | 保留待决操作并报告工具限制，不用聊天文本提问代替 |

模型不能以一句“是否继续？”结束回合等待聊天回复。依赖用户决定的操作必须等待询问工具的结果。默认推荐选项也不等于用户选择。

## 能力边界

Git 只保护实际记录过的内容。未提交、未跟踪、被忽略或仓库外的文件不能保证恢复；Git 提交也不能撤销数据库、部署和其他外部副作用。聊天 `rewind` / `fork` 只改变会话历史，不恢复文件。

`fs.apply_patch` 输出操作标识、文件变更、diff 和进度，不生成或持久化 `inverse_patch`、`before_hash`、`after_hash`。应用前校验、文件锁、原子替换和失败调用内的尽力清理继续负责单次写入的正确性；它们不是跨回合的撤销历史。用于隔离工作的现有 snapshot/worktree 功能不承担自动恢复承诺。

这是一套模型行为规则，不是 Git 命令的强制安全边界。现有运行时权限仍然生效；本次没有用新的运行时代码保证模型绝不执行危险命令。

## 调研依据与取舍

调研日期：2026-10-04。先检索公开提示词，再读取命中的原文，同时核对本机安装包和相邻仓库。

| 来源 | 借鉴与取舍 |
| --- | --- |
| 本机 Claude Code `2.1.288`，安装包内 `Git Safety Protocol`；`../claude-code` 的 `src/tools/BashTool/prompt.ts`、`src/tools/AskUserQuestionTool/prompt.ts`（`a371abb`） | 借鉴精确暂存、保留 hooks、失败后新建提交、禁止隐式危险操作。其默认“明确要求才提交”调整为 Agena 的任务里程碑本地提交策略。 |
| `../codex/codex-rs/core/gpt-5.2-codex_prompt.md`（`774d6425`） | 借鉴保护用户已有改动、避免未经授权的 amend/reset。无关改动继续工作；真正需要用户决定时调用工具。 |
| `../opencode/packages/opencode/src/tool/shell/shell.txt`、`src/snapshot/index.ts`、`src/session/revert.ts`（`c10134729d`） | 借鉴提交前审查和完整发布范围检查。OpenCode 使用独立 Git snapshot 存储与会话回退；Agena 使用项目本身的普通 Git 历史。 |
| [GitHub Awesome Copilot 的 Git commit 提示词](https://github.com/github/awesome-copilot/blob/143a3d976b3c1603cc8932984d5e1f28501cb5fc/skills/git-commit/SKILL.md) | 借鉴按逻辑分组、消息解释意图、检查敏感文件和 hooks 失败处理。未安装或执行该 skill。 |
| [Aider Git integration](https://aider.chat/docs/git.html)、[提交消息提示词](https://github.com/Aider-AI/aider/blob/main/aider/prompts.py) | 借鉴本地提交便于审查和恢复；不采用每次编辑都提交、自动提交用户原有脏文件或 blanket hard reset。 |
| [Pro Git：Contributing to a Project](https://git-scm.com/book/en/v2/Distributed-Git-Contributing-to-a-Project) | 每个提交是独立逻辑改动，提交前检查 diff，发布前整理适合合并的提交。 |
| [Pro Git：Rewriting History](https://git-scm.com/book/en/v2/Git-Tools-Rewriting-History)、[Undoing Things](https://git-scm.com/book/en/v2/Git-Basics-Undoing-Things) | 区分本地和已发布历史，明确从未提交的内容可能无法恢复。页面访问受限时核对官方 [progit2 源码](https://github.com/progit/progit2/tree/main/book)。 |
| [git-revert](https://git-scm.com/docs/git-revert)、[git-push](https://git-scm.com/docs/git-push)、[git-worktree](https://git-scm.com/docs/git-worktree) | 撤销生成新提交；授权改写远端时使用绑定已观察 OID 的 lease；worktree 从提交创建且不包含其他工作树未提交改动。 |

## 验证

自动测试验证真实补丁工具输出仍可解码和渲染，以及普通 Git 可以恢复 add/update/move/delete 和补丁工具之外的写入，同时保留无关的用户暂存及未跟踪文件。已有补丁校验、原子写入和失败清理测试继续执行。

```sh
cargo test --locked -p agena-tool -p agena-runtime-contracts -p agena-runtime-tools --lib
cargo test --locked -p agena-runtime-session --lib
cargo test --locked -p agena-bundled-plugins --test tool_correctness --test human_rendering --test docs_reference --test capability_manifest
```

提示词的行为审查应覆盖以下情境；这些检查点不表示静态测试可以证明模型一定遵守：

| 情境 | 预期行为 |
| --- | --- |
| 干净仓库中的已验证小功能 | 创建本地逻辑提交，报告哈希，不 push |
| 用户明确要求不要 commit | 保留未提交改动 |
| 已有无关修改、暂存内容或未跟踪文件 | 保持不动，只处理任务范围 |
| 同一文件混有用户改动，无法精确分离 | 用询问工具决定如何处理 |
| 新 worktree | 从明确提交创建，后续操作保持在该路径 |
| 无仓库或 Git 不可用 | 用询问工具选择范围或接受限制 |
| hook 拒绝提交 | 修复、重新检查暂存内容、新建提交 |
| 请求 push 且有多个同一功能的零碎提交 | 先通过工具确认 squash 或保留；只推送授权目标 |
| 已有发布历史 | 默认增加后续提交；不为美化历史自行 force push |
| 请求危险操作但未批准具体范围 | 工具确认目标、影响和替代方案 |
| 决策已明确或询问没有回答 | 前者不重复询问；后者不视为批准 |
