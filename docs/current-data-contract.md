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

## Skills 与项目指导

Skill / command discovery 只使用 Agena 自己的当前 roots，例如：

- Agena home 的 skills/commands；
- workspace `.agena/skills`；
- workspace `.agena/commands`。

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
