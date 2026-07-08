# Plugin / Tool 复用评估

这份评估基于两部分信息：

- 仓库当前实现：`docs/plugin.md`、`docs/architecture.md`、`crates/agena/src/plugins/provided/*`、`crates/agena-mcp-client`、`crates/agena-mcp-server`
- 外部调研结果：`/home/canxin/Git/ai/temp/ai_tool.md`

目标不是重新设计 Agena runtime，而是回答两个更实际的问题：

1. 哪些外部项目适合替代我们现在自研的 plugin/tool 底层实现，从而降低维护成本。
2. 哪些外部项目更适合接到现有内置 plugin 下面，用来增强能力，而不是替换当前 tool set。

## 结论摘要

最值得做的不是推翻现有 plugin host，而是保留现在已经很清晰的 `agena.*` static plugin surface，只替换几个维护成本高、协议变化快、或者安全边界脆弱的底层实现。

优先级最高的建议：

1. 用官方 MCP Rust SDK `rmcp` 替换或包裹现有 `agena-mcp-client` / `agena-mcp-server` 协议栈。
2. 保留 `agena.process` tool API，但把执行后端抽象出来，优先接入真正的 sandbox backend，而不是继续只靠本地进程执行。
3. 不自建浏览器自动化 plugin；直接通过 `agena.mcp` 接入 Playwright MCP 一类现成 server。
4. 保留 `agena.skills`，但把 skill/frontmatter 向公开格式靠拢，减少自定义规范维护成本。
5. 增强代码理解能力时，不要造新的“全家桶 agent 框架”；直接给内置 tool 增加 `ast-grep` / `tree-sitter` 这一层能力。

不建议做的事情：

- 不建议把整个 runtime 切到 Rig、Swiftide、LangChain、LangGraph 之类框架上。
- 不建议把 `agena.fs`、`agena.runtime` / `agena.plan` / `agena.snapshot` 这类和宿主强绑定的 tool 改成外部 MCP server。
- 不建议为了当前规模的 tool catalog 引入 Meilisearch 这类重型服务。

## 当前仓库的边界

从当前实现看，Agena 的抽象边界已经基本正确：

- `PluginHost` 是统一扩展平面，内置 static plugin 和外部 plugin 走同一路径。
- 内置能力已经按领域收敛成稳定入口：`agena.fs`、`agena.process`、`agena.web`、`agena.tools`、`agena.runtime`、`agena.plan`、`agena.tasks`、`agena.snapshot`、`agena.skills`、`agena.lsp`、`agena.memory`、`agena.mcp`、`agena.settings`。
- 对模型暴露的是高层 action tool，而不是零散 syscall 级工具。

这意味着最合理的策略是：

- 保留当前模型可见 tool 名称和 action 形状。
- 替换 tool 背后的协议栈、执行引擎、索引引擎、浏览器引擎。
- 只在确实缺能力的地方新增 action 或新 static plugin。

## 评估矩阵

| 领域 | 当前仓库实现 | 调研候选 | 建议 | 优先级 |
| --- | --- | --- | --- | --- |
| MCP 协议 | `crates/agena-mcp-client` + `crates/agena-mcp-server` 自研 | `rmcp` / 官方 Rust SDK | 适合替换底层协议实现 | P0 |
| Process 执行 | `agena.process` + 本地 executor/background process registry | `microsandbox`、Daytona、E2B、SWE-ReX 思路 | 保留 tool API，替换执行 backend | P1 |
| 浏览器自动化 | 当前无专门内置 browser plugin | Playwright MCP | 不自研，直接接入 `agena.mcp` | P1 |
| Skills 格式 | `agena.skills` 扫描 `SKILL.md` / slash command | GitHub Agent Skills、Vercel Skills、AGENTS.md | 保留实现，增强兼容层 | P1 |
| 代码结构化搜索 | 当前以 `glob`/`grep`/LSP 为主 | `ast-grep`、`tree-sitter` | 适合增强内置 tool | P1 |
| Web 搜索/抓取 | `agena.web` 内置 search/fetch/crawl | 独立 search MCP / 第三方 server | 保持本地内置，不默认依赖外部搜索服务 | P2 |
| Memory/RAG | `agena.memory` 已是文件持久化 + Tantivy 本地检索 + prompt 注入 | Qdrant、LanceDB、mem0 思路 | 适合新增更强 RAG/长期记忆能力，不适合替换当前本地 memory | P2 |
| MCP 安全 | 当前主要靠权限系统和网络审计 | `mcp-firewall`、`mcp-scan`、`agent-scan` | 适合加在接入层和 CI，不适合塞进 tool body | P2 |
| Tool discovery | `tool_search` 当前是内置 Tantivy 本地检索 | Meilisearch | 保持内置实现，暂不引入外部服务 | P3 |
| 观测/Eval | 已有 OTEL、事件、trace | Langfuse、Phoenix、Promptfoo | 适合外围集成，不替换 core | P3 |
| Agent 框架 | 自研 runtime | Rig、Swiftide、LangGraph、LangChain 等 | 只借鉴设计，不直接引入 | P3 |

## 一类：适合替代现有自研实现

### 1. `agena-mcp-client` / `agena-mcp-server` 最适合被 `rmcp` 替换

这是目前最明确的“用外部项目减轻维护负担”的点。

原因：

- 你们已经自己维护了一套 MCP 协议栈，包含 client、server、stdio/http/ws transport、token store、manager。
- 这类协议变化快，兼容性问题多，而且社区生态最终会围绕官方 SDK 收敛。
- 当前实现里已经直接把协议版本写在自研 crate 注释里，后续跟进规范演进会持续产生维护成本。

为什么这个替换值得做：

- 协议对外兼容性比功能创新更重要。
- `agena.mcp` 已经把模型可见面收敛成统一 `mcp` tool，这让底层替换不会影响模型调用面。
- 你们还提供了 MCP server 输出能力，双端都自己维护时，规范升级成本会翻倍。

建议替换策略：

1. 保留 `McpConnectionManager` 这层宿主抽象，不直接把 `rmcp` 散到 runtime 各处。
2. 先让 `agena-mcp-client` 内部改成基于 `rmcp` 的 adapter。
3. 再把 `agena-mcp-server` 改成 `rmcp` backend adapter。
4. 最后只保留你们自己的：
   - server 配置解析
   - token store / auth materialization
   - host callback 映射
   - `agena.mcp` 的模型可见 action 设计

不建议替换的部分：

- 不要把 `agena.mcp` 的单一入口 tool API 改回“一台 server 一个 tool”。
- 不要把 MCP server 配置和 host capability 管理交给外部 SDK；这些仍然应该是 Agena runtime 自己的边界。

### 2. `agena.process` 的执行后端应该抽象，优先接 sandbox，不优先继续自研隔离

当前 `agena.process` 的 tool 设计本身是对的：

- 模型必须显式声明 `filesystem_effects` 和 `network_effects`
- 前台 `run` 和后台 process log/stop 已经统一成同一个 process tool
- 权限系统已经能在执行前做检查

真正重的是执行隔离，不是 tool API。

建议：

- 保留 `process` 这个顶层 tool。
- 在 executor 后面增加 backend 抽象，例如：
  - `local`
  - `sandbox_local`
  - `sandbox_remote`

外部项目怎么用：

- `microsandbox`：最适合做本地 Rust 方向的可选 backend 试点。
- Daytona / E2B：更适合作为远端/弹性环境 backend，而不是默认内置依赖。
- SWE-ReX：更适合借鉴 runtime 抽象，而不是直接成为依赖。

不建议的做法：

- 不要把 `agena.process` 直接替换成外部 MCP shell server。这样会把最敏感的执行边界搬出 runtime。
- 不要把人类可审计的 `filesystem_effects` / `network_effects` 合约丢掉，即使接入 sandbox backend 也要保留。

## 二类：不替换当前内置 plugin，但很适合增强

### 3. 用 Playwright MCP 增强浏览器能力，不自建 `agena.browser`

你们当前用 `agena.web` 统一承载 search/fetch/crawl/index；它不是 browser automation。

对浏览器这类高变动、高兼容性成本的领域，自研 plugin 不划算。最合适的路线是：

- 继续保留 `agena.web` 负责轻量 search/fetch。
- 让 `agena.web` 的 crawl action 承担多页抓取、本地语料落盘和后续复用。
- 通过 `agena.mcp` 接入 Playwright MCP 作为 browser/computer-use 能力。

建议增强项：

- 给 `agena.mcp` 增加浏览器 server preset，降低配置成本。
- 给 `agena.skills` 内置几条 browser 相关 skill，指导模型先 `mcp list_prompts` / `call` 再执行页面动作。
- 在权限/风险层给 browser 类 MCP server 单独打标签，避免它和普通 read-only MCP server 混在一起。

这条路线的好处是：

- 浏览器演进留给 Playwright 生态。
- Agena 只维护接入、权限、tool presentation、skill 指南。

### 4. 用 `ast-grep` / `tree-sitter` 增强代码工具，比引入大框架更值

当前 `agena.fs` 很适合做文本级读写和 patch，但结构化代码理解仍然偏弱：

- `glob` / `grep` 是文本级别
- `agena.lsp` 适合导航，但不适合结构化批量匹配和重写

建议新增一个轻量 `agena.code` static plugin，或者给 `agena.fs` 扩 action：

- `search_ast`
- `symbol_context`
- `rewrite_ast` 或更保守的 `plan_rewrite`

外部项目的合理用法：

- `ast-grep`：优先用来做结构化搜索和规则式重写。
- `tree-sitter`：优先用来做上下文切片、节点提取、symbol 附近代码块。

为什么这比接 LangChain/Rig/Swiftide 更值：

- 这是直接补足当前 coding agent 最缺的工具能力。
- 它增强的是内置 tool，而不是替换整个 runtime 控制流。

### 5. `agena.skills` 应该兼容公开 skill 格式，而不是继续只维护私有约定

当前 `agena.skills` 已经做了很有价值的事：

- 扫描 skill roots
- 动态注册 tool
- 通过 prompt body 生成 workflow text

最适合减轻维护压力的方式不是重写实现，而是让格式对齐已有生态：

- 兼容 GitHub Agent Skills 风格 frontmatter
- 吸收 Vercel Skills 里的 scripts/resources 概念
- 读取 repo 根的 `AGENTS.md` 作为额外 instruction source

建议增强项：

- 在 `SKILL.md` frontmatter 中支持：
  - `tools`
  - `risk`
  - `scripts`
  - `resources`
  - `when_to_use`
  - `aliases`
- 把 `AGENTS.md` 映射成一个只读 skill 或系统指令层
- skill 注册时，把这些元数据转成 tags / help / summary

这会带来两个收益：

- 新增 skill 时，团队更容易复用公开样板
- 你们不用长期独自维护一套完全私有的 skill 规范

### 6. `agena.memory` 应该继续保留“文件记忆 + 本地检索 + 可调用工具”的单一实现

当前 `agena.memory` 更像是：

- 以 workspace 文件作为 durable memory source
- 通过模型可见 `memory` tool 提供 `search` / `get` / `list` / `write` / `delete`
- 用进程内 Tantivy 做本地全文检索
- 把项目指令和相关记忆注入 prompt

这条路线对当前产品形态是合理的：没有外部服务依赖，索引和 memory 文件保持同一宿主边界。

建议不要直接引入 mem0、Letta、Graphiti 作为核心依赖。更稳的路线是：

1. 保持现有 workspace 文件作为唯一 durable source of truth。
2. 保持本地 Tantivy 作为唯一全文检索实现，不再引入可切换 backend。
3. 只有在明确需要跨项目向量检索或长期记忆策略时，再考虑叠加：
   - Qdrant / LanceDB 作为向量层
   - mem0 一类策略层用于提取、排序、更新

适合借鉴的不是依赖本身，而是这些项目的思路：

- mem0：memory extraction / ranking / update 策略
- Letta：stateful memory 产品表达
- Graphiti / GraphRAG：时间敏感和图结构记忆

## 三类：可以增强现有内置 plugin 的具体建议

### `agena.mcp`

建议新增的 action：

- `servers`：列出已连接 server、transport、能力、最近错误、网络目标
- `refresh`：重连某个 server 或刷新 snapshot
- `add_server` / `remove_server`：利用已有 host MCP registry callback 做运行时管理
- `inspect_tool`：返回某个 MCP tool 的原始 schema、描述、server 来源

建议新增的运行时能力：

- server preset 模板，优先覆盖 `filesystem`、`github`、`playwright`
- 每个 server/tool 的风险标签和权限摘要
- MCP tool 调用失败的结构化错误归类，而不是只返回文本

### `agena.process`

建议新增的能力：

- backend 选择：`local` / `sandbox`
- 执行工件回传：stdout/stderr 分离、退出码、生成文件列表
- 对后台 process 任务支持更明确的“完成/仍在运行”状态语义
- 针对常见语言的 execution preset，例如 Rust/Node/Python 测试命令模板

### `agena.fs` 或新增 `agena.code`

建议新增的 action：

- `search_ast`
- `symbol_context`
- `read_block`
- `list_symbols`

其中：

- `search_ast` 用于减少 `grep` 命中噪声
- `symbol_context` 用于在模型修改前给出足够但不过量的上下文
- `read_block` 可以按符号/节点范围返回代码，而不只是按行切片

### `agena.skills`

建议增强项：

- 支持 scripts/resources
- 支持 `AGENTS.md`
- 支持 skill 级风险标签
- 支持 skill 对所需 tool 的显式声明，便于 runtime 提前提示或筛选

### `agena.web`

建议保持本地内置，不要做成远程服务依赖，也不要做成通用浏览器自动化。

可以增强的点：

- 为 search 结果返回更结构化的来源 metadata
- 继续增强多搜索引擎解析质量
- 在 `fetch` 里提供更稳定的正文提取模式
- 多页 crawl
- URL 规范化和去重
- 本地持久化语料和 metadata cache
- 本地全文/混合检索

不建议放在这里的能力：

- 需要远程托管服务才能运行的抓取核心路径
- 浏览器自动化和复杂交互

## 四类：只借鉴思路，不建议直接引入依赖

### Agent 框架：Rig / Swiftide / LangChain / LangGraph / CrewAI / AutoGen

这些项目更适合拿来参考：

- agent builder 怎么组织
- subagent / workflow 怎么表达
- RAG pipeline 怎么拼装

不适合直接成为 Agena 的基础依赖，原因很简单：

- Agena 已经有自己的 runtime、plugin host、session、permission、tool loop
- 再套一层 agent framework，会形成双控制面
- 维护成本不会下降，反而会上升

### RAG 产品层：mem0 / Letta / Graphiti / GraphRAG

这些更适合借鉴：

- memory item 分类
- ranking / recency / extraction 策略
- graph memory 的数据组织

不适合直接塞进当前 core runtime，除非你们明确要做“长期记忆平台”。

### Tool search 引擎：内置 Tantivy

当前 `tool_search` 是小规模 catalog 搜索。现阶段：

- tool 数量不大
- catalog 主要由内置 tool、skills、MCP server 组成
- 查询复杂度不高

继续引入 Meilisearch 这类外部索引服务的收益还不够覆盖：

- 新服务依赖
- 索引同步
- 部署/配置成本

只有在你们把 marketplace、skills、MCP tools 做到很大规模时，再考虑把 tool discovery 升级成真正索引服务。

## 分阶段落地建议

### Phase 1

建议先做这些，收益最高：

1. 为 MCP 栈做 `rmcp` 兼容性 spike。
2. 给 `agena.mcp` 增加 `servers` / `inspect_tool` / `refresh`。
3. 给 `agena.skills` 增加公开 frontmatter 字段兼容层。
4. 为 `agena.process` 设计 backend trait，为 sandbox 接入留接口。
5. 通过 preset/documentation 把 Playwright MCP 纳入标准接入路径。

### Phase 2

在第一阶段稳定后继续：

1. 新增 `agena.code` 或为 `agena.fs` 增加 AST action。
2. 让 `agena.memory` 拥有真正的 read/write/search action。
3. 在 CI 或安装流程中加入 `mcp-scan` / `agent-scan` 这类安全检查。

### Phase 3

只有在明确产品需求后再做：

1. 向量/全文/混合记忆后端
2. 更重的 observability/eval 平台集成
3. marketplace 规模化后的 tool discovery 升级

## 最终建议

如果目标是“减轻维护压力”，最应该外包的是：

- MCP 协议实现
- 浏览器自动化实现
- 沙箱基础设施
- skill 规范定义

如果目标是“增强内置工具”，最应该自己继续掌握的是：

- tool design
- 权限模型
- host callback 能力
- session/workflow/worktree/settings 这类宿主绑定逻辑
- 代码修改产物和事件日志的结构化表达

一句话总结：

**保留 Agena 自己的 runtime 和内置 plugin 边界；把协议、浏览器、沙箱、skill 格式、结构化代码分析这些“高变化底层”尽量复用外部生态。**
