# 当前数据与兼容性契约

Agena 以当前仓库状态作为唯一支持基线，只维护**一个当前数据模型、一个当前配置模型和一组当前公共工具身份**。

开发阶段修改内部 schema 时，不提供旧数据迁移、旧字段 alias、旧工具名重定向或旧本地状态升级。旧开发数据与当前 build 不匹配时，明确失败并重新创建。

## 仓库版本

仓库只维护一个 Agena 产品版本，当前固定为 `0.1.0`。根目录 `Cargo.toml` 的 `[workspace.package].version` 是版本来源；第一方 Rust crate 通过 `version.workspace = true` 继承它。Web 包的私有 `package.json` 使用相同值，安装包文件名、插件身份快照和工具参考文档从 Cargo 包版本生成。发布继续使用 beta channel，tag 写成 `agena-v0.1.0-beta.N`；`beta.N` 标识 beta 发布序号，不改变仓库或二进制版本。

协议版本、数据库并发 revision、插件自身发布版本以及第三方产品版本不是 Agena 仓库版本，不作为独立的 Agena 产品版本号。

## 数据库

### 主 SQLite 数据库

主库没有 `user_version`、schema generation 或 migration chain。

- 空数据库：一次创建当前 tables / indexes / seeds / invariant triggers。
- 非空数据库：逐对象比较当前声明。
- 缺少、增加或修改 table/index/trigger：启动失败。
- 不自动 ALTER TABLE。
- 不刷新旧 trigger。
- 不修改已有行来“升级”数据。

schema 改变后使用新的数据库路径。程序不转换、修补或自动删除用户原数据库；当前格式的导入导出只在新内核之间使用。

主库总是验证当前 schema，不提供跳过初始化/验证的启动开关。验证已有文件先于 WAL 模式切换和恢复写入，旧模型不匹配时保留原数据库文件。

文件模式由一个 writer 与最多四个 readers 访问同一数据库，所有连接使用 FULL 持久性策略；池的整个生命周期保有同一规范化路径的 owner 锁。内存模式复用同一 SQLite 引擎，并保留唯一私有连接；连接池不执行获取前探测、连接回调或自动轮换，防止取消读取时丢弃唯一连接而重建空库。内存模式不承诺跨进程恢复。

正文与历史读取受字节和并发预算约束。缓存容量回收只移除可重建条目，不撤销调用方已持有的只读视图，也不改变读取权限的 generation；提交围栏、显式失效和关闭仍会阻止旧 snapshot 回填。缓存记账不是进程 RSS 硬上限，调用方长期持有的视图仍计入各自预算。

### 同一个实例数据库

每个数据目录只有一个 `agena.db`。会话内核、workspace/permission 仓储、Scheduler job/history、`agena_server_state` 和 `agena_instance` 由同一个完整 schema 统一声明、初始化和验证。Scheduler 与 ServerState 接收已有连接或仓储 trait，不再另开独立数据库，也不在启动时各自修补表结构。未使用的 attachment-cache 表已删除。

小型应用状态仍通过有类型的 JSON wrapper 读写，落在 `agena_server_state`；凭据不放在这个表中。调度器与应用状态的写入复用同一实例的写入协调，不改变会话内核的事务、receipt、不可变正文块、blob pin、引用与恢复语义。

`agena_instance` 在新库创建事务中生成持久 UUID。API 的 `server.instance_id` 标识数据库实例，跨服务重启保持稳定；`server.id` 继续标识单次服务进程，用于端点记录和生命周期校验。TUI 草稿与输入历史在客户端按持久实例 ID 和远端工作区身份分目录，避免不同服务的数字 session ID 混用。

Scheduler 的 job/history JSON 同样只接受当前字段集合。当前 writer 总会写出的 policy/status 字段是必填项；未知字段直接拒绝。

## 统一数据目录

进程入口在启动异步任务前选择一次 `StorageLayout`。唯一存储位置参数是 `--data-dir DIR`，对应环境变量 `AGENA_DATA_DIR`，命令行优先，默认 `~/agena`。旧数据库、Scheduler、ServerState、插件、客户端和运行状态的独立位置变量及命令行参数已经移除，不保留 alias。

```text
<data-dir>/
  agena.db                       # 全实例 SQLite，含调度和 ServerState
  agena.json                     # 用户全局配置
  installation_id               # 安装身份
  secrets/                       # 应用自管的加密凭据与私有主密钥
  workspaces/<workspace-key>/
    memory/                      # 文件记忆的权威内容
    uploads/                     # 用户上传附件
    artifacts/                   # 工具、浏览器等生成物
    generated_images/<session>/  # 受会话访问边界约束的生成图片
  cache/memory/<workspace-key>/   # 可重建记忆索引
  marketplace/                   # 安装记录及插件缓存
  plugin-data/                   # 插件非敏感持久数据
  clients/<scope-key>/            # 本地客户端草稿、输入历史
  run/server.json                # 服务发现记录
  logs/                          # 服务日志
  tmp/                           # 实例临时 staging
  terminal-ui.json               # Terminal UI 状态
```

workspace key 使用规范化绝对路径的完整 BLAKE3 摘要；已存在的符号链接别名归一化，不使用有碰撞的字符替换或截断散列。远端工作区路径只作为资源身份使用，不在客户端机器上 canonicalize。

项目内用户维护的 `.agena/agena.json`、`.agena/skills` 等继续属于项目配置。应用产生的记忆、上传、工具记录及插件数据使用统一根目录。工具权限中的 `<state>` 仅指向当前工作区的 managed 目录，不授予整个实例数据根访问权限；workspace 附件接口只授权当前工作区的 uploads/artifacts，生成图片保留 session 边界。

## SecretStore

只有一个应用自管 `FileSecretStore`，位于当前实例 `secrets/`。Provider 管理凭据、MCP 客户端 OAuth、插件 secret 和 MCP 服务端签名私钥通过它读写。不使用系统钥匙串，也不提供回退存储或旧凭据搬运逻辑。显式用户配置的 inline/env 凭据仍是外部输入；通过认证流程管理的凭据不会复制回普通配置或配置 DTO。

命名空间在持有独立文件锁时整体读写，使用 ChaCha20-Poly1305 认证加密、随机 nonce 和命名空间 AAD。主密钥由应用随机生成，私有文件原子创建；凭据更新通过同目录临时文件、fsync、原子替换完成。已有密文而主密钥丢失、密文损坏、认证失败时明确报错，禁止重建密钥覆盖数据。文件名不拼接用户提供的 secret key。

Unix 私有目录为 `0700`，文件为 `0600`；Windows 使用私有 ACL。读写拒绝符号链接及非普通文件，在 Unix 上还拒绝多硬链接和非当前用户所有的凭据文件。错误和 Debug 输出不得包含凭据内容。异步调用通过 blocking worker 进入同步文件后端。

加密主密钥同样由应用存储，需要保留并保护。持有整份目录（包括主密钥）或拥有当前账户权限的程序仍能解密；此实现不声称抵御已控制当前账户的进程。备份密文与主密钥时应把它们作为敏感数据管理，丢失主密钥后重新认证。

## 本地持久化状态

以下状态只接受当前 shape：

- installation id；
- MCP server control / OAuth runtime；
- terminal session registry；
- terminal UI state；
- workspace preview registry；
- TUI composer draft；
- TUI prompt history；
- session persisted execution config；
- marketplace `installed.json`；
- cloud-media handle records；
- memory frontmatter。

client 不持有自己的 session 状态：session / execution / background-activity 的权威状态只在服务端，TUI、Web、CLI、IDE 都是纯展示客户端，不落盘自己的副本，也不写入共享数据库。每个数据目录同时只有一个服务端进程；内核在服务启动时将遗留的 running attempt、tool、provider round、background activity 和子任务标记为 interrupted；未认领的 queued 输入保留为已确认事实。服务端不会据此重放外部工具或进程，客户端不参与恢复。

共同规则：

- 未知字段拒绝；
- 当前必填字段缺失拒绝；
- 文件不存在可以初始化新状态；
- 文件存在但坏 JSON、旧 shape 或不完整 shape 不会自动改写成当前格式；
- 不通过 “version=0 → version=1” 或缺字段补默认来升级旧状态。

业务上真实可选的值仍可以使用 `Option`，当前 writer 主动省略的空集合也可以由当前 schema 明确声明为可选。这与旧版本迁移无关。

## 用户消息提交与显示

用户消息提交接口在输入校验、插件处理和附件准备完成后，原子保存用户消息 entry、execution、当前 generation 与附件引用，并在提交后发布变更，随后才返回成功确认。模型生成继续在后台执行，确认和用户消息显示均不等待模型回复。REST、RPC 与 TUI 使用同一个提交服务。公共 Part 仅是按需读取的展示投影，没有整块保存或覆盖会话的写入能力。

命令的 receipt 保存本次实际提交的身份和结果；其 `CommitPosition` 不能由之后的状态读取重建。Application 返回的 execution resource 是命令确认后的独立观测，可以包含更晚提交的事实。已确认命令的后续状态投影若失败，返回 `session.command_committed_projection_unavailable`，明确标记 `PartialSuccess`、`Never` retry 与 `Refresh` recovery：调用方应刷新会话，不能把它当作命令未提交而再次发送。

Web 与 TUI 的消息列表只显示服务端的真实 part，不创建本地用户消息占位、临时消息 ID，也不通过文本或消息数量猜测确认。提交响应、实时事件和后续刷新按相同的 part ID 与 revision 合并。提交前的失败返回错误以恢复草稿；确认后的模型失败属于已接受执行的结果。取消时，以服务端返回的撤回记录和原始文档恢复输入。

## Browser / Web 本地状态

Web 使用当前 Agena key：

- 不使用 `.v1` / `.v2` generation 后缀；
- 不读取旧 OpenCode query aliases；
- 不读取旧 OpenCode drag MIME；
- 不迁移旧 localStorage key；
- 当前 URL/query 只认当前 canonical key。

浏览器里遗留的旧 key 会自然被忽略；Agena 不扫描并搬运它们。

## 状态类型单一来源

session / execution / background-activity / notification 的状态类型只有一份 Rust 定义（`crates/agena-domain`、`crates/agena-api`、`crates/agena-notification`）：

- storage、API、TUI、CLI 与 RPC 直接使用或重导出同一类型，不做逐层字符串二次映射；
- 不保留层内的平行 enum、字符串常量表或别名映射；
- Web 端消费生成的 TypeScript 镜像 `packages/agena-web/src/generated/agenaState.ts`（`cargo run -p agena-web-types > …` 生成，`cargo test -p agena-web-types` 校验），不在前端维护第二份 union；
- 未知状态值在 API 边界一次性降级为文档化的 fallback（`ready`），不新增“兼容旧状态”的分支。

`awaiting_interaction` 这类 wire 字符串与 active / attention / recovery / terminal 分类都从该单一来源派生；前端帮助函数与 Rust 谓词由同一份定义校验一致。

## 会话历史操作

会话由 `SessionCommands` 修改配置与历史，由 executor 签发的 `AttemptHandle`、`RoundHandle`、`ToolHandle` 和 typed content producer 记录执行事实。Execution、attempt、round、tool、interaction、content 的权威行各自唯一；普通调用者不能通过通用 JSON 或可变 Session 写回状态。`PartResource` 是只读展示投影，没有与内核并行维护的 run marker。

配置使用独立的 metadata version 进行乐观并发校验；每个会话另有持久化 control version，在语义命令的同一事务内递增，幂等重放不递增。`KernelReader::control_snapshot` 在一个短 SQLite 读事务固定 metadata、summary、configuration、control version、有界 pending/active 页和从索引控制事实派生的执行阶段，显式完成回滚后返回受预算管理的不可变读值。Application 在该观测上叠加当前 runtime defaults，不重读其中已固定的会话事实；根 pending 复用同一观测，descendant task、历史、正文、用量、provider retry 与 scheduler 各自保留读取边界。整个响应不承诺全库同一时刻的原子快照。列表通知携带事务内捕获的 before/after 和最多两个受影响父行，只推进实际相关的列表、分组计数与工作区统计版本。

正文 cursor 的单位随 kind 明确区分：Text 使用 UTF-8 byte offset，Log、Mixed、Document 和 Terminal 使用 event sequence。对外 `total_bytes` 仅在 Text 已知时给出数字，事件源返回空值，不把事件数当作字节数。

事件分页只共享已校验的不可变物理块与仍有效的授权 context；缓存和块定位索引共同受 16 MiB / 2,048 entries 的上限及写入 COMMIT fence 管理，不保存整个 source 的目录。open head 每次实时捕获，小页只截取所需尾部；缓存不会把旧 accepted/committed head 当作当前事实。跨提交、撤权或裁剪的竞态会重新验证或明确重试，过期 epoch 和已删除前缀继续返回 reset/gap。frozen provider 输入仍按原 committed boundary 读取，后续追加不会进入该输入。

- 用户发送将真实 user EntryId、输入文档、附件 blob 引用和 queued attempt 一起提交后确认。
- AI 工作中继续发送也使用同一提交接口；后端按已提交的用户 EntryId FIFO 调度，等待当前整次 execution（含工具、交互等待和后续模型轮次）结束。后续待发消息不会提前进入当前 execution 的 prompt，模型历史按消息归属恢复为 user / assistant / user 顺序。Web 与 TUI 只提交和展示服务端 `queued_messages` / `queue_paused`，不保留自行触发发送的客户端队列。停止先持久化暂停标记、再取消 running，保留 queued；writer Claim 和后台恢复都服从该标记，只有显式恢复才继续 FIFO。两端停止后提供继续或保留队列的选择，并提供独立弹窗原位编辑（含附件）、单条删除、清空及暂停/恢复。
- `/runs` 按 part 的 `message_id` 归属提供正式对话分页，不把尚未认领或未认领即撤回的 user entry 计入消息窗口及导航总数。队列正文根据服务端 `queued_messages` 的明确身份读取，在 Web 输入框上方 / TUI footer 单独显示。`content.queue_only` 区分待发或撤回的原始输入与已使用过的用户回合；queued retry 不会隐藏先前已执行的输入。part revision 也覆盖 attempt generation 与执行阶段推进，确保认领后客户端能清除 pending 缓存。
- 未认领的 queued 输入在正常关闭及服务重启后保留并由后端恢复；新提交入口也先恢复同会话已持久化的队列，避免抢在后台扫描之前越序。`GET /api/v1/sessions/{session_id}/queue/{execution_id}` 返回原文档及 revision；同路径 `PUT` 以 `request_id` 和 `expected_revision` 条件更新从未开始、未冻结且未共享的 queued 输入，保留消息身份与 FIFO，原子替换正文和附件引用；`DELETE` 幂等地移除仍为 queued 的消息。已被认领、已移除或版本冲突时返回 HTTP 409，不能修改正在运行或已经使用过的输入。详细语义与调研见 [消息队列](message-queue.md)。
- 助手消息使用该 generation 第一条真实 assistant entry 作为稳定边界；后续正文、推理和工具属于同一 response。重试创建新 generation，历史响应身份不变。
- 压缩使用独立 provider round 与不可变摘要源；只有成功的 `commit_compaction` 才同时提交 checkpoint、覆盖边界与 prompt-window generation。失败、取消及 unchanged 保留原窗口。
- 普通 provider 请求在 round 开始时冻结 entry revision、tool/interaction 决定、capture head 与 opaque replay。后续正文或后台通知不会改写已经构造的请求。
- 文本为按 UTF-8 字节定位的不可变块；log/structured/terminal/mixed 使用独立事件序列。PTY 原始日志与屏幕分属两个源，screen patch 的 base 只使用 screen event cursor。
- plugin mixed 输出在单一事件流中保留 Text、Log 与 Structured 原顺序；document checkpoint 不替换或吞掉触发 checkpoint 的原始事件。

- 原生压缩的独立 `/responses/compact` 请求使用自己的协议结构，不复用普通生成请求的工具定义、推理、输出格式或流式控制字段。共享请求覆盖仅传递该接口支持的缓存和服务等级选项，模型、系统指令、完整输入与 continuation 边界由 runtime 持有。
- 只有包含有效加密 compaction 项的原生输出才可成为 checkpoint；返回的整个窗口原样保留，后续工具结果可以引用窗口内保留下来的调用。普通成功响应、空输出或普通 assistant 回答不会被当成压缩成功。
- 自动压缩记忆明确的原生路由不可用状态（404 / 405 / 501）；参数错误、输入溢出、认证或临时服务错误不构成能力判定。手动压缩可重新探测，成功后持久化清除旧记录；能力快照与失败抑制均受压缩策略版本约束。
- 本地摘要使用独立的无工具文本请求，保留路由和速度选项，移除普通回答的内容覆盖、格式、停止条件和 continuation 参数。完整历史能装入预算时不按固定单条长度截断；重复压缩必须把上一轮 checkpoint 的有效约束和状态交给摘要模型。历史 system / tool 文本作为带来源标记的 assistant 历史数据，不能变成新的系统指令或用户轮次。
- 近期窗口保留实际用户任务；输入溢出与空摘要／输出耗尽分别允许一次有界恢复。候选未减小且原窗口仍可用时完成为 `unchanged`，超限、失败、取消时保留原历史和 checkpoint。

历史操作使用真实用户 EntryId 或助手 response 的第一条 EntryId；HTTP、command、Web 与 TUI 都通过 `at_message_id` 指定目标。后续 generation 不改变先前消息边界。

- `fork` 创建子会话，保留截至目标消息末尾的历史（包含目标消息）；省略目标时保留源会话的完整历史。源会话必须已有可用的历史边界。
- `rewind` 只接受源会话中的已完成用户消息，创建只保留该消息之前历史的子会话，并把目标输入恢复到新分支的输入框。回退首条用户消息时，新分支历史为空。
- 两种操作都返回包含新会话的 execution resource；客户端打开返回的新会话。原会话及其输入草稿继续保留，分支后续写入不会改写原会话。
- 继承的历史可以包含仍在流式更新的父会话 part；它们只作为历史展示，不计入新分支的运行、待执行工具或待答交互。新分支的执行状态只由它 origin-owned 的 attempt、tool 与 interaction 决定。
- 显式复制或导出对话记录读取完整的用户可见分页历史；聊天界面的当前加载窗口不构成导出边界。

`fork` / `rewind` 不恢复工作区文件。已完成的文件修改通过项目 Git 历史恢复；`fs.apply_patch` 只输出操作标识、文件变更、diff 和进度，不提供反向补丁或撤销哈希。Git 提交、危险操作确认和发布的行为约定见 [Git 工作流程与文件恢复](git-workflow.md)。

## 当前格式归档与新数据库启动

内部 schema 改变后，启动时显式选择一个新的空数据目录，例如：

```sh
agena --data-dir /absolute/new-data server \
  --host 127.0.0.1 --port 3211 --workspace /absolute/workspace
```

该参数统一选择配置、数据库、凭据、workspace 管理文件和本地客户端状态的位置。原数据目录保持原样；不要删除原文件来绕过格式检查。旧数据库和旧 archive 不提供转换入口。当前格式 archive 用于当前内核之间复制会话，不是完整数据库备份。

CLI 通过服务端流式传输 JSONL；导出核对完整 footer、记录数量、字节数、SHA-256 和 EOF，导入成功只返回已提交的新会话身份：

```sh
agena --server http://127.0.0.1:3211 sessions export 42 > session.jsonl
agena --server http://127.0.0.1:3211 sessions import --path session.jsonl \
  --idempotency-key 7c7fef97-e321-4b1f-b608-d92297864670
```

丢失导入响应后，重试同一份文件时复用相同 UUID；相同键和相同输入返回相同结果，相同键与不同输入冲突。省略 `--path` 时从 stdin 读取；SDK 不会隐式重放已经消费的输入流。

HTTP 导出是 `GET /api/v1/sessions/{id}/export`，正文为原始 `application/x-ndjson`；导入是 `POST /api/v1/sessions/import`，使用相同 Content-Type 并要求 UUID `Idempotency-Key` header。HTTP 不包裹完整 archive 的 JSON 字符串，也没有整包 16 MiB 上限；单条编码记录上限为 32 MiB。导入先在私有临时文件与 staging 数据库中完整校验，再一次事务发布。坏 footer、截断上传或校验失败不发布部分会话。

归档固定 metadata 和历史 membership 的快照边界，entry/control/source head 按声明的记录观察范围读取；它不承诺全库同一时刻的原子备份。缺少成功 footer 的流不能作为有效归档。Web/TUI 的可读 Markdown 导出与该机器归档格式是两个用途明确的入口。

## 配置

Runtime config 只有当前 schema：

- 当前结构使用 `deny_unknown_fields`；
- 不维护旧字段名字词典；
- 不为旧字段提供专门迁移错误；
- 不读取旧 mode/default/provider-selection 环境开关；
- 未知字段就是普通配置错误。

Provider/adapter 自身的当前外部协议参数不属于 Agena 版本兼容；例如外部服务返回字段差异可以由 provider adapter 正常处理。

## 工具与插件

工具目录只包含当前 tool identity。

- 没有 old-name → new-name 映射；
- 没有 retired-tool migration catalogue；
- 未注册名称就是普通未知工具；
- UI renderer 不保留已删除工具的特殊标题/结果视图。

Plugin marketplace 只使用当前 plugin id：

- 没有 rename graph；
- 没有 old plugin id alias；
- manifest/index 中声明的 schema/version 必须显式存在并匹配当前支持值。

Plugin SDK 的通用参数 alias DSL 是当前插件开发能力，不是 Agena 旧版本迁移；Agena 自己的生产插件不依赖旧字段 alias。

每个 part 的正文只有一份持久化事实源，不把正文或派生摘要复制到通用 `summary` 列。工具的 `output.payload` 保存工具定义的完整原始结果，纯文本结果也放在这个 payload 中；退出码、耗时、分页与文件内容等结果属性属于同一个结果对象，不另设 `output.text` 预览副本或 `output.metadata`。AI 序列化和人类展示在读取时分别交给插件/工具处理，模型文本、人类摘要与视图块不落盘。客户端的 Output 详情展示完整原始输出，Presentation 展示工具提供的人类视图。调用层 `metadata` 仅保存调用身份、provider 协议上下文与运行控制信息。

工具图片、音频与视频通过 `output.attachments` 保存资源事实，统一投影为 `ViewBlock::Media { artifact: { uri, mime, name, ... } }`。Web 使用同一附件预览显示内联图片、远端媒体与本地文件；本地 `file://` 资源通过所属会话工作区的文件 API 读取，图片可打开查看器。OpenAI 原生图片生成在协议边界将 `result` 图片编码归入附件，结果元信息与 provider 调试记录不重复保存该编码。当前数据中已经同时包含图片附件与 `/result` 编码文本资源的记录，在读取展示时只排除对应编码资源，保留其他文字资源和显式 Output 原始详情。provider 调试资源只作为 metadata 引用，不进入普通正文的资源列表或客户端兜底展示；附件元信息也不展示图片编码。

provider 回放状态只保存在 execution 对应的 round 中，不同时复制到 execution 的独立 provider-state 列。每轮保存固定的输入身份、revision、source head 与回放引用；读取时按这些边界恢复正文与原始 opaque 协议值，保留签名、顺序和不同的 opaque 事实。成功的 native compaction 同时保存可移植摘要和 opaque 状态；无法生成有效摘要时不安装 checkpoint。Web 与 TUI 的默认策略仅展开 answer 和 text，think、工具、附件及待答交互默认折叠；用户显式配置和手动展开优先。

权限决定将原始回复的类型、原因、scope、request identity 和实际分类结果一起提交；记住规则与决定共用事务。重复相同决定保持身份和决定时间，冲突回复不能覆盖已提交记录。后台活动有独立于父 attempt 的生命周期；子任务指导绑定原 execution，通过可靠投递进入下一个模型边界，成功结束不能越过尚未消费的指导。

会话列表读取维护于同一事务的紧凑统计与索引，不读取完整历史或配置 JSON。子会话数量和处理状态随事实更新；创建、重挂父节点或删除子会话时，同时使相关父会话的摘要缓存失效。

## Commands 与项目指导

Command discovery 只使用 Agena 自己的当前 roots，例如：

- Agena home 的 `commands`（纯 `.md` command 文档）与 `skills`（其他 agent 的 `SKILL.md` package，经 `agena.commands` 桥接）；
- workspace `.agena/commands`；
- workspace `.agena/skills`（同样经 `agena.commands` 桥接）。

Agena 自己的词汇是 command；`skill` 只作为其他 agent 生态的词汇出现在桥接层和磁盘目录名上，两者都注册进同一个 command catalog。

不扫描跨-agent `.agents/*` 兼容目录。

项目指导文件使用当前支持的名称；不保留已删除文件名的专门兼容逻辑。

## 保留的“版本”概念

“不要兼容旧版本”不意味着删除所有名为 version 的字段。以下是当前功能的一部分，可以存在：

- API / WebSocket / plugin manifest 的当前协议版本；
- marketplace manifest/index 当前 schema/version 声明；
- ABI/API version；
- optimistic-concurrency revision；
- terminal UI state revision；
- plugin/package semantic version；
- Git/HTTP/第三方 Provider 的协议版本；
- 模型名本身包含的 v3/v4 等版本。

这些字段用于**当前协议判断或业务并发控制**，不是旧数据 migration chain。

## 保留的外部兼容

Agena 仍需要与真实外部生态交互，因此保留：

- OpenAI-compatible / Anthropic / Gemini 等第三方 wire 差异；
- 外部模型 catalog 的字段差异与模型命名；
- Git 不同版本的命令行为；
- 真实终端对 keyboard/paste 协议支持不一致时的输入兼容；
- 浏览器/操作系统的当前平台差异。

这些是外部协议互操作，不是 Agena 自己旧版本/旧数据兼容。

## 开发原则

修改 Agena 自己的内部 schema 时：

1. 直接修改当前 schema。
2. 更新当前 writer/reader/tests。
3. 删除旧 shape 的解析、repair、migration 和 alias 代码。
4. 开发者删除不兼容的本地状态并重新创建。
5. 测试验证“当前 shape 成功、不完整/额外字段失败”，而不是保存旧 shape 样本。
6. 文档只描述当前契约，不维护未发布版本的迁移史。

## 实时同步与按需核对

会话、part 和文件的权威状态属于服务器。实时通知是可丢失的失效提示或完整 part 快照，不是可持久回放的事件日志；客户端不能仅凭连接正常或某个事件游标就认定缓存已经完整。

- 服务端先建立订阅，再允许客户端读取快照。Web 在每次 SSE 建立连接后重新核对所有仍挂载的聊天面板；TUI 重连后分别读取执行状态和最近一页 transcript，按各自版本、cursor 与请求身份合并。断线、队列溢出、恢复前台和显式删除都有恢复路径。
- `SessionTranscriptPart`、实时 part 和工具详情携带 `revision` 与 `updated_at_ms`。客户端按这两个字段依次比较，拒绝旧响应覆盖新内容；完整快照中省略的字段不能继续沿用旧值。数据库的 `metadata_version` 用于配置与元数据的乐观并发控制；`SessionResource.version` 投影该 metadata version。`control_version` 在同一事务内随该会话的语义命令递增，相同 request 的重放不重复递增；`SessionExecutionResource.latest_event_seq` 标识原生 control snapshot 实际读取的 control version。两者均不等于命令的 `CommitPosition`，也不替代 history membership generation 或 content byte/event cursor。原生控制字段共享同一读事务，响应中的其他资源仍按各自声明的边界读取；不再通过等待整个响应组装期间版本静止来确认一个已经提交的命令。
- 流式正文区分 accepted 与 committed head：typed producer 被接收后及时发布 live head，由共享 checkpoint 在约 64 KiB／500 ms 阈值下提交；完成、取消、交互决定等业务屏障提交所需尾部。正文 chunk 不推进 entry revision。每个 source 使用自己的 epoch/cursor/watch；fork 中同一个 ContentId 继续共享 live 内容，重启后的旧 epoch 必须 reset。外部输入被丢弃时记录独立 capture-loss 窗口与 observed/committed byte/packet 计数，不伪造 accepted cursor；达到终态但有 capture loss 的源保持 interrupted，工具退出成功与捕获中断可以同时成立。
- `GET /api/v1/sessions/{id}/parts?ids=...` 每次最多接收 256 个正整数 ID，与分页参数互斥，只返回该会话中对用户可见的对应成员。客户端重连时按批核对**已经加载**的 ID，移除缺失成员；不会为了恢复而遍历尚未展开的历史。SQLite 按成员关系查询这些 ID，工具详情也通过单个 ID 读取。
- 交互控件的可操作性同时取决于工具 part 的持久化记录和完整执行快照中的待答列表。已回复、已终止或从待答列表移除的请求不再显示可提交控件；执行状态发现旧的已加载审批仍未收敛时，按 ID 重新读取其所属 part，即使该 part 已不在最近一页中。历史内容不因状态变化而伪造回复，未提交草稿也不作为已提交答案展示。
- Web 的权限批准、提交答案和取消回答共用提交前校验：强制向服务端校验执行快照，确认请求仍在待答列表，并使用请求上下文的 `session_id` 路由到真正的所属会话。继承的 part 只读；runtime 回复查找和展示确认也校验 `origin_session_id`。已知已回复或已终止请求的重试保持幂等；不再能匹配当前会话的请求返回 `interaction.request_not_pending` / HTTP 409，客户端重新同步状态。更高版本的状态不会因并发的展示确认而被丢弃。
- 会话删除有独立的 `session_deleted` 通知，包括空会话。通知保留 workspace ID 与临时会话可见性；origin 被 tombstone 后仍能按工作区投递，分支继续保有自己引用的不可变历史。不存在的会话读取返回 404；客户端保留删除标记，拒绝迟到请求重新创建已删除内容。
- Web 核心 API 的资源版本是 `server UUID:monotonic_updated_at_ms`。逻辑更新时间取墙钟时间与上次值加一的较大值，同毫秒修改、墙钟回拨和服务端重启都不能复用旧版本。版本按会话列表、工作区列表、工作区会话列表、会话执行状态、会话正文、文件修改、计划、part、工具 input/metadata/output 分区和 activity 描述/日志独立维护。SSE 通知携带相关 `resource_revisions`，只唤醒版本实际变化的订阅者。已显示的文本、推理和完整 activity 描述直接合并流快照，日志变化不触发全量列表或执行状态重读。核心 API 与预览 registry 的 UUID 属于独立来源，分别检测重启与拒绝旧响应；预览版本不能使核心 API 版本失效。
- 侧栏按 `workspaces:catalog`、`workspace:{id}:sessions:roots`、`workspace:{id}:sessions:parent:{parent}`、`workspace:{id}:sessions:bucket:{kind}`、`workspace:{id}:stats` 和全局 `sessions:bucket:{kind}` 隔离。目录 A 内某个子会话的标题变化只核对其直接父级的已展开列表；根列表、其他父级分支、无关置顶列表和 B、C 目录保持不变。子会话增删或更换父级会改变父行的 child count，必须连带核对父行所在列表。分页或排序发生真实变化时重读该查询的当前页，未改变的实际 Vue 行对象继续复用。关闭分支释放对应订阅，每个当前查询保留一页及其实际响应版本，独立于共享正文缓存的淘汰。
- 统计使用 `GET /api/v1/workspaces/session-stats?ids=...` 合批，只返回受影响目录的计数和读取前捕获的版本，单批最多 128 个目录。项目目录 GET 不附带会话统计。新增或删除项目才核对目录集合，移动会话更新旧、新目录；未展开目录只更新统计，不枚举历史。全局置顶、收藏及 recent/running/attention 汇总的行版本与 `sessions:bucket:{kind}:count` 分离。折叠 footer 使用 `count_only=true`，只计算 total，不加载会话行或投影状态；已知 membership 不变时的标题或行内容变化不唤醒计数读取。遇到未知执行状态、关联 memo 淘汰或跨进程修改时保守核对相关汇总及受影响目录，避免错误地认为成员数未变。
- 已挂载会话的恢复先合批核对 `session:{id}:transcript` 和 `session:{id}:state`。正文与状态各自保留实际 HTTP 响应版本，未改变时不下载正文、不核对 loaded part membership，共享正文缓存被淘汰也保持这一行为。正文确实变化后只恢复最近一页和已经加载的成员；其他可见会话保留原值。发送或 continue 的确认仍强制条件读取，不能因为完成通知丢失而复用发送前的正文。计划文档和已初始化 activity 列表也保留自己的实际响应版本，恢复前台时未变的内容直接复用；手动刷新仍执行条件校验。
- 侧栏创建、删除、收藏和置顶操作，以及单目录手动刷新、展开和分页，同样进入相应范围的读取。全局聊天 bootstrap 列表的实时 metadata 和删除直接更新缓存，可见会话执行状态单独订阅；只有版本兜底发现通知缺口，或重连恰好撞上未完成的首次列表读取，才恢复该列表。
- 工具详情只订阅已经展开的 `part:{id}:input|metadata|output` 分区，输出变化不会调度其他分区。组件保留当前分区正文对应的实际 HTTP 响应版本，即使共享正文缓存被淘汰，未变化的已显示分区仍直接使用现有值。终态只核对 output 和被取消的未完成读取；正文先进入终态、详情版本通知尚未到达时，直接条件校验尚未追上终态的 output，避免等到慢速兜底。隐藏、折叠或卸载解除对应订阅并取消请求。
- 同一页面实例内所有挂载订阅者共用一个 30 秒版本兜底队列，通过 `GET /api/v1/changes/revisions?resources=...` 每批核对至多 128 个资源，未变化时不读取正文。通用可见资源读取器在首次打开没有缓存的资源时直接读取正文，以响应 ETag 初始化版本，避免额外的先查版本请求。恢复前台、重连或通知缺口先合并失效提示，再核对版本；隐藏时停止周期检查。同一个数据目录由单个服务端持有，其他进程不能直接改写其会话库。服务端 change feed 以进程 epoch 与 commit position 恢复；明确列出受影响 EntryId，无法完整列出或共享 membership 变化时触发 snapshot/reset。慢消费者与重启通过有界权威页面收敛。
- part、会话 metadata、workspace 和 plan 的写入先比较实际内容。可证明的 no-op 不写数据库、不增加 revision 或更新时间、不发布失效通知；真实修改的更新时间单调增加。运行时 activity 与 plan 的资源时钟由同步 mutation observer 在发布通知前推进，不依赖异步消息消费者的速度。activity 描述使用 `activity:{id}`，日志使用 `activity:{id}:logs`；progress/message 的变化不读取日志，`LogsChanged` 可在同一游标内更新委派任务的流式文本且不重读描述、列表或会话状态。退出状态、日志游标、分页及 dropped-lines 的真实变化仍更新日志时钟。
- 条件正文接口使用弱 ETag、`If-None-Match` 和空正文 304；服务端在历史读取、投影和序列化前比较版本，并在读取前捕获响应版本，防止旧正文携带新版本，transcript 同样在重型历史扫描前返回 304。Web 缓存按 backend、认证作用域、资源与完整 URL 隔离，最多保留 80 个表示、估计 8 MiB，单个表示最多缓存 2 MiB。局部保留的正文使用它自身响应的 ETag，不能读取完成时从版本索引取一个较新 token 来冒充已同步；缺少 ETag 时不建立可复用版本。迟到的响应不能回退版本或覆盖重启后的 epoch。同 URL 读取共享一次请求，消费者独立取消，最后一个消费者取消时中止传输；显示正文读取共用最多四个并发名额。
- Rust HTTP 客户端保留最多 64 个正文、合计 4 MiB，单个正文上限 2 MiB，显示读取同样限制为四个并发。相同 URL 的排队读取合并，派发前到达的强制核对共享该次结果，派发之后到达的强制核对保留后续条件读取。强制核对仍发送 ETag；没有全局实时订阅的普通显式读取也要向服务器验证 ETag，不能假定缓存还有效。
- Web 的刷新队列限制同一数据源只有一个请求在运行，并保留运行期间收到的一次后续刷新。冷却时间从请求完成时开始，高频事件不能不断推迟首次刷新；失败读取采用最多 60 秒的指数退避，事件、手动刷新和恢复前台不能穿透退避。关闭面板或切换身份时取消适用的读取，只自动核对仍显示的会话及展开的详情。TUI 同样合并失效提示，按会话身份、请求标识和订阅代次拒绝过期结果。
- 文件面板通过 `/api/v1/workbench/fs/stream` 按需监听根目录、展开目录和选中文件的父目录，最多 128 个附加目录、每批 256 个变化路径；监听非递归，流断开即释放。服务端按 200 ms 合批，Web 按 1 秒合并读取，只重读受影响的已显示目录和已加载分页范围。同目录内切换选中文件且监听路径集合不变时复用连接；局部读取失败只重试原变化路径，不升级为全范围刷新。展开目录超过监听预算时，额外保留 60 秒一次的已加载目录核对；目录删除或替换后重建监听。面板隐藏时关闭监听；恢复、断线、unknown/rescan 和真正监听范围重建需要保守核对已加载范围。保存响应仅确认实际提交的文本，期间的新编辑仍保留为未保存草稿；跨文件或工作区的旧响应不能覆盖当前文件。TUI 连接时只读取当前目录；文件补全索引在首次使用补全时才加载，后续限制为每 10 秒至多重新读取一次。macOS 下从用户主目录或文件系统根目录发起递归发现时，先排除未主动选择的隐私目录和挂载卷，再打开目录；显式选择其中的项目作为工作区仍可正常读取。
- Git 每个仓库共享一个文件系统监听器和串行状态快照缓存，多个浏览器不重复计算同一快照，最后一个监听消费者关闭时释放。事件合并后再计算，空闲只保留共享的 60 秒安全核对。工作树、选中文件和列表分区签名限制 diff、已展开列表和冲突读取。小型 dirty 文件使用有界内容哈希，每文件最多 512 KiB、每轮至多 2 MiB；超预算文件保守使用元数据签名，纯 touch 可能唤醒读取，该预算不限制 libgit2 自身的状态扫描成本。
- Git 页面保留仓库级 watch 基线，切换选中文件只更换路径订阅。首次实时快照只核对已读取或正在读取的列表，之后的初次读取自行覆盖该基线；冲突列表拒绝被新快照淘汰的旧响应。重命名使用新路径，工作区签名同时覆盖 index 基线变化。列表按分区合并分页与重读，排队期间的失效会在实际派发时把 load-more 转为第一页读取，派发后的失效保留一次后续重读。空分区直接清理，收起的列表和 stash 不自动读取。
- 终端 UI 状态使用现有 SSE 快照和 patch，预览 registry 的共享 SSE 首次/重连发送恢复快照，后续仅发送修改记录及删除 ID。服务端每个 registry revision 只生成一次快照与序列化增量，所有浏览器共享；新连接或错过增量基线的慢消费者使用同样共享的恢复快照。跨 mutation 的迟到数据库读取不能回填已被清空的快照缓存，失败读取不能被永久记为该版本的空 registry。前端直接合并对应条目，保持其他目录条目的对象；创建、修改、启停和重命名直接应用操作响应，不再为一次修改重读整个 registry。条件列表 GET 用于显式核对和流载荷不完整时的恢复。同一页面实例内的预览消费者共用一条流。预览目标健康检查按需发送有超时和退避的 HEAD，托管进程退出会推送 registry 变化，外部目标不做持续周期探测。OAuth device flow 仍按 provider 指定间隔核对，activity 日志仅在 `has_more` 时继续短间隔增量分页，用户启用的 Git 分钟级自动 fetch/sync 保留。
- 插件市场操作和运行时重载等待已有 runtime task 对应的 activity，不为每次等待建立新 SSE 连接或读取整个后台任务列表、runtime 状态。订阅后至多读取一次目标 activity 以关闭启动竞态，后续优先直接接收全局流的终态描述，丢失通知通过同一个资源版本队列恢复。超时前若页面可见，额外做一次最多一秒的条件校验，避免短操作的完成通知丢失造成误报。目标 activity GET 支持条件读取；等待有总超时、隐藏暂停和消费者取消，已经在运行的去重任务同样等待其终态。

这些保证针对单个服务端进程与其客户端之间的收敛、顺序和有界工作量。网络断开期间无法提供新状态；恢复后通过权威读取收敛。消息发送失败后的未知结果不能通过无条件重发解决，客户端不自动重放没有幂等身份的发送命令。

## 工具进度、后台等待与会话编辑

前台命令执行时，stdout/stderr 的排空和 UI 消费分离。执行器保留最新的 8 KiB UTF-8 安全输出尾部，转发层至多每 100 ms 发送一次合并快照；显示尾部相同不会再次发布。慢客户端可以跳过中间快照，不能阻塞子进程的管道，也不能积累无界输出队列。退出结果仍以每条输出流最多 8 MiB 的捕获为准；达到结果捕获上限后，实时尾部继续更新。任意读取边界拆开的 UTF-8 字符会先保留未完成字节，再解码。取消时先释放持有发送端的执行 future，再等待最后输出的转发收尾，避免等待一个无法关闭的 watch channel。

运行中的工具 Presentation 展示命令代码块和当前输出；按需读取的 Output 详情也可以投影这个实时尾部。投影不写成持久化完成结果。Output 详情携带自身 `part_state`，使只因 sibling section 修改而过期的整 part 时间戳不会让有效的缓存 Output 被反复拒绝。Web 与 TUI 保留已经展示的详情，在运行状态内接受有用的中间快照并继续追赶；进入终态后拒绝旧运行请求覆盖最终结果。Web 各分节独立记录待更新与失败退避，成功读取其他分节不能重置失败分节的退避；关闭 disclosure、切换会话或隐藏页面会取消相应读取。

后台任务的启动 part 是已完成的启动回执，后台生命周期由 kernel 的独立 background activity 事实表示，完成通知使用有稳定业务 key 的 durable delivery。输入框上方的任务面板从 activity 状态和日志接口读取输出，不以启动回执的 completed 状态判断进程结束。普通管道和 PTY 日志用 `chunk=true` 保留原始换行；带正则条件的 monitor 使用行记录。委派任务的日志游标标识 run，同一个 run 的回复继续生成时，客户端替换该游标的文本快照，不重复追加，也不跳过其后续内容。

展开的启动 part 按 `source_part_id` 关联 activity，展示独立的实时状态与日志，保留 durable 回执的原始含义。Web 任务面板和 part 共用日志资源与条件读取；TUI 只为主界面视口内展开的 part 读取日志，最多同时跟随 16 个 activity，保留至多 64 个展示尾部，并优先淘汰视口外缓存。关闭、离开视口、切换会话和退出 TUI 都释放适用的读取。委派任务中的前台命令同样把运行输出投影进其当前 run 的日志快照。

正常 `agent.stop` 只在当前执行没有待处理输入、没有未被 provider 完整观察的后台通知、没有仍可唤醒会话的后台活动或定时任务时派发。后台等待会释放当前 execution，让 durable 通知可以唤醒会话；它不会发出正常完成 hook，也不通过额外模型调用轮询。`prepare_stop` 与通知提交通过同一个 kernel writer 排序；成功 round 只确认其冻结输入中的通知。Ready 决策会拒绝迟到的定向 guidance，stop hook 请求续跑后在新 round 清除该限制；普通业务事件仍由后续唤醒处理。Scheduler 使用独立数据库，stop 决策前读取其 session-scoped 快照，不承诺跨库事务。已暂停、删除或完成的定时任务不阻止停止；周期任务只要还有下一次触发，仍属于等待。已完成 provider round 的精确一次性定时通知可以证明本次触发已被处理，不依赖必须等 execution 释放的 scheduler ack。失败仍携带 `run_error` 到达 stop hook；终端 Done 提醒只在所有 stop 决策 hook 同意停止后发出。

`GET /api/v1/sessions/{id}/file-changes` 使用会话 membership 内归属本会话的工具记录投影编辑。服务端使用公开 `owned_tool_entries` 能力，按 origin、工具 kind 与 EntryId 索引分页，并验证当前 membership；普通文本和 provider opaque replay 不进入查询与反序列化。每页最多 128 项，先读取 generated body/input/result 字节数，按 1 MiB 预算选定 identities，再读取工具事实。cursor 绑定 session、membership 和首次读取的工具 ID 上界，允许会话继续执行；标题、round 和文本更新不使文件历史过期。真实锚点移除在服务端自动重启有界读取，保留的编辑事实也受字节预算限制。完成结果从 kernel 的工具事实读取，不维护另一份流式 overlay。只接受成功完成的 `fs.write`、`fs.replace`、实际应用的 `code.rewrite_ast` 和 `fs.apply_patch` 结果。fork 继承的编辑、预览、失败和可证明的 no-op 不计入新会话。连续且完整的 before/after SHA 链回到起始 revision 时可以消除净零变更；缺少基线或存在断开的链时展示记录的操作历史。该 API 支持摘要、文件分页和单文件详情，diff 返回预算上限为 2 MiB。一个文件只占据一行：该行合并此路径所有已记录操作的新增/删除行数，`fs.apply_patch` 只按本文件段落计数，整体缺基线时不补造净差异。

文件历史的资源通知版本与响应内容的 ETag 独立：`x-resource-revision` 表示读取开始前的资源版本，ETag 校验当前摘要、页或单文件响应是否实际变化。内容相同时返回无正文的 304，Web 与 Rust 客户端保留原对象并更新其资源观察版本，不能把缓存永久绑定到旧版本。同一会话的并发摘要/页/diff 请求共享一次历史构建。选中文件的 diff 按响应实际返回的文件 revision 保留，列表较旧时自动同步；读取冲突和终态详情的过渡快照由可见组件退避重读，保留有用内容，不要求用户手动 refresh/retry。后台或已卸载组件停止读取。

每个文件摘要携带由该路径完整编辑事实计算的 fingerprint，不依赖其他文件或当前 diff 截断预算。已选 diff 只在其自身 fingerprint 变化时自动读取，其他文件的变化只更新文件列表；同一 fingerprint 在隐藏恢复和重新展开时复用当前保留的 diff。服务端为了计算净 SHA 链仍读取本会话工具记录，并未实现持久化的逐文件增量索引。

文件资源时钟使用和 API 投影一致的编辑事实，而不是整个 part 的更新时间。无关工具、metadata、重复的写入声明和同一编辑结果的重复更新不会推进 files 版本。首次读取只在有界 memo 中登记缺失的基线，结果解析在资源时钟锁外完成；期间的修改或删除拥有更新的基线，迟到的登记不能覆盖它。

会话编辑面板不读取 Git 工作树。Shell 的 `writes` 声明只能说明执行者预期写入的范围，不能证明某个文件确实发生编辑；未记录实际编辑结果时返回 `recording_incomplete`。客户端只在确实记录了文件编辑时才显示会话编辑入口，`recording_incomplete` 不单独构成显示条件；入口存在时该状态通过入口文本说明。不能根据当前仓库状态补造会话 diff，也不能把不完整记录解释成工作区没有变更。
