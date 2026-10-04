# Git 工作流程与文件恢复

Agena 通过系统提示词指导模型使用项目自己的 Git 仓库保存和恢复文件改动。模型通过现有 shell 工具执行 Git，通过 `interaction.ask` 获取用户决策。没有新增 Git 自动提交服务、隐藏仓库、文件快照日志或命令拦截器。

行为规则的唯一来源是 [`git_workflow.md`](../crates/agena-runtime-contracts/src/identity/git_workflow.md)，由 `identity::system_prompt_with_sections` 注入所有会话。动态询问提示词、询问工具帮助和超时结果采用相同的授权语义。上下文压缩保留工作路径、基线、内容归属、验证对象、Git 操作状态和授权范围。本页说明这些规则的依据和实际用法，不是另一份运行时提示词。

## 核心模型：内容、归属、发布范围

只要求“用 Git、及时 commit、危险时询问”还不够。执行前需要知道将改变什么内容、内容属于谁、操作影响本地还是远端。

| 状态 | 保存什么 | 容易误判的地方 |
| --- | --- | --- |
| HEAD | 当前提交的树 | 给 HEAD 加 recovery 分支只保存这个已提交版本，不包含脏文件 |
| index / 暂存区 | 普通下一次提交的候选树 | 普通 `git commit` 会包含整个 index，不能因为本次只 add 了几个文件就认为只提交这几个文件 |
| working tree / 工作区 | 当前磁盘上的文件 | 普通测试通常读取这里；它可能包含未暂存修复、用户改动或未跟踪依赖，与候选提交不同 |

“这个文件由我修改过”也不意味着“这个文件的全部内容都属于我的任务”。同一文件可以同时包含用户已暂存内容、用户未暂存内容和模型本次编辑。需要保护内容本身，也需要保护用户已有的暂存边界。

默认实践是：复用合适的任务分支，按逻辑工作单元创建本地提交；用户要求或确实需要隔离时创建 worktree；只在得到相应授权后发布；根据仓库集成策略决定是否询问本地 squash。项目或用户另有提交约定时优先遵循。

## 从接手任务到交付

1. **建立上下文。** 检查仓库根、当前分支/worktree、HEAD、status、相关暂存与未暂存 diff、近期提交风格。记录任务开始时的提交和用户已有工作。确认是否有未完成的 merge/rebase/cherry-pick，不自动 abort 或删除 lock。新仓库可能没有 HEAD；submodule、嵌套仓库、sparse checkout 和 shallow history 需要按实际情况处理。
2. **确认基线与路径。** 任务基线、PR 目标分支和 push upstream 不一定相同。新 worktree 从明确提交创建，不含原目录的未提交改动；如果任务依赖那些改动，先解决依赖和归属。后续 shell 调用指定 workdir，文件工具使用对应路径。
3. **修改并建立恢复点。** 无关脏文件不阻塞工作。完成有价值的阶段，或准备进行有风险的实验时，提交自己的任务内容。只读任务、每次工具调用、临时输出和空改动不需要制造提交。
4. **审查实际候选提交。** 检查全部将提交的内容，按明确路径或改动块暂存。普通 commit 收入整个暂存区；带路径的 commit 又有不同语义，见下一节。不要自动暂存、stash、取消暂存或提交用户的其他工作。
5. **验证并提交。** 明确测试读取的内容是否等于待提交内容。按顺序执行暂存、审查、commit、检查新 HEAD 和剩余状态。保留 hooks 和签名；错误或超时后先检查实际状态，不盲目重试。提交后若 hook 改写了相关内容，应重新检查和验证受影响部分。
6. **交付或发布。** 报告 worktree、branch、提交、验证范围、遗留事项和 push 状态。没有发布授权就停在本地。保留用于交付的分支/worktree，不为了表面整洁自动合回原目录或删除恢复引用。

只需在接手/恢复任务和即将 commit、恢复、改写或发布等边界刷新相关证据。每次文件编辑前全仓扫描、对普通提交重复询问，都没有必要。

## 阶段提交与交付提交

阶段提交用于保存有价值的进度，允许记录尚未完成的任务，但要明确哪些检查没有运行或仍失败。它不表示任务完成，也不意味着可以跳过拒绝提交的 hook。等所有工作完全正确才允许 commit，会让长时间探索一直没有恢复点。

交付提交用于审查、集成和日后定位问题。一个逻辑变化应同时包含实现、相关测试和必需的 generated/lockfile 更新；按文件类型把实现和测试拆开，常常会制造不可独立使用的中间提交。多个独立且可理解的逻辑变化可以保留多个提交，不以提交数量或文件数量判断好坏。

普通提交前核对 `git diff --cached` 只能说明 index 的内容。使用 `git commit --only -- <paths>` 时，Git 取这些路径的当前工作区内容，也会收入未暂存改动。只有选中路径的全部内容都属于本次提交时，路径提交才适合用来保留其他路径的用户暂存。对同一文件的混合归属，不能把 `--only` 当作自动隔离工具。

若部分暂存导致测试内容和候选提交不同，应在合适的隔离环境验证候选树，或者明确报告验证缺口。不要为了得到干净工作区而擅自 stash 用户改动，也不要宣称工作区测试通过就证明提交本身通过。

## Worktree 的实际边界

Worktree 隔离工作目录和 index，但共享对象库、分支引用以及通常的仓库配置。一个 worktree 内仍应有一个 Git 修改协调者：Git 自己的锁不能把“读取状态 → 暂存 → 审查 → 提交”整个过程变成事务。其他执行者修改内容后，需要重新检查。

Agena 的文件工具把相对路径解析到当前会话工作区。shell 的 `cd` 或某次调用的 workdir 不会改变后续文件工具的路径基准。编辑另一个 worktree 时，应在运行时允许的范围内使用明确的绝对文件路径；每次 shell 调用同样指定 workdir。`fs.apply_patch` 的帮助已说明这一点，回归测试用两个包含同名文件的目录验证绝对路径修改不会重新绑定后续相对路径。

不要把 `.git` 必须是目录当作仓库判断条件：linked worktree 和 submodule 常使用 `.git` 文件。也不要为了测试或创建恢复点，在新 worktree 中修改签名、hooks、身份等共享配置。

## 什么时候询问

已知工具先通过 `tools_help` 读取实时契约，再通过 `tools_call` 调用。询问针对具体未决事项，同一范围已明确授权时不重复询问。

| 情境 | 默认动作 / 用户决策 |
| --- | --- |
| 普通编辑、只读 Git 检查、自己的阶段/交付提交 | 在任务授权内继续 |
| 有无关的用户修改、暂存或未跟踪文件 | 保留它们，继续处理任务范围 |
| 精确撤销自己刚做、且确认没有混入其他工作的改动 | 在任务范围内直接处理，不为每次纠正再问 |
| 同一文件混合归属，无法安全分离内容或暂存边界 | `interaction.ask` 决定范围、隔离方式或先保留未提交 |
| 要丢弃用户工作、删除未合并分支/脏 worktree、amend/rebase/squash 或强推，缺少具体授权 | `interaction.ask` 展示路径/提交范围/远端、影响和保留现状的选项 |
| 多个未发布阶段提交属于同一变化，且适合本地 squash | `interaction.ask` 给出精确范围、建议消息，以及 squash / 保留 / 暂缓 |
| 新的长期维护项目没有 Git 保护 | 尚未决定时询问一次：正确的 init 范围或接受没有 Git 恢复；不要为每个临时产物询问 |
| 需求、方向或偏好不清楚 | `interaction.ask` |
| 计划审批 | `plan.review`；一般计划批准不自动扩大为具体危险操作或发布授权 |
| 超时、取消、空答案、推荐默认项 | 不构成批准，只继续独立且已授权的工作 |
| 询问工具不可用 | 保留待决操作并报告工具限制，不用聊天问题代替 |

模型不能以一句“是否继续？”结束回合等待聊天回复。依赖用户决定的操作必须等待工具结果。需要确认时给用户看已经明确的操作范围和后果，而不是泛泛要求为整个任务背书。

## 发布与 squash

不主动 push。用户要求“创建/打开 PR”通常已经授权完成该 PR 所必需的分支发布；“准备 PR 描述”没有这个含义。具体发布授权仍不包含 merge、删除远端分支或改写共享历史。已有相同目标和范围的授权持续有效。

发布前同时检查本次 PR 的逻辑变化和实际将发送的历史。任务基线不能一律用 upstream 替代：upstream 可能就是已经发布过的 topic 分支。需要刷新远端证据时 fetch，不用隐式 pull、merge 或 rebase 代替判断。远端已有额外提交时，先看清分叉和归属。

`origin` 只是配置名。用 `git remote get-url --push --all <remote>` 等证据确认实际 push 目的地，并检查 follow-tags、mirror、submodule push 设置。即使指定 `HEAD:refs/heads/topic`，多个 push URL 和 follow-tags 仍可能让操作超出单分支、单远端的预期。限制的是实际端点和引用范围，而不是命令字符串看起来是否显式；无需为此偷偷修改用户的 Git 配置。

| 提交/集成形态 | 建议 |
| --- | --- |
| 一个逻辑变化只有一个完整提交 | 按已有发布授权继续，无需询问 squash |
| 一项变化包含多个未发布 WIP/fixup，目标需要整理过的线性历史 | 询问是否 squash 明确的范围 |
| 仓库在 PR 合并时 squash，当前分支有可追踪的阶段提交 | 可保留分支提交，由集成步骤生成最终提交；无需重复本地改写 |
| 多个独立且有用的逻辑提交 | 保留提交边界 |
| 已发布/多人共享分支、stacked PR 或包含 merge 的范围 | 不套用统一 reset/rebase 配方；检查拓扑和影响，任何改写按具体范围授权 |

纯 squash 前保证任务工作区/index 干净、保存旧 tip，完成后比较新旧 tree，文件内容应相同。保存一个 recovery ref 不能保存未提交内容；这部分要单独保护。涉及冲突解决或内容变化时重新验证，而不是仅比较提交数量。

授权替换远端历史时，优先使用绑定已观察提交的 `--force-with-lease=<ref>:<expected-oid>`。普通 `--force-with-lease` 依赖远端跟踪引用，后台 fetch 可能使保护失效。显式 lease 也只解决并发变化：它不代表可以丢弃已经观察到的远端提交。lease 失败时重新评估，不能刷新期望值后机械重试。网络报错或超时也先核对远端状态。

Squash merge 后，下一项任务从更新后的目标分支创建新 topic。直接延用旧 topic 会保留旧 merge base，使新 PR 再次显示已经集成的提交，甚至出现重复冲突。

## 可复现的 Git 语义实验

[`scripts/experiments/git_workflow.py`](../scripts/experiments/git_workflow.py) 用标准库创建临时仓库、hooks 和本地 bare remotes，复现容易误判的命令行为。它不读取个人/系统 Git 配置，不改动项目仓库，不向网络远端发布。测试夹具的身份和签名设置只应用于临时 Git 命令，不能照搬成产品绕过检查的默认行为。

| 实验 | 已验证现象 | 对行为规则的影响 |
| --- | --- | --- |
| 路径提交 | `commit --only -- file` 同时收进该文件未暂存内容 | 明确路径不等于隔离了用户改动 |
| 普通提交 | 新增任务文件后 commit 仍收进预先暂存的用户文件 | 每次检查全部候选提交 |
| 验证对象 | 工作区测试通过，暂存树的同一测试失败 | 将验证结论绑定到实际内容 |
| Worktree | 新目录有独立 index、不含脏改动、`.git` 是文件，但配置共享 | 区分内容隔离和仓库共享状态 |
| Hooks | pre-commit 拒绝时 HEAD 不变；post-commit 报错后提交已存在；post-checkout 非零退出时分支已改变 | 报错后读取事实，不把报错等同于未发生操作 |
| Recovery ref | 新引用仍指向旧提交，不含脏文件内容 | recovery ref 只保护已提交历史 |
| 干净 squash | 临时干净分支压缩后 tree 相同，旧 tip 仍可经引用访问 | 验证内容不变；实验配方不适用于任意真实拓扑 |
| Lease | 后台 fetch 后默认 lease 允许覆盖新远端提交，绑定旧 OID 的 lease 拒绝 | 保留审批时观察到的远端版本，失败后重新判断 |
| 发布范围 | 显式分支 refspec 仍向两个 push URL 发布，并跟随注解 tag | 检查端点、附带 refs 和配置 |
| Squash 后续任务 | 延用旧 topic 使旧提交和变化再次出现在新范围中 | 新任务从更新后的基线出发 |

运行：

```sh
python3 scripts/experiments/git_workflow.py
```

2026-10-04 在 Git `2.54.0 (Apple Git-157)`、Python `3.9.6` 下，10 个实验全部通过。POSIX hook 实验在 Windows 跳过。这些是 Git 语义实验，不是模型行为评测，也不构成所有版本和配置下的安全保证。

## 能力边界

Git 保护实际记录过的内容。未提交、未跟踪、被忽略或仓库外的文件不能保证恢复；reflog 会过期，也不是永久备份。Git 提交不能撤销数据库、部署或其他外部副作用。聊天 `rewind` / `fork` 只改变会话历史，不恢复文件。

`fs.apply_patch` 输出操作标识、文件变更、diff 和进度，不生成或持久化 `inverse_patch`、`before_hash`、`after_hash`。应用前校验、文件锁、原子替换和失败调用内的尽力清理继续负责单次写入的正确性；它们不是跨回合的撤销历史。现有 snapshot/worktree 能力仍可用于隔离，不承担自动恢复承诺。

这是一套模型行为规则，现有运行时权限继续生效；没有新增保证模型绝不执行危险命令的运行时边界。提示词保持一份紧凑规则，命令细节、依据和实验留在文档中，避免把每个边缘案例都扩展成独立的审批流程。

## 调研依据与取舍

调研日期：2026-10-04。先检索公开提示词，再读取原文，核对本机安装包和相邻仓库；对第二轮发现的 Git 语义使用官方文档和临时仓库实验交叉验证。

| 来源 | 借鉴与取舍 |
| --- | --- |
| 本机 Claude Code `2.1.288`，安装包内 `Git Safety Protocol`；`../claude-code` 的 `src/tools/BashTool/prompt.ts`、`src/tools/AskUserQuestionTool/prompt.ts`（`a371abb`） | 借鉴精确暂存、保留 hooks、pre-commit 失败后新建提交、禁止隐式危险操作。其默认“明确要求才提交”改为 Agena 的任务里程碑本地提交策略；不把 pre-commit 结论泛化到所有 hooks。 |
| `../codex/codex-rs/core/gpt-5.2-codex_prompt.md`（`774d6425`） | 借鉴保护已有改动、避免未经授权的 amend/reset。无关改动继续工作；真正需要决定时调用询问工具。 |
| `../opencode/packages/opencode/src/tool/shell/shell.txt`、`src/snapshot/index.ts`、`src/session/revert.ts`（`c10134729d`） | 借鉴提交前审查和完整发布范围检查。OpenCode 有独立 Git snapshot 存储；Agena 使用项目本身的普通 Git 历史。 |
| [GitHub Awesome Copilot Git commit 提示词](https://github.com/github/awesome-copilot/blob/143a3d976b3c1603cc8932984d5e1f28501cb5fc/skills/git-commit/SKILL.md) | 借鉴按逻辑分组、消息解释意图、检查敏感文件。未安装或执行该 skill。 |
| [Aider Git integration](https://aider.chat/docs/git.html)、[repo.py](https://github.com/Aider-AI/aider/blob/main/aider/repo.py)、[base_coder.py](https://github.com/Aider-AI/aider/blob/main/aider/coders/base_coder.py) | 编辑后和 lint 后均可有自动提交，说明恢复点不等于交付完成；不采用自动提交用户脏文件、默认细碎提交或绕过 hooks 的做法。源码按调研日读取。 |
| [git-commit](https://git-scm.com/docs/git-commit)、[git-status](https://git-scm.com/docs/git-status)、[git-restore](https://git-scm.com/docs/git-restore)、[git-reset](https://git-scm.com/docs/git-reset) | 核对 HEAD/index/working tree、路径提交、恢复范围和 reset 模式的实际语义。 |
| [githooks](https://git-scm.com/docs/githooks)、[git-revert](https://git-scm.com/docs/git-revert) | 区分 pre/post hooks 的失败结果；`revert -n` 可以在不等于 HEAD 的 index 上工作，不能忽略既有暂存内容。 |
| [git-worktree](https://git-scm.com/docs/git-worktree)、[gitworkflows](https://git-scm.com/docs/gitworkflows) | worktree 的隔离/共享边界；独立主题、集成方向和有意义的提交单元。 |
| [git-push](https://git-scm.com/docs/git-push)、[git-remote](https://git-scm.com/docs/git-remote) | 多 push URL、附带 tags、lease 与后台 fetch、远端更新的实际范围。 |
| [Google Engineering Practices: Small CLs](https://google.github.io/eng-practices/review/developer/small-cls.html) | 小而自洽的一项变化，同时包含相关测试；不是机械按文件数拆分。 |
| [GitHub: About pull request merges](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/incorporating-changes-from-a-pull-request/about-pull-request-merges) | squash merge 更适合短期分支；延用旧 head 分支会重复显示提交并增加冲突。 |
| [Pro Git: Contributing to a Project](https://git-scm.com/book/en/v2/Distributed-Git-Contributing-to-a-Project)、[Rewriting History](https://git-scm.com/book/en/v2/Git-Tools-Rewriting-History)、[Undoing Things](https://git-scm.com/book/en/v2/Git-Basics-Undoing-Things) | 逻辑提交、发布前审查、本地/共享历史和未记录内容的恢复边界。页面受限时核对官方 [progit2 源码](https://github.com/progit/progit2/tree/main/book)。 |

## 实现验证与后续行为评测

Rust 测试验证提示词装配、补丁工具契约与路径解析，以及普通 Git 能恢复 add/update/move/delete 和工具之外的写入，同时保留无关用户暂存和未跟踪文件。原有补丁校验、原子写入和失败清理测试继续保留。

```sh
cargo test --locked -p agena-runtime-contracts --lib
cargo test --locked -p agena-runtime-session --lib
cargo test --locked -p agena-bundled-plugins --test tool_correctness --test docs_reference --test capability_manifest
cargo fmt --all -- --check
```

下一步若评估模型实际遵循程度，应在临时仓库中观察完整工具轨迹和最终 Git 状态，至少覆盖以下任务，而不是只检查模型有没有复述规则。本次没有运行真实模型行为评测。

| 任务场景 | 应观察的行为 |
| --- | --- |
| 干净项目实现小功能 | 相关验证、本地逻辑提交、不自动 push |
| 用户要求不 commit | 保留未提交内容，不重复询问 |
| 用户同一文件有 staged/unstaged 改动 | 内容与暂存边界不被误收；必要时用询问工具解决 |
| 任务依赖源 worktree 的脏文件 | 不直接在遗漏依赖的新 worktree 宣称完成 |
| Shell 切到新 worktree 后使用文件工具 | 实际文件修改落在预定目录 |
| 工作区修复未暂存、候选提交仍失败 | 识别验证对象差异，不误报提交通过 |
| pre-commit 拒绝、post-commit 报错或命令超时 | 先检查实际 HEAD/index/status，再决定重试 |
| 长任务尚未完成或检查失败 | 可保留诚实的阶段提交，不宣称交付完成或绕过 hook |
| 允许 push，多个阶段提交，仓库 squash merge | 根据集成策略决定是否有必要本地 squash |
| 多 push URL、follow-tags、远端已前进 | 有效发布范围不超出授权；不盲目强推 |
| 已批准具体操作，或询问超时 | 已批准的不重复问；超时的不执行依赖操作 |
| 中途压缩上下文后继续 | 仍知道路径、内容归属、验证对象和精确授权范围 |
