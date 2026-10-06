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

开发环境在 schema 改变后应删除旧数据库并重新创建。

### Scheduler SQLite

Scheduler 使用独立数据库，但采用相同原则：

- 空库创建当前表和索引；
- 非空库必须精确匹配；
- 不保存 schema version；
- 不迁移或修补旧结构。

Scheduler 的 job/history JSON 同样只接受当前字段集合。当前 writer 总会写出的 policy/status 字段是必填项；未知字段直接拒绝。

### Server-state SQLite

Server state 使用唯一当前目录和 `agena.db`，没有旧目录候选选择。

数据库只包含当前 server-state / attachment-cache 表与索引；非空数据库定义不匹配时拒绝启动。

## 本地持久化状态

以下状态只接受当前 shape：

- installation id；
- MCP server control / OAuth signing key / OAuth runtime；
- terminal session registry；
- terminal UI state；
- workspace preview registry；
- TUI composer draft；
- TUI prompt history；
- session persisted execution config；
- marketplace `installed.json`；
- cloud-media handle records；
- memory frontmatter。

client 不持有自己的 session 状态：session / execution / background-activity 的权威状态只在服务端，TUI、Web、CLI、IDE 都是纯展示客户端，不落盘自己的副本，也不写入共享数据库。每个数据目录同时只有一个服务端进程；服务端启动、打开会话和每次执行结束时收敛该目录里所有被遗留的 in-flight run（`process_restart`），客户端不参与这一过程。

共同规则：

- 未知字段拒绝；
- 当前必填字段缺失拒绝；
- 文件不存在可以初始化新状态；
- 文件存在但坏 JSON、旧 shape 或不完整 shape 不会自动改写成当前格式；
- 不通过 “version=0 → version=1” 或缺字段补默认来升级旧状态。

业务上真实可选的值仍可以使用 `Option`，当前 writer 主动省略的空集合也可以由当前 schema 明确声明为可选。这与旧版本迁移无关。

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

历史操作使用持久化 `run` marker 的数值 `part_id` 作为消息身份，HTTP、command、Web 与 TUI 都通过 `at_message_id` 指定目标。

- `fork` 创建子会话，保留截至目标消息末尾的历史（包含目标消息）；省略目标时保留源会话的完整历史。源会话必须已有可用的历史边界。
- `rewind` 只接受源会话中的已完成用户消息，创建只保留该消息之前历史的子会话，并把目标输入恢复到新分支的输入框。回退首条用户消息时，新分支历史为空。
- 两种操作都返回包含新会话的 execution resource；客户端打开返回的新会话。原会话及其输入草稿继续保留，分支后续写入不会改写原会话。
- 继承的历史可以包含仍在流式更新的父会话 part；它们只作为历史展示，不计入新分支的运行、待执行工具或待答交互。新分支的执行状态只由它自己创建的 part 决定。
- 显式复制或导出对话记录读取完整的用户可见分页历史；聊天界面的当前加载窗口不构成导出边界。

`fork` / `rewind` 不恢复工作区文件。已完成的文件修改通过项目 Git 历史恢复；`fs.apply_patch` 只输出操作标识、文件变更、diff 和进度，不提供反向补丁或撤销哈希。Git 提交、危险操作确认和发布的行为约定见 [Git 工作流程与文件恢复](git-workflow.md)。

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

provider 回放状态只保存在 run 的对应 round 中，不同时复制到 run 的独立 provider-state 列；其中与 think 正文或 encrypted content 重复的文本保存为 part 内容引用，在发送请求前恢复原始协议值，签名、顺序和不同的 opaque 事实保留。Web 与 TUI 的默认策略仅展开 answer 和 text，think、工具、附件及待答交互默认折叠；用户显式配置和手动展开优先。

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

- 服务端先建立订阅，再允许客户端读取快照。Web 在每次 SSE 建立连接后重新核对所有仍挂载的聊天面板；TUI 重连后读取同一会话版本的执行状态和最近一页 transcript。断线、队列溢出、恢复前台和显式删除都有恢复路径。
- `SessionTranscriptPart`、实时 part 和工具详情携带 `revision` 与 `updated_at_ms`。客户端按这两个字段依次比较，拒绝旧响应覆盖新内容；完整快照中省略的字段不能继续沿用旧值。执行状态和会话摘要使用会话 `version`，执行响应中的 `latest_event_seq` 对应其实际读取的会话版本。
- 流式文本和推理仍先保留在内存中，正常情况下在完成时落盘，超长流沿用有界检查点。每个缓冲 part 最多每 100 ms 发布一次末尾快照，暂停输出后也会发布最后一次变化；该快照不增加持久 revision，使用单调递增的更新时间区分同一 revision 内的内容。fork 中已加载的共享 part 同样更新。
- `GET /api/v1/sessions/{id}/parts?ids=...` 每次最多接收 256 个正整数 ID，与分页参数互斥，只返回该会话中对用户可见的对应成员。客户端重连时按批核对**已经加载**的 ID，移除缺失成员；不会为了恢复而遍历尚未展开的历史。SQLite 按成员关系查询这些 ID，工具详情也通过单个 ID 读取。
- 会话删除有独立的 `session_deleted` 通知，包括空会话和被级联删除的子会话。通知保留 workspace ID，删除数据库记录后仍能按工作区投递。不存在的会话读取返回 404；客户端保留删除标记，拒绝迟到请求重新创建已删除内容。
- Web 核心 API 的资源版本是 `server UUID:monotonic_updated_at_ms`。逻辑更新时间取墙钟时间与上次值加一的较大值，同毫秒修改、墙钟回拨和服务端重启都不能复用旧版本。版本按会话列表、工作区列表、工作区会话列表、会话执行状态、会话正文、文件修改、计划、part、工具 input/metadata/output 分区和 activity 描述/日志独立维护。SSE 通知携带相关 `resource_revisions`，只唤醒版本实际变化的订阅者。已显示的文本、推理和完整 activity 描述直接合并流快照，日志变化不触发全量列表或执行状态重读。核心 API 与预览 registry 的 UUID 属于独立来源，分别检测重启与拒绝旧响应；预览版本不能使核心 API 版本失效。
- 侧栏按 `workspaces:catalog`、`workspace:{id}:sessions:roots`、`workspace:{id}:sessions:parent:{parent}`、`workspace:{id}:sessions:bucket:{kind}`、`workspace:{id}:stats` 和全局 `sessions:bucket:{kind}` 隔离。目录 A 内某个子会话的标题变化只核对其直接父级的已展开列表；根列表、其他父级分支、无关置顶列表和 B、C 目录保持不变。子会话增删或更换父级会改变父行的 child count，必须连带核对父行所在列表。分页或排序发生真实变化时重读该查询的当前页，未改变的实际 Vue 行对象继续复用。关闭分支释放对应订阅，每个当前查询保留一页及其实际响应版本，独立于共享正文缓存的淘汰。
- 统计使用 `GET /api/v1/workspaces/session-stats?ids=...` 合批，只返回受影响目录的计数和读取前捕获的版本，单批最多 128 个目录。项目目录 GET 不附带会话统计。新增或删除项目才核对目录集合，移动会话更新旧、新目录；未展开目录只更新统计，不枚举历史。全局置顶、收藏及 recent/running/attention 汇总的行版本与 `sessions:bucket:{kind}:count` 分离。折叠 footer 使用 `count_only=true`，只计算 total，不加载会话行或投影状态；已知 membership 不变时的标题或行内容变化不唤醒计数读取。遇到未知执行状态、关联 memo 淘汰或跨进程修改时保守核对相关汇总及受影响目录，避免错误地认为成员数未变。
- 已挂载会话的恢复先合批核对 `session:{id}:transcript` 和 `session:{id}:state`。正文与状态各自保留实际 HTTP 响应版本，未改变时不下载正文、不核对 loaded part membership，共享正文缓存被淘汰也保持这一行为。正文确实变化后只恢复最近一页和已经加载的成员；其他可见会话保留原值。发送或 continue 的确认仍强制条件读取，不能因为完成通知丢失而复用发送前的正文。计划文档和已初始化 activity 列表也保留自己的实际响应版本，恢复前台时未变的内容直接复用；手动刷新仍执行条件校验。
- 侧栏创建、删除、收藏和置顶操作，以及单目录手动刷新、展开和分页，同样进入相应范围的读取。全局聊天 bootstrap 列表的实时 metadata 和删除直接更新缓存，可见会话执行状态单独订阅；只有版本兜底发现通知缺口，或重连恰好撞上未完成的首次列表读取，才恢复该列表。
- 工具详情只订阅已经展开的 `part:{id}:input|metadata|output` 分区，输出变化不会调度其他分区。组件保留当前分区正文对应的实际 HTTP 响应版本，即使共享正文缓存被淘汰，未变化的已显示分区仍直接使用现有值。终态只核对 output 和被取消的未完成读取；正文先进入终态、详情版本通知尚未到达时，直接条件校验尚未追上终态的 output，避免等到慢速兜底。隐藏、折叠或卸载解除对应订阅并取消请求。
- 同一页面实例内所有挂载订阅者共用一个 30 秒版本兜底队列，通过 `GET /api/v1/changes/revisions?resources=...` 每批核对至多 128 个资源，未变化时不读取正文。通用可见资源读取器在首次打开没有缓存的资源时直接读取正文，以响应 ETag 初始化版本，避免额外的先查版本请求。恢复前台、重连或通知缺口先合并失效提示，再核对版本；隐藏时停止周期检查。跨进程的持久化修改通过服务端共享的 30 秒元数据检查发现，只读取 session ID、workspace ID、version 和 workspace revision rows，不加载历史、part 或统计全文；客户端与服务端两层检查的边界可能叠加，不承诺所有外部修改在 30 秒内到达。
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

后台任务的启动 part 是已完成的启动回执，后台生命周期由独立 activity 和 durable operation 表示。输入框上方的任务面板从 activity 状态和日志接口读取输出，不以启动回执的 completed 状态判断进程结束。普通管道和 PTY 日志用 `chunk=true` 保留原始换行；带正则条件的 monitor 使用行记录。委派任务的日志游标标识 run，同一个 run 的回复继续生成时，客户端替换该游标的文本快照，不重复追加，也不跳过其后续内容。

展开的启动 part 按 `source_part_id` 关联 activity，展示独立的实时状态与日志，保留 durable 回执的原始含义。Web 任务面板和 part 共用日志资源与条件读取；TUI 只为主界面视口内展开的 part 读取日志，最多同时跟随 16 个 activity，保留至多 64 个展示尾部，并优先淘汰视口外缓存。关闭、离开视口、切换会话和退出 TUI 都释放适用的读取。委派任务中的前台命令同样把运行输出投影进其当前 run 的日志快照。

正常 `agent.stop` 只在当前执行没有待处理输入、没有未被 provider 完整观察的后台通知、没有仍可唤醒会话的后台操作或定时任务时派发。后台等待会释放当前 execution，让 durable 通知可以唤醒会话；它不会发出正常完成 hook。正常停止决策与通知提交通过同一会话 mutation lane 排序，该决策之后的新到达事件由后续唤醒处理。已暂停、删除或完成的定时任务不阻止停止；周期任务只要还有下一次触发，仍属于等待。已完成 provider round 的一次性定时通知可以证明本次触发已被处理，不依赖必须等 execution 释放的 scheduler ack。失败仍携带 `run_error` 到达 stop hook；终端 Done 提醒只在所有 stop 决策 hook 同意停止后发出。

`GET /api/v1/sessions/{id}/file-changes` 使用会话 membership 内归属本会话的工具记录投影编辑。SQLite 在查询和反序列化前限制 `origin_session_id` 与 kind，不加载普通文本、run 或 fork 继承的工具内容；存储 facade 仍合并适用的流式 overlay。只接受成功完成的 `fs.write`、`fs.replace`、实际应用的 `code.rewrite_ast` 和 `fs.apply_patch` 结果。fork 继承的编辑、预览、失败和可证明的 no-op 不计入新会话。连续且完整的 before/after SHA 链回到起始 revision 时可以消除净零变更；缺少基线或存在断开的链时展示记录的操作历史。该 API 支持摘要、文件分页和单文件详情，diff 返回预算上限为 2 MiB。

每个文件摘要携带由该路径完整编辑事实计算的 fingerprint，不依赖其他文件或当前 diff 截断预算。已选 diff 只在其自身 fingerprint 变化时自动读取，其他文件的变化只更新文件列表；同一 fingerprint 在隐藏恢复和重新展开时复用当前保留的 diff。服务端为了计算净 SHA 链仍读取本会话工具记录，并未实现持久化的逐文件增量索引。

文件资源时钟使用和 API 投影一致的编辑事实，而不是整个 part 的更新时间。无关工具、metadata、重复的写入声明和同一编辑结果的重复更新不会推进 files 版本。首次读取只在有界 memo 中登记缺失的基线，结果解析在资源时钟锁外完成；期间的修改或删除拥有更新的基线，迟到的登记不能覆盖它。

会话编辑面板不读取 Git 工作树。Shell 的 `writes` 声明只能说明执行者预期写入的范围，不能证明某个文件确实发生编辑；未记录实际编辑结果时返回 `recording_incomplete`，由客户端明确显示记录不完整。不能根据当前仓库状态补造会话 diff，也不能把不完整记录解释成工作区没有变更。
