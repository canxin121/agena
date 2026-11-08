# Agena 助手：人格、记忆、主动交互与持续工作

Web 和 TUI 使用同一套 Agena Runtime。日常入口是普通聊天：助手负责维护偏好、记忆、提醒和后台工作，不要求用户创建或维护 Task。

## 直接聊天

```text
以后叫你小鱼，叫我阿林，用中文直接一点。
记住：我的报告先给结论，再列行动项。
纠正一下，周报应先列行动项。
删除刚才保存的报告偏好。

20 分钟后提醒我核对结果。
未来三小时，每小时主动问一次这个任务的进展。
晚上十点到早上八点不要主动打扰我，时区用 Asia/Shanghai。
先暂停主动提醒。
取消刚才那条提醒。

检查测试失败的原因，能修就修，并告诉我验证结果。
再补一下取消请求的测试。
先停下刚才的检查。
```

明确的多步骤工作由 `agena.work` 委派到独立执行会话；前台仍可以聊天。结果、补充要求和后台提问回到原会话。权限审批仍由用户通过原聊天中的真实审批卡片操作。详细工作流见 [自然聊天持续工作](conversation-work.md)。

## 人格与长期记忆

人格配置包含助手名字、对用户的称呼、表达风格、回复语言和补充表达要求。配置按工作区保存到实例 SQLite，在后续模型请求中生效，同一工作区的新会话可以复用，切换工作区不会自动带入另一工作区的个人信息。

人格属于表达偏好，不提升工具权限，不替代系统约束，也不要求模型伪装成人类。

记忆由 `agena.memory` 统一管理，分类为事实、偏好、经历、方法和当前任务。用户要求“记住”时，助手通过 `memory.write` 显式保存；后台按空闲和证据条件整理有用经历，压缩时保留当前会话任务检查点。详情见 [分层记忆](memory.md)。

自动检索会优先加载当前会话任务检查点和重要偏好，再补充与本次问题相关的证据片段，并按上下文预算裁剪。召回内容被标记为上下文数据，不能作为创建任务、批准权限或安排提醒的新授权。

纠正记忆时先读取当前条目，并使用 SHA-256 版本更新；删除同样绑定当前版本。模型得到成功回执后才能说“已经记住”或“已经删除”。删除会移除实际保存的条目及索引引用，不只是口头承诺不再提及；已有聊天记录不随之删除。

关闭记忆开关会停止自动召回、后台整理和模型的 `memory.write` 写入。已保存条目仍可由用户查看、纠正或删除。记忆关闭不等于删除；管理页会显示这个状态。

凭据保存在 Secret Store；长期记忆用于任务知识、偏好和有来源的经验。

## 主动提醒与跟进

主动联系包括一次性提醒和用户明确授权的有限次数周期跟进。默认开关允许这样的安排，但不自动创建任何安排，不因普通闲聊就开始定期打扰用户。

相对时间由服务端时钟计算；绝对时间使用带明确 UTC 偏移的 RFC3339 时间，并在模型上下文中提供当前 UTC 与工作区 IANA 时区。时间含义存在实质歧义时应澄清。周期联系最短间隔一小时，必须有有限次数，最多 365 次；一次性或首次到期时间应在未来一年内。

每个工作区可以配置主动联系开关、免打扰时段、每天上限和最短联系间隔。默认时区为 UTC、每天上限 4 次、最短间隔 300 秒；免打扰默认关闭。配置时区后，跨午夜、夏令时和每天限额按该时区计算。

到期提醒会等到原会话空闲、没有待处理权限请求且符合当前策略后发出。关闭客户端不删除提醒；Agena 服务必须运行，或稍后重新启动。重启时错过的周期会合并到下一个未来时点，不会集中补发所有错过的次数。

“跟进”可以读取已有工作的当前状态并提出相应问题，但不代表已授权新的文件修改或新任务。收到定时通知的模型回合是独立的控制续接；`companion.manage` 会拒绝把旧用户消息当作修改人格或再次安排提醒的授权。其他已有工具仍按各自的运行时权限和模型使用规则执行。

入队和会话唤醒在同一个内核事务中完成，重复回执使用同一业务身份；暂停、取消和策略检查串行协调。已经交给会话执行的提醒无法撤回，尚未交付的安排可以暂停、恢复或取消。

## Web 与 TUI 管理

管理页面用于检查和纠正，不是使用这些能力的前置步骤。

Web 聊天标题旁有“助手设置”，也可输入 `/assistant`。页面分为人格与主动交互、长期记忆、提醒与主动跟进三个页签，提供真实保存条目的修改/删除、提醒暂停/恢复/取消、时区、免打扰、每日限额和全局开关。窄屏保留同一表单和确认流程。

TUI 输入 `/assistant` 打开原生面板：

| 按键 | 操作 |
|---|---|
| `1` / `2` / `3` 或 `Tab` | 切换人格、记忆、提醒 |
| `↑/↓` 或 `j/k` | 选择条目 |
| `Enter` | 编辑文本或切换选项 |
| `s` | 保存人格/主动联系配置 |
| `r` | 刷新；发生版本冲突后重新读取再确认 |
| `e` / `d` | 编辑/确认删除所选记忆 |
| `p` / `c` | 暂停或恢复/确认取消所选提醒 |
| `Ctrl+Enter` 或 `Alt+Enter` | 保存文本编辑 |
| `Esc` | 关闭编辑或返回聊天 |

两个客户端共享服务端类型和版本检查。网络回执丢失时，配置保存保留同一请求标识；发生冲突时不会自动替用户重写最新配置。保存会使旧的在途读取失效，避免成功保存后被迟到响应回退。

管理页使用有界记忆预览，最多显示 100 条、单条正文最多 16 KiB。超出预览预算的条目通过现有完整记忆工具管理，不能用截断的正文覆盖原文。

## 执行链与代码

Web 和 TUI 的普通聊天通过以下组件执行：

| 组件 | 职责 |
| --- | --- |
| `agena.companion` | 人格、经授权的提醒和跟进 |
| `agena.memory` | 记忆查询、读取、写入、纠正、删除和自动召回 |
| `agena.work` | 识别交办、关联原工作、跟进和自动回报 |
| Agena Runtime | Session、权限、后台记忆整理、工具、Subagent 和 MCP |

主要代码位置：

| 层次 | 位置 |
|---|---|
| 人格和提醒领域、时间策略 | `crates/agena-superagent/src/companion.rs` |
| 持久配置、回执、提醒、配额 | `crates/agena-storage-sqlite/src/companion_repository.rs` |
| 运行时操作与主动投递 | `crates/agena-runtime/src/runtime/superagent/companion.rs` |
| 自然聊天工具与提示 | `crates/agena-bundled-plugins/src/plugins/provided/companion.rs`、`assets/prompts/companion.md` |
| 权限感知记忆 | `crates/agena-bundled-plugins/src/memory/plugin.rs` |
| 原子提醒投递 | `SessionManager::deliver_companion_notice`、`ExecutorCommands::deliver_and_resume` |
| Web 页面 | `packages/agena-web/src/features/assistant/` |
| TUI 面板 | `crates/agena-tui-app/src/app_assistant.rs` |

认证管理 API 使用已有实例访问控制：

```text
GET  /api/v1/sessions/{source}/assistant
PUT  /api/v1/sessions/{source}/assistant
POST /api/v1/sessions/{source}/assistant/memory
POST /api/v1/sessions/{source}/assistant/reminders/{id}
```

模型调用 `companion.manage` 时不能自己选择任意来源会话。运行时从真实调用上下文取得用户会话与用户消息，写操作需要当前用户原文引文。这个检查证明来源和回合，不是对所有自然语言意图的形式化证明；模型仍需判断请求含义并遵守提示与权限。

数据库由现有实例所有者统一迁移，只接受明确识别的完整旧基线，不在各 Repository 中偷偷建表。升级真实实例前需要保留备份；本轮开发测试只操作临时数据库。

## 验证入口

```sh
cargo test -p agena-superagent --lib --offline --locked
cargo test -p agena-storage-sqlite --test companion --test superagent_tasks --offline --locked
cargo test -p agena-tui-app app_assistant --lib --offline --locked
cargo test -p agena --test system scenarios::assistant --offline --locked -- --test-threads=1

# 真实 Chrome + Vue、真实 tmux + TUI，使用独立测试实例
cargo test -p agena --test system assistant_personality_memory_and_proactive_contact \
  --offline --locked -- --ignored --test-threads=1 --nocapture

cd packages/agena-web
bun test ./tests
bun run build
```

真实界面用例从普通聊天修改人格、保存记忆和安排主动提醒，再进入两端管理界面编辑/删除实际条目、取消提醒并验证重载。底层用例覆盖跨工作区隔离、记忆开关、旧版本冲突、过期回执、免打扰、夏令时、跨日限频、有限周期合并、暂停取消以及进程重启。

测试模型是本地脚本化服务，验证真实产品在相应模型决策下的行为，不代表已完成不同真实模型的意图识别质量评测。自然交办仍由模型理解；执行、权限、状态、时间、版本和投递由宿主系统约束。

本轮不接 QQ 和飞书。主动回报的目标是原 Agena 会话，不是外部渠道或手机系统推送。现有单实例、单操作者和顶层任务保守串行策略保持不变。

## 本轮实际验证记录

| 验证 | 结果 |
|---|---|
| 完整产品系统测试 | 112 通过、0 失败；6 个真实 UI 用例另行执行 |
| 三组真实 Web/TUI 流程 | 6 通过：任务面板、自然持续工作、完整助手；包含真实 Chrome、私有 tmux 和产品服务 |
| 最终助手界面复验 | TUI 通过；Web 增加旧快照回放、并发配置冲突、保持其他新字段、刷新后状态恢复后再次通过 |
| 全部内置插件行为 | 76 通过，包括真实 Agent 生命周期、MCP 握手/发现/调用、记忆权限与 Cron 投递 |
| 原生库回归 | 9 个库测试集共 500 通过，1 项既有测试默认忽略；后续新增 TUI 冲突保护专项 3 项复验通过 |
| 存储集成 | 73 通过，1 项既有测试默认忽略；其中助手存储、策略和原子投递专项 7 项复验通过 |
| 最终自然助手核心流程 | 5 通过，含跨会话人格、工作区隔离、记忆纠正/关闭/删除、提醒重启和授权拒绝 |
| Web 行为测试 | 315 通过、0 失败 |
| Web 生产构建、类型、模块与语言检查 | 通过 |
| 相关 Rust 严格 Clippy | 通过，使用 `-D warnings` |

不同测试集存在重叠，不能把这些数字简单相加当作独立功能覆盖率。系统测试中的真实 UI 用例默认忽略，是因为需要 Chrome/tmux；本轮已经用 `--ignored` 实际运行，而不是只登记。

验收中发现并修复了设置保存与迟到读取的竞争：草稿保留原始版本，只提交真正修改的字段，冲突后明确重新确认，不会用旧表单覆盖其他并发更新。浏览器刷新探针也已改成等待新文档加载，避免误读即将卸载的旧 DOM。

可追溯日志位于开发主机 `/tmp/agena-assistant-final/`，汇总为 `validation-summary.json`。`real-ui.log`、`assistant-ui-final.log`、`web-assistant-verified.log` 包含实际界面证据目录。旧失败日志保留用于审计，以最终复验结果为准。

没有提交或推送 Git，没有重启用户现有服务，也没有迁移真实会话数据库。启用这些能力需要运行包含本工作区改动的新构建；默认插件 `agena.companion` 与 `agena.work` 随该构建加载。


## 插件接入与外部事件

聊天渠道与事件源现可通过 `agena.integration` v1 插件服务扩展。`work.connections` 可以在自然聊天中发现插件、提出规则草稿或暂停规则；Web/TUI `/integrations` 提供一次性授权确认和事件/投递审计。GitHub 新 Issue 及签名聊天桥的独立进程参考实现见 `examples/integration-plugins/`。完整契约、安装和安全边界见 `docs/integrations.md`。这增加的是通用扩展层，不表示已经连接用户真实 GitHub 或实现 QQ/飞书专用协议。
