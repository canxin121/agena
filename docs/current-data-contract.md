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
- Web 的刷新队列限制同一数据源只有一个请求在运行，并保留运行期间收到的一次后续刷新。高频事件不能不断推迟首次刷新；失败读取采用有上限的退避。TUI 同样合并失效提示，按会话身份、请求标识和订阅代次拒绝过期结果。已显示的文本 part 直接合并实时快照，语义变化走 250 ms 刷新门限，另有 5 秒状态核对兜底。
- 文件面板通过 `/api/v1/workbench/fs/stream` 按需监听根目录、展开目录和选中文件的父目录，最多 128 个附加目录、每批 256 个变化路径；监听非递归，流断开即释放。服务端按 200 ms 合批，Web 按 500 ms 合并读取，只重读受影响的已显示目录和已加载分页范围。展开目录超过监听预算时，额外保留 10 秒一次的已加载目录核对；目录删除或替换后重建监听。面板隐藏时关闭监听，恢复后重新核对。保存响应仅确认实际提交的文本，期间的新编辑仍保留为未保存草稿；跨文件或工作区的旧响应不能覆盖当前文件。TUI 的文件补全索引在使用时重新读取，并限制为每 10 秒至多一次。

这些保证针对单个服务端进程与其客户端之间的收敛、顺序和有界工作量。网络断开期间无法提供新状态；恢复后通过权威读取收敛。消息发送失败后的未知结果不能通过无条件重发解决，客户端不自动重放没有幂等身份的发送命令。
