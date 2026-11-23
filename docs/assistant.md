# Agena 助手的记忆与提醒

Web 和 TUI 使用同一套 Agena Runtime。可以直接在普通聊天里管理记忆、安排提醒和继续项目工作。

## 直接聊天

```text
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

AI Bot 通过独立的 `/bot` 聊天入口管理项目和普通会话：查询项目、新建会话、继续已有对话、查看进度或停止执行。Web 与 TUI 共用同一个 Bot 对话。项目会话直接执行工作并复用现有 Subagent，你可以随时从项目列表打开并继续。权限审批保留原生卡片。详细用法见 [AI Bot](ai-bot.md)。

## 系统提示词与长期记忆

普通会话的助手名称、称呼、语气和语言由配置的系统提示词决定。具名 Bot 可以通过 `bot.profile` 更新自己的名称和附加系统指令。详见 [系统提示词配置](prompt-layers.md) 和 [AI Bot](ai-bot.md)。

记忆由 `agena.memory` 统一管理，分类为事实、偏好、经历、方法和当前任务。用户要求“记住”时，助手通过 `memory.write` 显式保存；后台按空闲和证据条件整理有用经历，压缩时保留当前会话任务检查点。详情见 [分层记忆](memory.md)。

自动检索会优先加载当前会话任务检查点和重要偏好，再补充与本次问题相关的证据片段，并按上下文预算裁剪。召回内容被标记为上下文数据，不能作为创建任务、批准权限或安排提醒的新授权。

纠正记忆时先读取当前条目，并使用 SHA-256 版本更新；删除同样绑定当前版本。模型得到成功回执后才能说“已经记住”或“已经删除”。删除会移除实际保存的条目及索引引用，不只是口头承诺不再提及；已有聊天记录不随之删除。

关闭记忆开关会停止自动召回、后台整理和模型的 `memory.write` 写入。已保存条目仍可由用户查看、纠正或删除。记忆关闭不等于删除；管理页会显示这个状态。

凭据保存在 Secret Store；长期记忆用于任务知识、偏好和有来源的经验。

## 主动提醒与跟进

主动联系包括一次性提醒和用户明确授权的有限次数周期跟进。默认开关允许这样的安排，但不自动创建任何安排，不因普通闲聊就开始定期打扰用户。

相对时间由服务端时钟计算。绝对时间使用带明确 UTC 偏移的 RFC3339 时间。模型可以通过 `companion.manage` 的 `profile` 动作读取当前 UTC 时间和工作区 IANA 时区。时间含义存在实质歧义时应澄清。周期联系最短间隔一小时，最多 365 次。首次到期时间应在未来一年内。

每个工作区可以配置主动联系开关、免打扰时段、每天上限和最短联系间隔。默认时区为 UTC、每天上限 4 次、最短间隔 300 秒；免打扰默认关闭。配置时区后，跨午夜、夏令时和每天限额按该时区计算。

到期提醒会等到原会话空闲、没有待处理权限请求且符合当前策略后发出。关闭客户端不删除提醒；Agena 服务必须运行，或稍后重新启动。重启时错过的周期会合并到下一个未来时点，不会集中补发所有错过的次数。

“跟进”可以读取已有工作的当前状态并提出相应问题。定时通知会在原会话中启动一个专门的投递回合。修改设置或安排新的提醒需要本轮用户的明确请求。其他工具按各自的运行时权限和模型使用规则执行。

入队和会话唤醒在同一个内核事务中完成，重复回执使用同一业务身份；暂停、取消和策略检查串行协调。已经交给会话执行的提醒无法撤回，尚未交付的安排可以暂停、恢复或取消。

## Web 与 TUI 管理

管理页面用于检查和纠正，不是使用这些能力的前置步骤。

在 Web 的普通会话中输入 `/assistant`，打开「设置」「长期记忆」和「提醒与主动跟进」三个页签。可以设置自动记忆召回、提醒开关、时区、免打扰和频率限制，也可以编辑或删除已保存的记忆，暂停、恢复或取消提醒。

TUI 输入 `/assistant` 打开原生面板：

| 按键 | 操作 |
|---|---|
| `1` / `2` / `3` 或 `Tab` | 切换设置、记忆、提醒 |
| `↑/↓` 或 `j/k` | 选择条目 |
| `Enter` | 编辑文本或切换选项 |
| `s` | 保存自动召回和提醒设置 |
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
| `agena.companion` | 记忆召回设置、提醒设置、经授权的提醒和跟进 |
| `agena.memory` | 记忆查询、读取、写入、纠正、删除和自动召回 |
| `agena.bot` | 修改当前 Bot 的资料、管理普通会话和接入草稿 |
| Agena Runtime | Session、权限、后台记忆整理、工具、Subagent 和 MCP |

主要代码位置：

| 层次 | 位置 |
|---|---|
| 记忆与提醒设置、时间策略 | `crates/agena-assistant/src/companion.rs` |
| 持久配置、回执、提醒、配额 | `crates/agena-storage-sqlite/src/companion_repository.rs` |
| 运行时操作与主动投递 | `crates/agena-runtime/src/runtime/assistant/companion.rs` |
| 自然聊天工具与提示 | `crates/agena-bundled-plugins/src/plugins/provided/companion.rs`、`assets/prompts/companion.md` |
| 权限感知记忆 | `crates/agena-bundled-plugins/src/memory/plugin.rs` |
| 原子提醒投递 | `SessionManager::deliver_companion_notice`、`ExecutorCommands::deliver_control_notice` |
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

升级后会从已有 SQLite 记录读取记忆和提醒设置。旧记录中的普通会话表达偏好已退出运行时配置。

## 验证入口

```sh
cargo test -p agena-assistant --lib --offline --locked
cargo test -p agena-storage-sqlite --test companion --offline --locked
cargo test -p agena-tui-app app_assistant --lib --offline --locked
cargo test -p agena --test system scenarios::assistant --offline --locked -- --test-threads=1

# 真实 Chrome + Vue、真实 tmux + TUI，使用独立测试实例
cargo test -p agena --test system assistant_memory_and_proactive_contact \
  --offline --locked -- --ignored --test-threads=1 --nocapture

cd packages/agena-web
bun test ./tests
bun run build
```

真实界面用例从普通聊天修改提醒时区、保存记忆和安排提醒，再进入两端管理界面编辑或删除条目、取消提醒并验证重载。底层用例覆盖跨工作区隔离、记忆开关、版本冲突、旧回执重放、免打扰、夏令时、跨日限频、有限周期合并、暂停取消以及进程重启。

测试模型是本地脚本化服务，验证真实产品在相应模型决策下的行为，不代表已完成不同真实模型的意图识别质量评测。自然交办仍由模型理解；执行、权限、状态、时间、版本和投递由宿主系统约束。

本轮不接 QQ 和飞书。主动回报的目标是原 Agena 会话，不是外部渠道或手机系统推送。项目执行使用原有会话队列与并发边界。

## 插件接入与外部事件

聊天渠道与事件源现可通过 `agena.integration` v1 插件服务扩展。`bot.connections` 可以在自然聊天中发现插件、提出规则草稿或暂停规则；Web/TUI `/integrations` 提供一次性授权确认和事件/投递审计。GitHub 新 Issue 及签名聊天桥的独立进程参考实现见 `examples/integration-plugins/`。完整契约、安装和安全边界见 `docs/integrations.md`。这增加的是通用扩展层，不表示已经连接用户真实 GitHub 或实现 QQ/飞书专用协议。
