# 统一项目会话执行

项目工作由 Session Kernel 承载。AI Bot 和用户操作同一份项目会话：创建、发送消息、继续、读取结果、停止执行。内部并行分工使用现有 Subagent，状态、工具输出、权限卡片和消息顺序都属于原生会话。

## Bot 与项目作用域

Bot 是实例级全局会话，项目关联和默认工作目录都为空；普通项目会话关联真实的已登记项目。全局会话的文件与命令操作必须明确路径基准，权限仍按实例与会话规则审查，不继承当前项目的规则、提示词或记忆。Bot 管理某个项目时只选择目标，不把自己的作用域改成那个项目。

## 组件职责

| 组件 | 职责 |
|---|---|
| `agena.bot` | `bot.profile` 修改当前 Bot 的资料，`bot.sessions` 操作项目与普通会话，`bot.connections` 管理接入草稿 |
| Session Kernel | 原生执行身份、执行代次、消息队列、幂等回执、取消与恢复 |
| Subagent | 项目会话内的委派、进度、结果和取消 |
| `agena-assistant` / `AssistantRuntime` | 记忆设置、提醒、接入授权与渠道收发 |
| 现有 SQLite 实例 | 保存会话、助手配置、规则、原生回执和事件审计 |

Bot 投递使用 `bot_instruction`；授权事件使用 `integration_instruction`；提醒和事件通知分别使用 `assistant_notice`、`integration_notice`。这些来源都是有明确身份的系统续接，不伪造新的用户输入。涉及用户授权的修改继续检查真实当前用户消息。

事件规则的 `session` 模式把执行投递到规则选定的普通项目会话，或明确授权的渠道服务主体会话。原生执行回执同时记录会话、执行 ID 和代次。规则暂停、到期、撤权或超时只取消对应执行，用户随后发起的人工工作继续保留。

## 数据库基线

SQLite 实例在首次启动时创建完整架构。已有数据必须匹配当前基线。使用空的 `--data-dir` 启动新基线时，原目录可以继续用于保存旧实例。

## 用量与费用

`/api/v1/usage` 汇总实例内的全部持久化用量，按页读取所有工作区和全局（Bot）会话。Web 与 TUI 使用同一份报表。`/api/v1/sessions/{id}/cost` 提供单个会话的模型、Token 与费用明细；Web 输入框上方的上下文进度可点击展开，显示当前上下文占用与该会话的历史 Token 与费用。

单条损坏或版本较新的 round 状态只跳过该条记录，不会让整个报表失败。

会话作用域设置（模型配置、权限、提示词层）使用同一套页面：作用域选择「当前会话」后，页面顶部的会话选择器默认最近使用的会话，也可切换并写入 `?sessionId=`。会话模型写入 `/api/v1/sessions/{id}/selection`，是会话自身的持久选择。

## 提示词前缀与缓存命中率

提供商通过逐字节相同的提示词前缀复用缓存。Companion 提供固定的提醒指引，模型通过 `companion.manage` 的 `profile` 或 `reminders` 动作读取实时钟、策略版本和提醒状态。具名 Bot 的附加系统指令由 Bot 插件加入对应会话的系统提示。

系统提示只承担模型每轮都需要的规则与稳定事实。工具的输入 schema、插件内部标识与路径、以及任何能通过工具重新读到的数据都不写进系统提示：执行契约由 `tools_help` 提供（`tools_call` 本来就必须先读到它），实时状态由返回它的工具提供。Agent roster 同理只保留身份与成员，不写会逐轮变化的执行状态。自动召回的记忆插在当前请求之前，而不是消息列表开头，这样本轮召回变化不会改写此前已缓存的整段对话。

当工具脚本 `tool_script.run` 出现在当前会话的工具目录时，内置系统提示会引导模型用它处理需要循环、分支、筛选或汇总的重复调用和依赖调用。模型先通过 `tools_help` 读取契约，再调用脚本。脚本里的 `tools.call` 沿用会话权限。单个简单操作，以及需要用户交互、流式输出或启动后台任务的工具，使用直接调用。

诊断方法：同一会话两轮请求的 `instructions` 应当完全一致；若不同，先比较差异位置，落在时钟或日程字段上即为前缀失效来源。

## 清空会话

`/clear`（别名 `/reset`）把一段对话重新开始，并提供两种模式。两者都不删除任何已存储的数据：条目、内容块与 segment 行都保留，可在需要时回读。

| 模式 | 对话记录 | 模型上下文 |
|---|---|---|
| 清空历史（默认） | 不再显示旧轮次 | 不再包含旧轮次 |
| 仅清空上下文 | 仍可阅读旧轮次 | 不再包含旧轮次 |

「清空历史」和 `truncate` 一样把 segment 退役到 `agena_retired_session_segments`，因此快照仍能寻址旧历史，只有在没有快照引用时才由回收流程释放。「仅清空上下文」不改动 membership，而是在当前尾部安装一个空检查点，把 prompt window 推过去；这条检查点不调用模型，也不会写入摘要文本。

两种模式都会先结算排队与运行中的执行，避免仍在运行的连接把输出写到新边界之后。Web 端 `/clear` 与侧栏会话菜单都会弹出模式选择框，TUI 同样先选模式再确认。

## 回归检查

```sh
python3 scripts/ci/check-task-retirement.py
python3 scripts/ci/check-global-bot-scope.py
cargo test -p agena-storage-sqlite --test global_scope --offline --locked -- --test-threads=1
cargo test -p agena --test system --offline --locked -- --test-threads=2
cargo test -p agena --test system scenarios::assistant_ui --offline --locked -- --ignored --test-threads=1
cargo test -p agena --test system scenarios::bot_operator_ui --offline --locked -- --ignored --test-threads=1
cargo test -p agena --test system scenarios::bot_profiles_ui --offline --locked -- --ignored --test-threads=1
```

静态检查覆盖退役目录、生产类型、工具名、接口和生成清单。存储测试覆盖无项目会话的创建与继承、全局 Bot 分支、删除祖先后的权限边界、事务回滚与重启。数据库基线测试验证不兼容文件会被拒绝，并保留原始数据库、WAL 和恢复日志。实际产品测试覆盖 Bot、原生会话、Subagent、事件接入及 Web/TUI。
