# Agena TUI Roadmap

## 目标

把 `agena-tui` 从“可用的终端聊天界面”升级成“长期可用的终端开发工作台”。

参考对象：

- Codex CLI
- OpenCode
- Claude Code

设计原则：

- 先补统一交互层，再补单点功能。
- 优先做高频路径，而不是做零散按钮。
- 尽量复用现有 `App` 状态机、overlay 和 backend 能力，避免推倒重来。

## Phase 1

状态：已完成

目标：建立统一命令层，让后续功能有统一入口。

已实现：

- slash commands
- command palette
- provider picker
- model picker
- runtime overrides
  - model
  - provider default model
  - temperature
  - max output tokens
  - system prompt
- child session fork
- child session picker
- parent session navigation
- 中英文 discoverability 文案补齐

当前命令集：

- `/help`
- `/commands`
- `/new`
- `/sessions`
- `/resume`
- `/lineage`
- `/rewind`
- `/search`
- `/find`
- `/rename`
- `/timeline`
- `/export`
- `/pager`
- `/continue`
- `/user-input`
- `/allow`
- `/allow-always`
- `/deny`
- `/deny-always`
- `/attach`
- `/editor`
- `/image`
- `/copy`
- `/copy-visible`
- `/providers`
- `/provider`
- `/models`
- `/model`
- `/temperature`
- `/max-output`
- `/system`
- `/fork`
- `/children`
- `/parent`
- `/status`

说明：

- `/sessions` 现在支持 `all|roots|subtree` 视图参数

## Phase 2

状态：进行中

目标：补齐会话生命周期，接近 Codex/OpenCode/Claude Code 的日常使用体验。

已实现：

- session rename
- session timeline overlay
- session export to markdown and open in editor
- session list parent/child affordance
- session pane all / roots / subtree branch views
- tree-aware session search that preserves ancestors
- transcript header parent/child summary
- richer transcript/session chrome with context summaries
- parent/child session hotkeys (`[` / `]`)
- per-session draft persistence and recovery
- transcript pager mode
- richer resume flows with a global session switcher
- session lineage / branch-history picker
- branch-aware transcript header, status line, and session-pane tags
- rename/timeline/export discoverability
- message rewind / backtrack picker with confirmation flow
- timeline event → transcript message jump
- unified blocked / awaiting-model / permission / user-input status summary
- timeline jump discoverability in transcript help/footer/status copy

剩余计划项：
- none in this phase

## Phase 3

状态：下一阶段

目标：补齐多 agent / 多线程工作流。

计划项：

- primary agent switcher
- subagent invocation
- agent-aware permission and status display
- agent-specific runtime overrides

## Phase 4

状态：下一阶段

目标：补齐治理与生态面板。

计划项：

- richer permission inspection and policy summary
- MCP server/resource browser
- plugin and skill browser
- LSP status and diagnostics surface
- runtime status panel
- config and auth surfaces for providers

## Phase 5

状态：下一阶段

目标：补齐产品化和个性化能力。

计划项：

- theme system
- keybinding customization
- layout options
- statusline customization
- diff view modes
- terminal mode options
  - alternate screen policy
  - inline mode

## 详细执行计划

下面这部分不是“愿望清单”，而是按依赖关系拆开的执行顺序。目标不是一次把所有功能都做完，而是保证每一轮都能沿着同一条主线，把 `agena-tui` 持续推进成真正可长期使用的终端开发工作台。

### Workstream A: 会话生命周期与分支工作流

1. branch history / lineage 可视化
   - 新增 lineage picker 或 overlay，围绕当前会话展示 root 到 current 的祖先链。
   - 在同一视图里显式标记 current、ancestor、sibling、child、leaf。
   - 展示每个节点的 session id、标题、更新时间、消息数、child 数量。
   - 支持直接从 lineage 视图跳转到任意祖先或兄弟分支。
   - 这一步是后续 rewind/backtrack 和多 agent 分支工作的基础。

2. branch affordance 强化
   - 在 transcript header 和 status line 中加入 lineage 摘要。
   - 在 session pane 当前选中项上增加更明显的 branch 标识。
   - 让 `[`、`]`、`s`、`/resume`、`/children`、`/parent` 形成闭环，而不是分散快捷键。

3. rewind / backtrack API 设计与前端接线
   - 先明确 Agenta session service 是否需要新增“truncate to event/message”的后端接口。
   - 如果需要后端变更，优先做最小接口：
     - rewind 到某条 message
     - rewind 到某个 event seq
   - UI 层需要预留 command、picker、confirmation overlay 和 flash。

4. rewind confirmation 流程
   - 在 TUI 中加入二次确认，明确提示会丢弃哪一段后续消息。
   - 如果当前 composer 有草稿，先走草稿保存。
   - rewind 成功后自动刷新 transcript、timeline 和 session tree。

5. rewind 之后的 fork / continue 策略
   - 允许 rewind 后继续在原分支执行。
   - 允许 rewind 后先 fork 再继续，保留原分支完整历史。
   - UI 上要让“在原分支继续”和“从这里开新分支”都是显式动作。

6. transcript pager mode 第二阶段
   - 支持 pager 打开 markdown 版 transcript，而不只是 plain text。
   - 支持带 event timeline 的 pager export。
   - 让 pager 模式在长对话、日志密集型 session 中成为一等路径。

7. transcript viewport / jump affordance
   - 加强顶部和底部的 jump 状态提示。
   - 支持从 timeline event 定位到相关消息区间。
   - 让 transcript 不只是“滚动窗口”，而是可导航的信息面板。

### Workstream B: Agent / Thread 工作台

8. primary agent switcher
   - 当 session 存在明确的 agent / role 语义时，在 header 中显示。
   - 允许快速切换“当前主要 agent 视图”。
   - 需要避免把 session tree 和 agent tree 混为一谈，因此先做单一 agent summary，再扩展多 agent。

9. subagent invocation surface
   - 在 slash command 和 palette 中引入 subagent 入口。
   - 先不做复杂 orchestration，先打通“创建 / 查看 / 切换 subagent session”的最短路径。
   - 保持和主 session 共享 branch affordance。

10. agent-aware session chrome
    - 在 transcript header 标出当前 session 是 primary、child branch 还是 subagent。
    - 权限请求、运行中状态、用户输入请求都要带上 agent 上下文。

11. agent-specific runtime overrides
    - provider/model/temperature/system 等 override 需要能看出是 session 级还是 agent 级。
    - 如果同一工作流中存在多个 agent，要能快速辨认谁在使用什么模型。

12. agent timeline / activity summary
    - 让 timeline 不只显示事件，还能按 agent 汇总。
    - 后续可扩展成并行 agent activity 条。

### Workstream C: 权限、输入与恢复

13. richer permission inspection
    - 当前只有快速 allow/deny，需要一个可读的 permission summary panel。
    - 面板要能看到请求原因、工具、路径范围、历史决策。

14. permission policy summary
    - 区分 allow once / allow always / deny once / deny always 的当前有效状态。
    - 在 status/header 中显式显示 session 是否处于“策略已放宽”的模式。

15. richer user-input surface
    - 目前 user-input overlay 可回复，但 discoverability 和上下文仍弱。
    - 需要在 transcript/header/status 中明确显示当前待回复问题数、问题摘要。

16. resume / blocked-state flows
    - 统一 blocked、awaiting model、awaiting permission、awaiting user input 的状态表达。
    - 让 resume picker 可按“blocked only / active only / all”过滤。
    - 后续把 `r` 从“盲 continue”升级为“上下文明确的恢复动作”。

### Workstream D: 生态与工具面板

17. runtime status panel
    - 把当前 provider/model/system/temperature/max-output 聚合成独立面板。
    - 避免状态只能靠 flash 或 composer title 瞥一眼。

18. MCP browser
    - 先做 server 列表，再做 resource / template drill-down。
    - 不急着把所有操作塞进 TUI，先把“看见和理解”做出来。

19. plugin / skill browser
    - 展示已安装 plugin / skill，以及它们的简述、来源和启用状态。
    - 后续才考虑安装、启停等写操作。

20. diagnostics / LSP status surface
    - 先从只读 summary 做起，不碰复杂编辑器集成。
    - 目标是让用户知道工作区当前有没有明显问题，而不是在黑盒里操作。

21. provider auth / config surface
    - 至少能在 TUI 内看见 provider 是否可用、缺少哪些凭据、当前加载了什么配置模式。

### Workstream E: 产品化与可配置性

22. theme system
    - 抽离统一配色 tokens，避免继续在组件里散落 `Color::Rgb(...)`。
    - 先支持 2-3 套清晰风格，不追求开放式主题市场。

23. keybinding customization
    - 先把关键动作收敛成动作名，再做 keymap。
    - 不先做动作抽象，后面改键位会非常痛苦。

24. layout options
    - 至少支持 session pane 宽度、header 高度、composer 最大高度配置。
    - 后续再考虑左右反转、隐藏 pane、单列模式。

25. statusline customization
    - 把当前上下文摘要、focus hint、flash 三者拆成可配置槽位。
    - 让用户可以选择更偏“IDE”还是更偏“CLI”的状态栏风格。

26. alternate screen / inline mode policy
    - 当前默认 alternate screen，后续需要支持 inline mode。
    - pager/export/editor 等外部动作的 terminal suspend/resume 逻辑要保持一致。

## 里程碑顺序

推荐按下面顺序推进，而不是看到哪里缺就补哪里：

1. branch history / lineage 可视化
2. branch affordance 强化
3. rewind / backtrack 最小闭环
4. blocked / resume flows 强化
5. runtime status panel
6. primary agent switcher
7. subagent invocation surface
8. agent-aware chrome 与 timeline summary
9. permission inspection / policy summary
10. MCP / plugin / skill browser
11. diagnostics / provider auth surfaces
12. theme / keymap / layout / statusline customization

## 每个里程碑的完成标准

每一轮功能不按“代码写完”定义完成，而按下面标准验收：

- 有清晰入口：
  - 至少同时具备 command、palette discoverability、必要时的热键。
- 有状态表达：
  - header、status、overlay 文案中至少有一处能解释当前状态。
- 有失败路径：
  - 网络失败、数据为空、上下文不满足时有明确 flash 或空态文案。
- 有恢复路径：
  - 不要求每次都成功，但要求失败后用户知道接下来该做什么。
- 有测试与验证：
  - 至少补纯函数 / command parser / selector 这类低成本单测。
  - 每轮都跑 `cargo fmt --all`、`cargo check -p agena-tui`、`cargo test -p agena-tui -- --nocapture`。

## 当前下一步

如果严格按依赖顺序继续，接下来最该做的是：

1. `message rewind / backtrack`

原因：

- 会话树现在已经具备 session pane、parent/child 导航、resume picker、lineage picker、timeline、pager、draft persistence，以及 header/status/session pane 级别的 branch affordance。
- 继续加零散命令的收益会明显下降。
- branch 与 rewind 做完之后，`agena-tui` 才算真正进入接近 Codex / Claude Code 的“长期工作台”阶段。

## 验收标准

- 不再依赖记忆大量单键快捷键才能使用核心功能。
- provider/model/session tree 都能在 TUI 内直接发现和切换。
- 用户能通过命令层完成 80% 以上高频操作。
- 新功能优先通过统一 picker / command palette 暴露，而不是继续增加零散快捷键。
