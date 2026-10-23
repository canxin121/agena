# Agent 会话与通信

Agena 的协作身份是持久化 session。`agent_id` 就是 session 的数字 ID；`name` 在同一个父会话下唯一。一次执行的 UUID 只供运行时使用，继续工作沿用原来的 agent 身份。

## 工具

| 工具 | 父会话 | 子会话 | 行为 |
| --- | --- | --- | --- |
| `agent.spawn` | 有 | 无 | 创建独立上下文的子 agent，持久化启动后立即返回 handle |
| `agent.message` | 有 | 有 | 发送普通消息，不暂停发送者，不启动空闲接收者 |
| `agent.wait` | 有 | 有 | 等待自己的邮箱、新用户输入或超时，仅返回唤醒原因 |
| `agent.followup` | 有 | 无 | 运行时补充指导；空闲时在同一 session 中启动下一次执行 |
| `agent.cancel` | 有 | 无 | 停止直属子 agent 的执行，保留身份与会话 |

上述“有”仍受正常的工具权限和模型可用性限制。子会话还会继承父会话的权限上限、工作目录和能力禁用项。运行时再次检查直属父子关系，不能靠换成别名或直接调用 host API 绕过权限。

`target` 可以使用可见 agent 的名字、数字 ID，子会话也可以使用 `parent`。可见范围是同一个父会话及其直属 agents，允许父子双向通信与兄弟通信。禁止跨组发消息，子会话不能再次创建子 agent。

不提供 list/get/inspect/output/transcript 工具，也不提供单独的 report/ask/complete。子会话正常输出最终回复，运行时自动把结果发给父会话。

## 上下文与投递

父会话历史不会复制给新 agent。调用 spawn 时应在 prompt 中写清范围、约束、已有结论及希望交付的结果；commands 可以附带已有命令的引用。运行时每次调用模型前重新生成简短 roster，只含自身、父会话、同组 agent 的 ID、名称和状态。roster 不进入历史消息，压缩后及重启后均从持久化元数据重建。

消息写入 kernel 的 durable delivery 表。wait 在读取快照前订阅变化，避免“检查后消息刚好到达”的漏唤醒。wait 的返回值只有 `message`、`user_input` 或 `timeout`；消息正文在下一个模型边界转成一次 notice 并消费，不同时从 wait 再返回一遍。新用户输入会唤醒等待，模型应让出当前轮次，以便排队的用户输入执行。

普通消息给空闲 agent 时保留在邮箱，下次显式 followup 会送进它的上下文。普通消息本身不会启动该 agent。followup 给运行中的 agent 时绑定当前执行，和结束屏障共享事务检查；遇到结束竞争就等结算后继续检查，确保指导进入当前执行或下一次执行。

最终状态和父会话邮箱里的结果在同一笔事务中提交。交付按执行 UUID 幂等，重复持久化不会重复投递。进程重启会把未完成的 agent 运行（包括已经排队但尚未开始的启动）标成 interrupted 并补发恢复通知；恢复由父会话显式 followup 决定，避免普通用户输入恢复流程绕过 agent 的预算和最终结果交付。

每个父会话最多保留 8 个可用 agent 身份，优先 followup 复用。单条消息上限 12 KiB，最终回复预览上限 12 KiB；每个邮箱至多积压 32 条普通消息、128 KiB 的普通消息 JSON。超限拒绝新消息，已经接收的消息不覆盖、不丢弃，最终结果仍可提交。完整结果可以由用户打开对应子会话查看。

spawn 指定的超时、token 和费用上限保存在 agent 配置中；后续执行继续使用这些上限，每次执行独立计量。wait 期间释放外层工具调用的并发名额，唤醒后再取得，避免工具并发数为 1 时无法发送解锁消息。

## 代码归属

- `agena-bundled-plugins/.../provided/agent.rs`：薄工具适配层，只调用 HostClient。
- `agena-plugin-sdk/src/host_api.rs`：五个 host 操作的输入和简短回执，调用方身份取自有效 callback 上下文。
- `agena-runtime-session/.../manager/agents.rs` 与 `agents/messaging.rs`：唯一的身份、生命周期、权限、邮箱与 roster 实现。
- `agena-storage-sqlite/.../kernel/lifecycle.rs`：事务化状态和投递、幂等及消息容量边界。

插件不再持有第二份 registry、后台执行 fiber 或 async_tasks 存储。操作界面的 agent 活动从 kernel 配置投影，不再把子会话 transcript 复制成父工具的输出日志。

## 不兼容变更

旧 `tasks.*`、原生 `task`、subtask host RPC、插件私有 async_tasks 存储及旧会话字段均已移除，没有兼容别名。会话字段改为 `agent_name`、`agent_status`、`agent_started_at*`、`agent_finished_at*`、`agent_failure`。

kernel 使用精确 schema 校验。本次变更需要新建数据库路径；旧数据库会被拒绝打开且不会被修改，不进行隐式迁移或删除。旧工具权限配置应按新的 `agena.agent.*` 名称重写。
