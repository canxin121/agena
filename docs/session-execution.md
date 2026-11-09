# 统一项目会话执行

项目工作由 Session Kernel 承载。AI Bot 和用户操作同一份项目会话：创建、发送消息、继续、读取结果、停止执行。内部并行分工使用现有 Subagent，状态、工具输出、权限卡片和消息顺序都属于原生会话。

## Bot 与项目作用域

Bot 是实例级全局会话，项目关联和默认工作目录都为空；普通项目会话关联真实的已登记项目。全局会话的文件与命令操作必须明确路径基准，权限仍按实例与会话规则审查，不继承当前项目的规则、提示词或记忆。Bot 管理某个项目时只选择目标，不把自己的作用域改成那个项目。

## 组件职责

| 组件 | 职责 |
|---|---|
| `agena.bot` | `bot.sessions` 操作项目与普通会话；`bot.connections` 管理接入草稿 |
| Session Kernel | 原生执行身份、执行代次、消息队列、幂等回执、取消与恢复 |
| Subagent | 项目会话内的委派、进度、结果和取消 |
| `agena-assistant` / `AssistantRuntime` | 人格、提醒、接入授权与渠道收发 |
| 现有 SQLite 实例 | 保存会话、助手配置、规则、原生回执和事件审计 |

Bot 投递使用 `bot_instruction`；授权事件使用 `integration_instruction`；提醒和事件通知分别使用 `assistant_notice`、`integration_notice`。这些来源都是有明确身份的系统续接，不伪造新的用户输入。涉及用户授权的修改继续检查真实当前用户消息。

事件规则的 `session` 模式把执行投递到规则选定的普通项目会话，或明确授权的渠道服务主体会话。原生执行回执同时记录会话、执行 ID 和代次。规则暂停、到期、撤权或超时只取消对应执行，用户随后发起的人工工作继续保留。

## 升级既有数据库

新数据库只创建当前功能使用的表。既有实例在安装新版前使用独立的一次性本地迁移工具，先复制并校验完整数据库，再变换旧结构和保存历史数据；部分安装或外来结构会被拒绝。正式服务启动只接受当前结构，不会自动处理历史数据格式。

旧任务数据的历史迁移属于一次性离线操作，迁移工具不留在正式运行时中。它将旧业务记录写入现有实例状态的审计归档后删除退役表与索引。项目会话、聊天正文、Bot 身份、模型选择和文件/网络权限继续保留。

执行回收根据旧记录中的执行 ID、代次及可核对的提交回执判断归属。已经由用户接续的较新执行和权限卡片保留；不能仅凭会话曾经关联旧任务就取消它。无法确认的旧工作和调度记录暂停并保留诊断，避免重复产生外部效果。

曾采用旧自动任务模式的规则转换为会话规则后保持未启用，需要在 `/integrations` 重新核对授权。普通提醒和通知继续保留；已投递但结果不确定的旧提醒发生次数会归档并暂停，避免换一种通知身份重复发送。

当前运行时没有旧任务控制器、任务仓储、任务工具、任务工作台或旧数据库升级兼容模块。历史数据转换只在离线升级工具中执行，不提供旧接口或工具别名。

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

静态检查覆盖退役目录、生产类型、工具名、接口和生成清单。存储测试覆盖无项目会话的创建与继承、全局 Bot 分支、删除祖先后的权限边界、事务回滚与重启。历史数据库的一次性转换通过独立迁移程序在复制的真实数据库及异常结构夹具上验收；正式运行时代码不保留旧结构支持。实际产品测试覆盖 Bot、原生会话、Subagent、事件接入及 Web/TUI。
