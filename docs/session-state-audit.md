# 会话状态、分支上下文与恢复审计

日期：2026-11-01。审计基线：`aa47c881a`（`master`）。本文记录这次代码审计、修复决策和可重复验证方式。

## 范围与判定标准

范围包括 root、child、fork、fork 的 fork、rewind、subagent，以及它们的工具执行、交互等待、排队、取消、继续、compact、服务重启和归档恢复。独立工作的含义是：执行、队列、交互和代理任务按所属会话隔离；继承历史本身只提供读取上下文，不授予操作祖先或其他会话执行的权限。显式授权的父代理管理操作仍按既有权限规则执行。

测试使用真实 SQLite、真实会话管理器和 HTTP 服务，模型端使用可控制的测试 provider。HTTP 工具测试实际执行文件读取及权限回复。所有服务和数据库均放在测试临时目录，不重启已有服务，不修改已有用户会话数据。

## 状态模型

这些维度必须分开理解；单个 `running` 或 `ready` 无法表达全部含义。

| 维度 | 定义/取值 | 所回答的问题 |
| --- | --- | --- |
| 会话关系 | `SessionRelationKind`: Root、Child、Fork、Rewind、Subagent | 会话如何产生、父节点是谁、是否有代理角色？ |
| 创建/可用性 | `SessionLifecycleState`: Creating、Ready、Failed | 会话是否已建立并可使用？当前内核采用事务内创建。 |
| 客户端处理状态 | `SessionStateKind`: Creating、Ready、Running、AwaitingInteraction、Failed | 当前是否有本会话的执行、交互或失败需要呈现？ |
| 持久化执行状态 | `session_kernel::ExecutionPhase`: Queued、Running、Succeeded、Failed、Cancelled、Interrupted | 某个 execution 的某代 attempt 处于什么状态？ |
| 实时执行阶段 | `execution::ExecutionPhase`: Starting、PreparingModel、StreamingModel、ExecutingTools、AwaitingInteraction、Cancelling | 正在运行的监督器当前执行到哪一步？ |
| 工具/交互工作流 | `WorkflowState`: Quiescent、ToolPending、AwaitingInteraction | 当前工具工作是否等待执行或用户交互？Quiescent 本身不表示模型必然空闲。 |
| 代理任务状态 | `AgentStatus`: Created、Running、Completed、Failed、Cancelled、TimedOut、Interrupted | 被委派的任务是否完成、取消、超时或被重启打断？ |
| 执行用途 | `ExecutionPurpose`: Message、Continuation、Compaction | 排队和重启恢复时，这次执行应处理用户消息、继续上下文还是压缩上下文？ |
| 实时触发来源 | `ExecutionSource`: User、Continue、Compaction、PermissionReply、UserInputReply | 当前执行由哪种请求触发或唤醒？这不能替代持久化的执行用途。 |

`execution.rs` 和 `session_kernel.rs` 都有名为 `ExecutionPhase` 的类型，含义不同。投影时不能把实时阶段、持久化结果或代理状态互相替代。历史条目也可以记录“分支创建瞬间，祖先工具仍在运行”，而分支自身的当前处理状态为 Ready；历史事实不构成本分支的在途工作。

历史资源还保留各自的状态：`ToolPhase` 为 Preparing、Running、Succeeded、Failed、Cancelled、Interrupted；`ProviderRoundPhase` 为 Running、Succeeded、Failed、Interrupted；`CaptureState` 为 Open、Complete、Interrupted。`ToolExecutionKind` 区分 Runtime 与 ProviderHosted，后者记录 provider 自己执行的工作，不应被当作本地工具调度请求。分支快照冻结这些资源在截点的可见事实，同时保留会话自身独立的当前状态。

下面几种组合是正常状态，不能通过强制统一某个字段来“修复”：

| 组合 | 含义 |
| --- | --- |
| fork 当前 Ready，继承工具历史为 Running | 历史记录的是分支截点，分支没有继承源工具的执行所有权。 |
| subagent 当前 Ready，代理任务为 Completed | 当前没有执行，上一轮受委派任务已经结束。 |
| subagent 正在 compact，代理任务仍为原终态 | 维护上下文和重新执行委派任务是不同用途，不应产生新的任务结果。 |
| 会话有队列且 `message_queue_paused=true` | 暂停是调度配置，不能仅凭队列不为空推导模型正在运行。 |
| 模型 StreamingModel，workflow 为 Quiescent | workflow 描述工具/交互工作；此时模型仍可以正在生成。 |

上下文继承与消息传递也不同：fork/rewind 继承选定的历史边界；`agent.spawn` 不复制父会话聊天历史，子代理从明确提供的任务 prompt 开始，并通过受控 mailbox 向父会话发送结果。共同 workspace 只确定工具使用的目录，不让一个会话获得另一个会话的执行权限。

## 已确认的问题与修复

### 1. 工作流投影和工作目录

原 `get_session` 分别读取 metadata、summary、configuration 和交互，并将没有待回复交互的工作流统一视为 Quiescent。工具执行中会因此被错误投影为静止。现在使用同一个 `control_snapshot`，并统一从待处理交互和实时 ExecutingTools 阶段推导工作流。

当 `effective_workspace_root` 未显式配置时，工具执行器原先可能沿用服务默认工作目录。修复后回退到会话所属 workspace 的目录；显式目录覆盖仍保留。运行时和应用 API 的 effective workspace 投影采用同一规则。真实 HTTP 文件读取回归先复现了读到错误 workspace 内容，再验证修复后 root 和二级 fork 均读到自己的文件。

主要代码：`crates/agena-runtime-session/src/session/manager/queries.rs`、`crates/agena-application/src/service/execution.rs`。

### 2. 分支继承暂停状态与不合法父关系

原 fork 克隆配置时也复制 `message_queue_paused`，导致来自暂停会话的新分支无法正常启动。新 fork 和 rewind 现在有独立、未暂停的队列；源会话的暂停状态保持原值。

空 root 原来即使已有 child，仍可能被挂到该 child 下面，构成环，或让既有后代的 root/depth 失效。首次绑定父节点现在要求会话没有历史条目、没有既有父节点，也没有子节点，保留创建新 child/subagent 的正常路径。

主要代码：`crates/agena-storage-sqlite/src/kernel/lifecycle.rs`。

### 3. 无本地用户消息的 fork 无法 compact 或 Continue

原入口依赖最新的本地用户消息。刚创建的 fork 和二级 fork 只有继承历史，因而无法 compact/Continue。借用祖先 execution 做 Retry 会破坏所有权，复制一条伪用户消息又会改变用户历史。

修复采用本会话独立的控制执行，并使用有明确用途的本地 notice 作为 FIFO 边界。控制执行有自己的 execution ID、attempt 和递增 message ID；不会增加用户消息计数，也不修改祖先 attempt。compact 始终使用 Compaction；没有可重试本地输入的继续操作使用 Continuation。

普通用户消息的既有重试保留 generation 递增语义。如果其后已经出现控制执行，则继续操作创建新的 Continuation，避免较旧 message ID 把后续上下文排除，或让“最新执行”仍指向之前的 compact。存储层在同一事务内再做一次检查，防止后台通知改变历史尾部造成竞态。

应用层可编辑消息队列只呈现 Message；compact 控制 notice 不进入被压缩的模型输入。恢复、通知消费和归档都保留明确用途。

主要代码：storage kernel 的 `ExecutionPurpose`/`ControlExecutionPurpose`、`lifecycle.rs`、`input_window.rs`，以及 runtime 的 `execution.rs`、`deliveries.rs`。

### 4. compact 的用途与预算边界

原 compact 标志仅存在于监督器内存中，排队后重启会按普通对话恢复。现在新提交 compact 的用途保存在 `agena_execution_purposes`，队列恢复和监督器均据此执行维护操作。没有用途记录的旧 execution 默认按 Message 解释；持久用途恢复适用于新版本提交并保存了用途的 compact。

重启时，运行中 subagent 的任务和未执行的委派消息仍按原恢复策略变成 Interrupted；明确排队的 Compaction 被保留，恢复后不会重启原委派任务，也不会把维护结果当成 agent_result 发给父会话。

压缩预算回退原先可能把候选窗口减到 1，而存储接口要求 2–10000。最小值现在保持 2。过小模型预算会返回真正的 PromptBudgetExceeded，并保留已有上下文/checkpoint，不会变成无关的参数错误。

### 5. fork 创建后仍读取祖先的可变内容

原 fork 固定了 segment membership，却继续从共享条目读取最新执行状态、工具结果和内容流。真实 HTTP 回归复现：在 BEFORE 后 fork，父会话继续输出 AFTER，子会话也会读到 AFTER。只复制历史条目 ID 不足以提供上下文隔离。

现在 fork 在同一个写事务内提交源会话已经接受的流尾部，再保存以下小型快照：

- 条目的 revision、execution generation/phase、响应边界、内容/压缩状态，以及工具和交互控制事实。
- 每个可见内容源的已提交 head、epoch、保留起点和丢失计数。
- 创建时可见的工具输出、交互、活动和 blob 身份集合。
- provider round 的阶段与 replay 是否已存在，后台活动的结果是否已存在。

用户文档、工具正文、provider replay 和内容块继续复用原存储；快照限制分支可见的版本、字段和内容边界，无需复制整份正文。二级 fork 复制已存在的快照，不能重新采集祖先的最新状态。

历史分页、单条读取、execution/tool/interaction/round/activity 读取、模型输入选择、round input capture、文本、日志、文档快照、capture loss、watch、blob 授权及归档恢复均遵守分支边界。父会话后续增加的 source、blob、交互或活动不会因旧工具条目的 membership 而自动向分支开放。用户输入请求后来的 presentation acknowledgement 也不会进入分支。

源会话重启可能改变 open stream 的 epoch。只有数据库中可信的分支 head 可以授权旧 epoch 的已提交前缀；调用方自行构造的更大 head 仍会被拒绝。显式内容保留/清理造成的数据缺口仍报告 ContentGap，不会回退到更新的父流。分支 watch 不订阅父会话实时输出。

主要代码：`crates/agena-storage-sqlite/src/kernel/branches.rs` 及 reads、content、prompt、input_window、archive/import。

### 6. subagent 直接接收消息时绕过生命周期和预算

专用 spawn/followup 原有监督路径维护代理状态和预算；直接向 subagent 发送消息或 Continue 原先走通用执行路径，可能显示旧 Completed/Interrupted，并绕过已保存的 token、cost、timeout 限制。

现在通用执行在成功 claim 后识别 subagent，继承已保存限制，提交新的 Running 身份，并在执行结束后结算。维护 compact 不改变代理任务结果，也不投递 agent_result。完成 provider round 后立即释放其 prompt/stream 资源预算，避免在后续工具或 agent.wait 阶段继续占用模型运行容量。

专用 launch 和直接执行之间仅等待真实存在的旧结算任务；不因数据库中的 Running 字样无限等待。结算注册具有 drop 清理和通知，避免任务退出后遗留等待。结果投递和幂等键包含 attempt generation，Continue 不再与前一代结算冲突。provider/tool panic 会先取消并隔离 attempt，再完成代理状态与父会话通知。

主要代码：`crates/agena-runtime-session/src/session/manager/agents.rs`、`execution.rs`、`mod.rs`。

## 保持的权限与角色规则

fork 和 child 拥有独立执行权限；继承条目只提供历史读取权限。subagent 不能继续祖先 execution，不能回复祖先待处理交互。一个会话的取消不取消兄弟或后代 fork 的执行。

已有角色限制保持有效：subagent 不能再次 spawn 子代理，也不能直接向用户发起需要独立用户交互的工具请求。`fork(subagent)` 成为 Fork 并清除代理名字和任务状态，同时保留继承的 permission ceiling 和 capability denials。这里的“工具正常工作”是指在该角色的权限范围内正常工作。

同一 workspace 的多个会话仍共享真实文件系统。会话上下文和执行隔离不等于磁盘快照；一个会话改文件后，同目录中的另一个会话可以读到新文件内容。

## 数据库与归档兼容

新增表采用精确识别旧 schema 后的追加迁移，不修改旧表声明。只允许完整匹配已知前一基线的数据库升级；未知、缺项或部分改变的 schema 继续拒绝，且只读预检不修改源文件。新表在一个事务中安装。

原版本未保存旧 fork 的精确流截点。升级只能在 recovery 修改 open 状态之前，冻结这些分支当前仍持久保存、可以读取的事实；无法找回进程退出时丢失的内存尾部，也无法可靠重建未记录的原始分支截点。同样，旧版本排队 compact 的用途没有持久化，不能事后从普通 Retry/Continue 中无歧义恢复，没有用途记录的旧执行仍按 Message 解释。新版本创建的分支和新提交的控制执行会保存相应快照与用途。

归档携带用途与分支快照，并对 entry、execution、round、content、activity、interaction、blob 和 epoch 做身份映射和边界验证。恢复后的 snapshot 仍然只读，不获得旧进程的执行能力。现有归档格式仍可保存共享 source 的完整底层内容块；冻结的是导入后的会话视图，不应将归档原始文件当作对分支截点以后字节做过脱敏的文件。

普通 blob 读取按分支创建时的身份集合授权，即使调用方知道祖先后来发布的 blob ID，也不能越过冻结边界。归档的内部 snapshot reader 则保留源记录所需的完整依赖，并继续检查 snapshot membership、真实引用、ready 状态、长度和 hash；否则会因原始工具 payload 引用后来出现的附件而导出失败。

补充回归还发现单独的 `entry_blobs` 列表接口遗漏了同一过滤。工具详情中的附件列表、独立附件列表和实际 `open_blob` 授权现在都遵守相同的冻结事实，避免列表泄漏祖先后来新增的附件身份。

控制执行使用 Notice 起点，因此归档导出 attempts、provider rounds 和 round inputs 的条件也扩展到有明确 purpose 的控制起点。只在 `entry.id == entry.message_id` 时导出，避免后续 Notice/Text 重复导出同一 execution。补充回归覆盖 Continuation 已经产生正文和 provider state 后的导出、导入、再次导出及 prompt 读取。

## 回归与运行方式

新增覆盖位于：

- `crates/agena-runtime-session/src/session/manager/queries/tests.rs`
- `crates/agena-runtime-session/src/session/manager/execution/compaction_tests.rs`
- `crates/agena-runtime-session/src/session/manager/agents/tests.rs`
- `crates/agena-storage-sqlite/tests/session_branches.rs`
- `crates/agena-storage-sqlite/tests/branch_content.rs`
- `crates/agena-storage-sqlite/tests/branch_facts.rs`
- `crates/agena-storage-sqlite/tests/branch_blobs.rs`
- `crates/agena-storage-sqlite/tests/session_recovery.rs`
- `scripts/agena/tests/test_server_lifecycle.py` 的 `SessionRestartTests`

HTTP fixture 同时实现流式 Responses SSE 和非流式 Responses JSON。测试模型明确设置 `native_compaction: false`，因此 compact 场景验证实际的本地 summarizer 请求、checkpoint 和后续上下文选择。真实外部 provider 的原生 compact API 不包含在这些 HTTP 用例的覆盖声明中。

```bash
cargo test --locked --offline --no-fail-fast \
  -p agena-domain -p agena-storage-sqlite -p agena-runtime-session \
  -p agena-runtime-tools -p agena-application -p agena-api --lib --tests
cargo build --locked --offline -p agena --bin agena
AGENA_TEST_BINARY=/absolute/path/to/current/agena \
AGENA_TEST_PREVIOUS_BINARY=/absolute/path/to/previous/agena \
PYTHONDONTWRITEBYTECODE=1 \
python3 scripts/agena/tests/test_server_lifecycle.py SessionRestartTests -v
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
python3 scripts/ci/verify-runtime-capabilities.py
```

旧库升级 HTTP 回归使用旧可执行文件真正创建数据库与 fork，再由新程序打开，核对 instance identity 和历史不变，并执行二级 fork 的 compact/Continue。未提供 `AGENA_TEST_PREVIOUS_BINARY` 时，仅该升级用例跳过。

最终验证结果：

| 检查 | 结果 |
| --- | --- |
| 六个相关 crate 的 `--lib --tests` 回归 | 223 passed、0 failed；1 个 ignored 为由 crash recovery 测试调用的 `crash_fixture_child` 子进程入口。 |
| 归档补充回归 | `branch_blobs` 1 项与 `session_branches` 5 项通过，并已纳入上述 223 项。 |
| 当前程序构建 | `cargo build --locked --offline -p agena --bin agena` 通过。 |
| Rust 格式 | `cargo fmt --all -- --check` 通过。 |
| 全工作区静态检查 | `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` 通过。 |
| Runtime 能力门禁 | `CARGO_NET_OFFLINE=true python3 scripts/ci/verify-runtime-capabilities.py` 通过。 |
| HTTP 会话场景 | 最终完整套件 9 passed、0 failed、0 skipped，43.776 秒；包括真实 agent 工具链与真正旧数据库升级。 |

旧库升级用例已实际使用前一版本可执行文件创建数据库，而非在新库上删除表模拟旧 schema。HTTP 运行期间发现并修正了 fixture 的 `part_id` 身份字段和非流式响应协议，保留原有上下文、所有权、消息计数和重启断言。

| HTTP 场景 | 实际验证 |
| --- | --- |
| 崩溃后首次读取 | abandoned attempt 在首次会话读取之前已被恢复为终态。 |
| 崩溃后队列 | 排队消息按 FIFO 恢复，不依赖原发送客户端存活。 |
| 运行中 fork | BEFORE 后创建分支，源继续写入 AFTER，分支及二级 fork 的内容仍停在截点。 |
| 仅有继承历史的二级 fork | Continue 创建本地执行，不修改祖先 attempt，不增加伪造用户消息。 |
| 二级 fork 的组合操作 | compact、实际文件工具与权限回复、并发执行、取消隔离、重启和后续 summary 上下文。 |
| 非默认 workspace | root 和经过两次 fork 创建的分支实际读取所属 workspace 文件，未读到服务默认目录的同名文件。 |
| 排队 compact 重启 | 暂停队列中的维护用途保留，恢复后压缩上下文且用户消息身份不变。 |
| 真正旧数据库升级 | instance identity 与历史保留，升级后继续二级 fork、compact 和 Continue。 |
| 真实 subagent 工具链 | 模型实际调用 `agent.spawn`/`agent.wait`，子代理读取已授权 workspace 文件，父代理等待期间为 ToolPending；结果只投递一次；子代理 compact 不改变代理任务终态或重复投递。 |

subagent 工具场景还核对了 parent/root/depth、独立工作目录、父历史不被复制到子代理、真实文件字节进入模型请求、`agent.wait` 由消息唤醒而非超时，以及 compact 不增加用户消息。模型响应在可控 barrier 上暂停，保证观察到的 Running/ToolPending 不是依赖时间偶然性的瞬间状态。

## 结论的边界

这些回归验证会话内核、状态投影、工具授权/目录、控制执行、上下文快照和恢复协议。测试 provider 可确定性地控制流、失败和重启时机，适合验证这些不变量；它不能代替每一家外部模型服务或每个第三方插件的线上兼容性测试。已有角色权限边界和明确的 retention gap 也不会被消除。

审计执行环境中还观察到过 ChatGPT 协作子代理调用 smyl 连接器的部分远程工具挂起，而主代理相同调用正常。尚无可关联的服务端请求追踪来确定发生层级；该外部调用现象没有被计作已修复。上面的真实 HTTP 场景验证的是 Agena 自身的代理工具调用链。

第一轮验证结束时，修复与测试文件仍位于工作区，最新可执行程序已构建，尚未提交或推送 Git。审计没有替换正在运行的用户服务，也没有升级或修改真实用户会话数据库；启用这些修改需要启动新构建。以上升级和恢复验证全部使用临时测试数据库。
