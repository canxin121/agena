# Agena 内置工具与第三方替代方案调查

调查日期：2026-10-04。源码基线：`740b54df4107ade81242a335c49bca590c52c1cd`。调查分支：`research/tool-alternatives`。独立 worktree：`/Volumes/Rc20/Projects/agena-tool-alternatives`。

**后续实施入口：** 本文保留最初的调研基线与候选比较；独立 `feat/tool-modernization` 分支中的实际接入、缺陷修复、最终候选取舍和验证状态见[实施台账](tool-modernization-progress.md)。以下“当前实现”和 135 个定义均指调查基线，现代化实施完成时的目录为 138 个定义。2026-10-05 移除托管工作区快照后，当前目录为 21 个插件、135 个定义；下文关于保留 `snapshot.*` 的建议已被该决定取代，文件恢复与版本历史交给 Git。

本次只做源码调查、第三方官方资料核对和独立 CLI 基准测量，没有改动产品实现，没有安装候选工具，没有调用任何 Agena tools。目录中的脚本只运行 `git`、`rg`、`grep`、`fd`、`find`，不启动 Agena。

**建议先做工具选择与能力发现，再做有证据的实现替换。** `fs.grep` 已经采用 ripgrep 核心库，`code.search_ast` 已经采用 ast-grep，网页抓取和记忆检索也已经复用第三方库。最明确的改进是让 AI 在 shell 中选对命令、给内置搜索补齐有用参数，以及评估成熟浏览器后端和正式搜索 API。

文中优先级是建议：**P0** 表示首批、低迁移成本；**P1** 表示值得做针对性原型；**P2** 表示按场景选装。收益区分为本机实测、源码/官方功能证据、尚待验证的工程判断，不把项目宣传的倍率当作 Agena 的实测收益。

## 1. 当前内置工具覆盖范围

基线提交中的[生成工具参考][R1]列出 **22 个插件、135 个工具定义**。这是源码中的静态清单，不等于某个正在运行的 session 实际启用的工具数；本次没有调用运行中的工具目录。下表覆盖全部插件，后续按能力合并比较，避免给同一实现重复推荐替代品。

| 插件 | 数量 | 主要能力或完整工具集合 | 调查结论 |
| --- | ---: | --- | --- |
| `fs` | 8 | `read`、`read_many`、`stat`、`glob`、`grep`、`apply_patch`、`write`、`replace` | 搜索引擎已复用；改进参数与调度；保留读写协议 |
| `code` | 2 | `search_ast`、`syntax_tree` | 已内嵌 ast-grep / Tree-sitter |
| `lsp` | 5 | `definition`、`references`、`hover`、`diagnostics`、`servers` | 已桥接外部语言服务器；可扩展语义编辑 |
| `web` | 13 | 搜索、抓取、爬取及 10 个浏览器操作工具 | 浏览器和搜索服务是主要替代候选 |
| `shell` | 7 | `run`、`list`、`logs`、`write`、`resize`、`signal`、`stop` | 保留生命周期管理，改善它启动的命令 |
| `memory` | 5 | `search`、`get`、`list`、`write`、`delete` | 已采用 Tantivy，不宜为替换而换检索引擎 |
| `notebook` | 1 | `edit_cell` | 保留修订检查；可用 nbformat 对照验证 |
| `snapshot` | 3 | `enter`、`exit`、`status` | 已接 Git worktree / Rift |
| `monitor` | 2 | `start`、`stop` | 保留事件通知协议；文件监听可接现成 CLI |
| `cron` | 7 | `create`、`update`、`delete`、`list`、`history`、`pause`、`resume` | 会话唤醒语义属于 Agena，系统 cron 不等价 |
| `plan` | 6 | `set`、`edit`、`get`、`clear`、`phase`、`review` | 保留计划状态、版本与审批语义 |
| `tasks` | 7 | `run`、`get`、`list`、`output`、`message`、`followup`、`cancel` | 保留子任务、取消和结果交付协议 |
| `interaction` | 2 | `ask`、`notify` | 客户端交互协议，无通用 CLI 等价替代 |
| `report` | 1 | `findings` | 文件/行号结构化结果协议，保留 |
| `session` | 5 | `get`、`environment`、`model`、`tokens`、`rename` | 保留；建议补充宿主 CLI 能力目录 |
| `settings` | 7 | `get`、`inspect`、`list`、`set`、`patch`、`delete`、`validate` | 配置分层与校验属于 Agena |
| `commands` | 6 | `get`、`install`、`list`、`read_resource`、`refresh`、`remove` | 保留命令/技能发现和管理 |
| `mcp` | 9 | 工具搜索/调用、资源和模板、prompts、连接状态/重连 | 接入第三方能力的现有入口 |
| `tools` | 7 | `help`、`list`、`search`、`tags`、三个 `plugins_*` | 保留发现网关，改善排序/路由即可 |
| `chatgpt` | 11 | 云端搜索、计算、文件、图像、文档等 | 已是外部服务包装；按能力选择，不能假定本地等价 |
| `claude` | 9 | 云端搜索、抓取、计算、文件、理解和 advisor | 同上 |
| `gemini` | 12 | 云端搜索、Maps、URL、计算、文件、图像、文档等 | 同上 |
| **合计** | **135** | 云端服务三个插件共 32 个定义 | 不能把 135 个入口都视作 135 个自研引擎 |

## 2. 内置实现应如何利用第三方项目

| 能力 / 当前实现 | 第三方候选 | 实际收益及证据 | 迁移边界与建议 | 优先级 / 成本 |
| --- | --- | --- | --- | --- |
| **`fs.grep`**：`grep-regex` + `grep-searcher` + `ignore`；目录按路径排序、逐文件搜索 [R2]、[R3] | [ripgrep](https://github.com/BurntSushi/ripgrep) 的现有库；必要时可选 `rg --json` 后端 | 引擎已经相同生态。并行遍历/搜索可能改善大目录吞吐；增加固定字符串、上下文、只列文件/计数可减少输出和调用次数。后两者是功能收益，内置提速尚未实测 | 首选保留 Rust 内嵌实现，增加结构化参数与可选并行策略。并行需要重新设计总预算、取消及结果顺序；不能直接用无界 CLI 输出替代现有结果契约 | **P1 / 中** |
| **`fs.glob`**：`ignore` + `globset`，分页、确定性顺序 [R3]、[R4] | [fd](https://github.com/sharkdp/fd)、`rg --files`，或继续使用 `ignore` | CLI 文件发现更易组合；本机 `fd` 比 `find` 有实测收益，但不能据此宣称快于内置 glob | `rg --files` 只枚举文件；内置 glob 可以返回目录。保留分页和默认忽略规则；大仓库先测遍历/排序成本，再选择并行或索引 | **P1 / 中** |
| **`fs.read/read_many/stat`**：分页流式读、总字节预算、附件及 SHA-256 [R5]、[R6] | 标准文件 I/O；[bat](https://github.com/sharkdp/bat) 只作终端预览 | 已有适合模型的输出控制。bat 的价值是展示功能，没有证据说明启动 bat 比内嵌读更快 | **保留**内置读；优先使用已有 `read_many` 合并小文件读取。不要为语法着色增加进程与输出 | 保留 / 低 |
| **`fs.apply_patch/write/replace`**：自定义 patch、变更记录、修订检查、文件锁和原子文件发布 [R6]、[R7] | `git apply`、[sd](https://github.com/chmln/sd)、ast-grep rewrite | 批量文本或结构改写可由第三方生成候选改动；不是现有写入协议的直接替代 | `*** Begin Patch` 与 unified diff 不兼容。`sd` 不替代 SHA-256、出现次数和并发检查；保留 Agena 的校验与发布层。现有回滚也不是跨进程/断电事务保证 | **保留**；按需加适配 / 中 |
| **`code.search_ast/syntax_tree`**：ast-grep + Tree-sitter [R8] | [ast-grep](https://github.com/ast-grep/ast-grep) 的规则、rewrite；[Semgrep](https://github.com/semgrep/semgrep) 的规则分析 | AST 搜索已经具备；可补规则文件、重写预览。Semgrep 适合更专门的模式/安全分析，不是更快的普通字符串搜索 | 优先扩展现有 ast-grep。Semgrep 做可选任务能力；社区引擎、规则集和商业能力分别评估，不假定完整功能都可直接嵌入 | **P1 / 中** |
| **`lsp.*`**：自研桥接配置的语言服务器 [R9] | [rust-analyzer](https://github.com/rust-lang/rust-analyzer) 等语言服务器；[Serena](https://github.com/oraios/serena) | 定义/引用/诊断已有专业后端。Serena 可提供更多符号级导航和编辑，适合现有读接口之外的需求 | 首先改善已有服务器配置和可用性。Serena 是可选 MCP 扩展；避免重复起相同服务器。其 GPL-3.0-or-later 与独立服务/代码复用方式需分别考虑 | **P1–P2 / 中** |
| **`web.browser_*`**：Rust 自管 Chrome/CDP、JS 可见文本与控件快照 [R10]、[R11] | [Playwright CLI](https://github.com/microsoft/playwright-cli)、[Playwright MCP](https://github.com/microsoft/playwright-mcp) | 成熟定位器、自动等待、可访问性快照与浏览器自动化生态，可减少自行维护复杂交互的成本；这是功能/维护收益，未做延迟或成功率对照实验 | **优先做可选后端原型**。官方当前同时提供 CLI 与 MCP；Agena 已有按需工具发现，不能照搬“CLI 必然省更多 token”的宣传。保留 session 所有权、下载归属、效果声明和输出预算 | **P1 / 中–高** |
| 同一浏览器能力，偏向轻量命令接口 | [agent-browser](https://github.com/vercel-labs/agent-browser) | 官方当前实现是原生 Rust CLI，支持引用快照、JSON 和独立 session；适合作为 AI 的 CLI 候选 | 当前 README 明确 daemon 不要求 Node.js/Playwright，仅构建过程需要相应工具，不能沿用旧架构印象。仍需要 Chrome；与 Playwright 选择一个主后端做原型，避免双套状态 | **P1 / 中** |
| 浏览器诊断、网络与性能检查 | [Chrome DevTools MCP](https://github.com/ChromeDevTools/chrome-devtools-mcp) | 官方覆盖 trace、网络、控制台和调试；比继续往简单浏览工具里补诊断功能更值得评估 | 作为专门调试插件；不与普通抓取或主浏览器无条件同时启动。Node/Chrome 依赖和会话生命周期仍要管理 | **P2 / 中** |
| **`web.search`**：请求 Bing/DDG/Baidu 搜索 HTML 并自行解析 [R12] | [Brave Search API](https://api-dashboard.search.brave.com/documentation/services/web-search)、[Tavily](https://docs.tavily.com/documentation/api-reference/endpoint/search)、[Exa](https://docs.exa.ai/reference/search) | 有文档化的结构输出，降低跟随网页 DOM 改版维护解析器的成本；部分服务可同时返回正文。召回质量与延迟需按中文/英文任务集验证 | **建议可配置 provider**，保留本地 HTML 方案作无密钥选项。云端查询涉及 API 凭据、额度、费用和查询发送；不在本次作付费调用，也不宣称稳定性已验证 | **P1 / 中** |
| `web.search` 的自托管路线 | [SearXNG](https://github.com/searxng/searxng) | 将多引擎聚合和适配维护交给专门项目；可自托管 | 增加服务部署和维护；仍受上游限流、验证码影响，不等于拥有独立索引。AGPL-3.0-or-later；适合用户可选连接 | **P2 / 中–高** |
| **`web.fetch/crawl`**：`spider` + `crw-extract`，缓存/去重/索引 [R13]、[R14] | 继续用 [Spider](https://github.com/spider-rs/spider)；复杂场景选 [Crawl4AI](https://github.com/unclecode/crawl4ai) / [Firecrawl](https://github.com/firecrawl/firecrawl) | 当前已经是第三方 Rust 抓取栈。候选增加浏览器抓取、正文或结构抽取选项，但没有证据说明整体替换更快 | **保留默认本地后端**。Crawl4AI 增加 Python/浏览器环境；Firecrawl 增加自托管服务或云费用。Firecrawl 核心 AGPL-3.0，SDK/部分组件 MIT，云版还有额外能力 | **P2 / 中–高** |
| **HTML 正文抽取**：`crw-extract` readability + Markdown [R13] | [Trafilatura](https://github.com/adbar/trafilatura) | 可用另一套正文抽取策略改善特定站点的正文召回/噪声；官方有抽取评估资料 | 按文章、文档、论坛建立样例集后比较。多引入一个 Python 进程不自动带来性能收益；只在现有结果较差时回退 | **P2 / 中** |
| **本地 PDF/Office 文本摄取缺口**：`fs.read` 对二进制主要返回附件引用，云端理解另外提供 [R5]、[R6] | [MarkItDown](https://github.com/microsoft/markitdown)、[Docling](https://github.com/docling-project/docling)、[Poppler](https://poppler.freedesktop.org/)、[ripgrep-all](https://github.com/phiresky/ripgrep-all) | 新增本地文档提取/搜索。轻量文本 PDF 可用 `pdftotext`；通用格式转 Markdown 可用 MarkItDown；版面/表格/OCR 选 Docling；多格式检索选 rga | **补充能力，不替代视觉理解**。扫描件、阅读顺序、表格需检查；Docling 的模型/算力和 rga 的 pandoc/poppler/ffmpeg 等适配依赖都不是零成本；MarkItDown 的可选插件能力单独确认 | **P1–P2 / 中** |
| **`notebook.edit_cell`**：JSON 操作、格式检查、revision、输出清理 [R15] | [nbformat](https://github.com/jupyter/nbformat) | Jupyter 格式参考实现，可用于格式兼容性验证或复杂 notebook 处理 | 保留 revision 与原子发布；以 nbformat 验证样例比较合适。每次改单元格都启动 Python 不一定更快 | **P2 / 中** |
| **`memory.*`**：小端平台使用 Tantivy，另有可移植回退 [R16] | 继续用 [Tantivy](https://github.com/quickwit-oss/tantivy) | 已有成熟全文检索引擎。向量库解决的是不同的召回问题，不是无条件更优替代 | 没有语义检索任务集和失败证据前保留现状；如需向量召回，再比较混合检索的质量、存储和延迟 | **保留** |
| **`shell/monitor/cron`**：进程、PTY、日志、通知和会话唤醒 [R17] | [Watchexec](https://github.com/watchexec/watchexec) 处理文件变动触发；系统命令处理具体任务 | 文件监听可复用；具体命令的性能可以优化 | 保留 Agena 的进程句柄、所有权、取消、输出恢复和通知。`tmux`、`watch`、系统 cron 不能直接替代这些会话语义 | **保留框架，P2 扩展** |
| **`snapshot.*`**：Git / Rift 后端探测 [R18] | `git worktree` 等现有外部后端 | 已经采用第三方实现 | 保留托管层；不为“现代 CLI”另换版本控制系统 | **保留** |
| **计划/任务/设置/交互/报告/发现及云端包装** | 专业 CLI/MCP 可作为执行目标 | 内置工具承载产品状态；外部工具处理任务内容 | 保留 Agena 协议，在现有 MCP/插件边界接后端。云端文件、账号和计算容器生命周期与本地工具不等价 | **保留** |

### 搜索和浏览器值得注意的具体差异

`fs.grep` 当前不是调用 `/usr/bin/grep`。它的硬上限包括 500 条匹配、单文件 32 MiB、累计文件大小 256 MiB、25,000 个文件、100,000 个目录条目，搜索循环在文件之间检查 20 秒预算。后者不是能在任意时刻中断单个慢文件 I/O 的强制进程超时。直接改成 `rg` 子进程会改变排序、忽略路径、预算和错误语义，也会加入进程启动成本。

现有 `GrepToolInput` 只有 `pattern/path/include/include_ignored` [R19]。可增加 `fixed_strings`、显式大小写模式、前后文、仅文件名、计数、多 include/exclude、匹配数上限等参数。大小写等部分行为可通过正则内联标记表达，问题在于缺少清晰结构化入口，而非底层引擎绝对不支持。`rg -m` 是每文件限制，不能直接映射成 Agena 的全局 500 条预算。

浏览器快照使用 `document.querySelectorAll` 收集指定控件，并截取前 200 个；可见文本另有预算，当前代码还主动省略编辑区/敏感后代的文本 [R11]。这与完整可访问性树不是同一语义。当前等待逻辑按 100 ms 间隔检查 selector 是否存在、正文包含文本或 document readiness [R10]；[Playwright 的 actionability](https://playwright.dev/docs/actionability) 还覆盖可见、稳定、接收事件、可用等条件。后端替换时应同时保留现有敏感值处理和所有权规则，不能仅比较功能数量。

## 3. AI 在 shell 中的 CLI 选择表

这里的“替代”限定到任务场景，**不建议全局 alias 或自动改写用户提供的脚本**。`grep` 的 BRE、`find` 的表达式、`sed` 的流编辑语言都不与现代工具完全兼容。收益为功能/输出质量的行没有实测提速承诺。

| 任务 / 常见做法 | 推荐工具 | 更合适的原因 | AI 使用方式与边界 | 建议 |
| --- | --- | --- | --- | --- |
| 仓库正文搜索：`grep -R` | **[rg](https://github.com/BurntSushi/ripgrep)** | 本机同文件集合固定字符串搜索约 3.9–5.9 倍；支持 ignore、文件类型、JSON | 普通串用 `-F`；`-n --color=never`；程序解析用 `--json`。无匹配退出码 1 是正常结果 | **P0 默认** |
| 按文件名/扩展名查找：`find` | **[fd](https://github.com/sharkdp/fd)** / `rg --files` | fd 表达式短、并行遍历；本机文件发现约 1.36 倍 | `fd --type f --extension rs . crates`；glob 用 `--glob`，全路径匹配用 `--full-path`。`.gitignore`/隐藏文件默认语义需明确 | **P0 默认** |
| 仅查 Git 跟踪文件 | **`git ls-files` / `git grep`** | 利用 Git 已知文件集；明确“跟踪文件”语义 | 未跟踪文件不会自动包含；不能作为全工作区搜索的无提示替换 | **P0 按范围选** |
| 用正则定位/修改代码结构 | **[ast-grep](https://github.com/ast-grep/ast-grep)** | AST 模式跨空白/格式变化，rewrite 支持结构改写 | 普通文本仍用 rg；结构搜索用 `ast-grep run`；输出可选 JSON。使用全名避免 `sg` 同名程序冲突 | **P0 路由；CLI 按需装** |
| 用 grep/sed 解析 JSON | **[jq](https://github.com/jqlang/jq)** | 正确处理嵌套、转义、数组，避免 JSON 被当作行文本 | `-c` 减少格式开销、`-r` 提取字符串；选择必要字段。`-e` 的非零状态可能是 false/null 语义 | **P0 默认** |
| 用 sed 改 YAML/XML 等结构文件 | **[yq](https://github.com/mikefarah/yq)** | 按字段更新比行替换可靠 | 指定 **Mike Farah yq v4**，不要与另一同名 Python 工具混淆。`-i` 会写文件；注释/空白不能保证字节级保留 | **P1 按需** |
| 用 `cut`、`awk -F,` 处理真实 CSV | **[xan](https://github.com/medialab/xan)** / [qsv](https://github.com/dathere/qsv) | 正确理解带引号逗号/换行，提供选择、过滤、统计和 join | 挑一个常用实现即可；编码、分隔符、null/空串和类型推断需确定。qsv 的扩展功能视构建版本而定 | **P1 按需** |
| 大 CSV/Parquet 临时分析、多个文件 join | **[DuckDB](https://github.com/duckdb/duckdb)** | 直接用 SQL 聚合/连接列式与表格文件，减少手写数据脚本 | 明确列类型与行数范围；适合分析任务，不是普通日志 grep 的替代 | **P1 按需** |
| 多文件固定串/正则替换：复杂 `sed -i` | **[sd](https://github.com/chmln/sd)** | 字面/正则替换语法简单，提供预览 | 文件参数通常意味着就地写；先预览并生成可审查 diff。复杂 sed 流处理仍保留 sed；Agena 修订检查仍保留 | **P1 可选** |
| Python 环境/依赖：`pip + venv + pipx` | **[uv](https://github.com/astral-sh/uv)** | 统一环境、锁文件、工具运行和缓存；上游有性能基准 | 遵循项目已有 lockfile/工具链；安装或 `uv run` 可能下载与同步环境。未测本仓库 Python 安装倍率 | **P1；现有环境可优先** |
| 临时 `curl` 拼 API JSON 请求 | **[xh](https://github.com/ducaale/xh)** / [HTTPie](https://github.com/httpie/cli) | 请求参数更易写，适合交互式 API 调试；xh 关注低开销 | 不宣称网络请求更快。简单下载、代理、证书和复杂传输仍保留 curl；机器输出关闭装饰 | **P2 可选** |
| 手写 GitHub REST curl / 网页抓取 | **[gh](https://github.com/cli/cli)** | 对 PR、issue、checks 等提供明确命令和结构输出 | 选择 `--json` 字段；仍需已有认证和对应操作授权；GitLab 等平台使用自己的工具 | **P1 按平台** |
| `cat` 阅读代码、手工加行号 | **[bat](https://github.com/sharkdp/bat)** | 行范围、行号和人类终端高亮更便利 | AI 使用 `--color=never --paging=never`，按需 `--style=numbers`。原始小文件用 cat 或内置读通常更直接；没有速度优势结论 | **P2 人类界面优先** |
| `ls` / `tree` 浏览目录 | **[eza](https://github.com/eza-community/eza)** | 树和 Git 状态等便于人读 | AI 搜索路径首选 fd/rg；eza 的图标、颜色、元数据会增加输出。项目采用 EUPL-1.2，分发时单独核对 | **P2 展示用途** |
| 看 Git diff | **[delta](https://github.com/dandavison/delta)** | 更好的终端差异展示 | 主要给人看；AI 和补丁应用默认 `git --no-pager diff --no-ext-diff --color=never` | **P2 展示用途** |
| 格式变化掩盖语义变化的 diff | **[Difftastic](https://github.com/Wilfred/difftastic)** | 通过语法结构展示差异，可帮助复杂评审 | 官方明确不生成可应用补丁；语法解析有成本，不能替代 patch 格式 | **P2 评审用途** |
| `wc -l` 统计代码量 | **[tokei](https://github.com/XAMPPRocky/tokei)** / [scc](https://github.com/boyter/scc) | 区分语言、代码、注释和空行，语义优于简单行数 | 使用机器输出；明确是否包含 vendor、生成代码。scc 的复杂度数字不直接等于质量或人力 | **P2 按需** |
| `du` 逐层定位大目录 | **[dust](https://github.com/bootandy/dust)** | 更直观找到大文件/目录，支持 JSON | AI 可用 `dust -j`；遍历本身仍需 I/O，不声称所有文件系统上更快 | **P2 按需** |
| `df -h` 读磁盘信息 | **[duf](https://github.com/muesli/duf)** | 过滤和 JSON 输出更易处理 | `duf --json`；简单单点检查保留系统 df，避免新增依赖无收益 | **P2 按需** |
| `ps aux` 人工筛进程 | **[procs](https://github.com/dalance/procs)** | 更丰富搜索、列和树视图 | 当前官方 README 将 macOS 支持标为实验性；进程身份和生命周期仍由 Agena 管理；机器解析可继续用系统 ps 的明确列 | **P2，不设跨平台默认** |
| `watch` / while+sleep 等待源码变化 | **[Watchexec](https://github.com/watchexec/watchexec)** | 文件事件触发、过滤与重启命令，减少定时重跑 | 适合“文件改变后执行”；并非替代所有周期监控。进程仍放在 Agena 后台作业管理里 | **P1 按需** |
| 手工拼长构建/验证命令 | **[just](https://github.com/casey/just)** | 仓库内具名 recipe 降低 AI 记错参数的概率 | 它是命令运行器，不是增量构建系统；已有 Make/Cargo/Bun 流程先遵循，无需仅为统一而迁移 | **P2 按仓库** |
| 单次 `time` 判断快慢 | **[Hyperfine](https://github.com/sharkdp/hyperfine)** | 多轮、预热、统计、JSON 导出，使性能主张可检查 | 先验证输出等价、范围一致；冷热缓存、进程启动与系统负载分别说明 | **P1 开发工具** |
| `grep` 搜 PDF/Office/压缩包 | **[ripgrep-all](https://github.com/phiresky/ripgrep-all)** (`rga`) | 经适配器转换后搜索更多格式，并可缓存 | 只在多格式资料目录启用；并非更快的源码 rg；额外转换依赖和初次处理成本需计入 | **P2 文档场景** |
| 原样把构建/测试/git 大输出喂给 AI | **[RTK](https://github.com/rtk-ai/rtk)** 或借鉴其定向输出过滤 | 过滤日志可减少模型输入；RTK 官方给出压缩宣称，但其 token 计数采用 bytes/4 估计 | **先做实验，保留原始输出和退出码**；确认失败细节、警告不被删。不能把“输出少 90%”换算成“费用少 90%”，本次未测 token 或任务成功率 | **P1 试验，不做透明默认代理** |
| 交互式找文件/目录：反复 `cd`、列表选取 | **[fzf](https://github.com/junegunn/fzf)** / [zoxide](https://github.com/ajeetdsouza/zoxide) | 对人类交互和历史目录选择友好 | AI 默认使用明确 cwd 和确定路径；fzf 仅在明确需要选择时使用，或采用非交互 filter；zoxide 历史状态不宜成为任务隐式依赖 | **不作为 AI 默认替换** |

本机审计 shell 已发现 `rg 15.2.0`、`fd 10.5.0`、`jq 1.8.2`、`uv 0.12.6`、`bat`、`eza`、`delta`。`ast-grep` CLI、`yq`、`sd`、`hyperfine`、`duckdb` 等未出现在这次 shell 的 PATH 中。未找到 ast-grep CLI 不影响源码已经内嵌其库。以上只说明调查进程的环境，**不能推定后台 Agena 服务继承了相同 PATH**。

## 4. 本机可复现 CLI 基准

环境：macOS 26.6.2 / arm64，10 个逻辑 CPU；macOS 自带 BSD grep 和 find，对比 Homebrew 的 rg/fd。文件范围是同一基线的 **1,220 个 `crates/**/*.rs` 文件，共 18,346,497 bytes（18.35 MB）**。不包含 `third_party`。

文本搜索给每个命令传入相同的显式文件列表，固定字符串模式、行号/路径、文本处理和 `LC_ALL=C` 一致。文件发现对 fd 关闭隐藏/ignore 过滤，和 find 使用相同范围。每组先比较完整结果的排序后内容，再预热 2 轮、以固定随机顺序测量 21 轮；计时包含进程启动，stdout 写入 `/dev/null`。没有清 OS 缓存或隔离其他系统负载。

| 工作负载 | 结果记录数 | 系统命令中位数 | 推荐命令中位数 | 中位数比值 | 说明 |
| --- | ---: | ---: | ---: | ---: | --- |
| 固定串 `MAX_SEARCHED_FILES` | 2 | grep 56.85 ms | rg 14.64 ms | **3.88×** | 稀疏命中，结果内容一致 |
| 固定串 `ToolInvokeOutput` | 396 | grep 86.24 ms | rg 17.22 ms | **5.01×** | 较多输出，48,338 bytes，一致 |
| 不存在的固定串 | 0 | grep 113.71 ms | rg 19.33 ms | **5.88×** | 两者退出码均为 1，正常无匹配 |
| 发现全部 Rust 文件 | 1,220 | find 20.94 ms | fd 15.42 ms | **1.36×** | 同文件集合，无忽略规则差异 |

另记录了 rg 单线程中位数 18.44 / 28.01 / 31.92 ms，均慢于这次 rg 默认配置。它支持“并行配置值得评估”的方向，但不是内置搜索并行化收益的估计。

可复现文件：

- [基准脚本](benchmark-tool-alternatives.py)：Python 标准库，不安装依赖，直接 argv 调用，不经 shell 重写。
- [完整结果](tool-alternatives-benchmark.json)：保留 21 次样本、版本、命令、corpus hash、输出 hash、退出码及限制说明。
- [官方资料登记](tool-alternatives-sources.json)：51 份项目 README / 官方文档的 URL、字节数和抓取内容 SHA-256。URL 可能随上游变化；hash 标识本次核对的内容。

```sh
python3 docs/research/benchmark-tool-alternatives.py --output /tmp/agena-tool-benchmark.json
```

**这些倍率只描述这台机器上的这组 warm-cache CLI 工作负载。** 不覆盖 GNU grep、Windows、大型仓库、网络文件系统、PCRE2、复杂正则、冷启动缓存；不覆盖 Agena 内置 `fs.grep/fs.glob` 的运行时间、token 消耗或任务成功率。系统负载会改变绝对数值和倍率，不应据此承诺产品端到端提升。

## 5. Agena 的落地顺序

1. **P0：让 AI 知道可用工具及选择规则。** 当前基础身份 prompt 和动态 session prompt 侧重 Tool API 与工作流，没有明确的 `grep → rg`、JSON → jq 选择策略 [R20]、[R21]。在 Agena 实际运行环境中探测允许的候选可执行文件、路径、版本和可用特性，按需提供精简 capability 信息；进程 PATH/配置变化时刷新，不在每次 tool call 全量跑 `--version`。已知名字和已嵌入库的能力不需要反复 discovery。
2. **P0：按任务输出最少的充分信息。** 路径发现用 glob/fd/`rg --files`，纯文本用内置 grep/rg，语法模式用 ast-grep，符号关系用 LSP，JSON 用 jq；默认禁 pager/颜色，限定路径/字段/行范围。仅凭“输出更短”不能判定更好，错误与 truncation 标志必须保留。
3. **P0：同步命令效果识别。** 当前两个 shell 分类入口已经认识 `rg`，但列出的只读命令中没有 `fd/jq/ast-grep` [R22]、[R23]。接入时需按子命令/参数判断，而不是把命令名整体加入只读白名单：`fd -x/-X` 可执行命令、`rg --pre` 可执行预处理器、ast-grep rewrite、`yq -i`、`sd` 可写文件；jq 的文件/模块参数也影响读取范围。外部命令效果仍使用既有声明与校验流程。
4. **P1：先补搜索 API，再决定后端。** 为 `fs.grep` 增加匹配模式、输出模式和上下文；测真实仓库的扫描时间、返回字节、扫描是否完整。之后比较现有串行库、并行库和可选 rg 进程，使用相同 ignore/隐藏/大小预算。分页要求稳定顺序时，明确排序成本和一致性策略。
5. **P1：浏览器与搜索服务各做一个原型。** 浏览器比较 Playwright CLI/MCP 或 agent-browser 中选定的候选；使用真实的表单、SPA、遮挡、iframe、下载和 session 隔离样例。搜索 API 用固定中文/英文查询集合比较链接有效性、召回、正文质量、延迟、费用；不以官网宣传作排名。
6. **P1–P2：按真实任务补选装能力。** 本地文档摄取、DuckDB/CSV、语义改写和日志压缩分别提供小范围入口，避免一次性给 AI 增加几十个重叠工具。RTK 类压缩实验同时测“实际模型 tokenizer 的输入量、错误保留率、恢复原始输出次数、任务成功率”。

建议的依赖层次：核心保持现有内嵌 ripgrep 库、ast-grep、Spider、Tantivy 和 Agena 状态协议；宿主推荐发现 `rg/fd/jq`；`uv/ast-grep CLI/yq/DuckDB` 按工作负载补充；浏览器、文档转换及外部搜索以可选 provider/MCP/插件接入。它们的后台进程、文件发布和网络调用仍走 Agena 的现有执行边界。

可在后续实现中使用的简短选择提示示例：

> 先确定任务需要的是路径、文本、代码结构还是符号关系。优先使用已可用的专用工具；使用 shell 时，正文搜索优先 rg，文件发现优先 fd 或 rg --files，JSON 解析优先 jq。只有确认依赖可用时才选择它。关闭分页和颜色，限定输出；无匹配与执行失败分别处理。遵循用户给定命令和仓库既有脚本，不做全局 alias 替换。

## 6. 源码证据索引

以下路径以调查时的 worktree 为准，链接行号对应基线提交。第三方能力的官方链接已放在对应表格中；本次没有验证候选服务的生产 SLA、付费额度或跨平台性能。

| 引用 | 文件 / 主要位置 | 支持的结论 |
| --- | --- | --- |
| R1 | [生成工具参考][R1] | 22 个插件、135 个定义及全部输入/帮助 |
| R2 | [grep.rs][R2] | ripgrep 核心库、串行扫描、全局/文件预算 |
| R3 | [discovery.rs][R3] | ignore 策略、禁止跟随符号链接、路径排序 |
| R4 | [glob.rs][R4] | 分页、超时/条目限制、目录路径匹配 |
| R5 | [read.rs][R5] | 流式分页、目录和附件处理 |
| R6 | [fs.rs][R6] | 批量读、写入修订/出现次数校验 |
| R7 | [apply_patch.rs][R7] | 自定义 patch、锁、原子文件发布、失败回滚 |
| R8 | [code_search.rs][R8] | ast-grep / Tree-sitter；逐文件 AST 搜索 |
| R9 | [lsp.rs][R9] | 可配置语言服务器及导航/诊断桥接 |
| R10 | [web/plugin.rs][R10] | 自建 CDP、浏览器操作和等待逻辑 |
| R11 | [snapshot.js][R11] | DOM 控件/可见文本快照、上限及敏感值处理 |
| R12 | [search.rs][R12] | Bing/DDG/Baidu HTML 获取和解析 |
| R13 | [extract.rs][R13] | crw-extract readability/Markdown |
| R14 | [spider.rs][R14] | Spider 抓取/浏览器渲染 |
| R15 | [notebook.rs][R15] | 单元格编辑和 SHA-256 修订检查 |
| R16 | [memory-index/lib.rs][R16] | Tantivy 和平台回退 |
| R17 | [shell.rs][R17] | 后台进程、PTY、效果声明及通知契约 |
| R18 | [snapshot_capabilities.rs][R18] | Git/Rift 已有后端探测 |
| R19 | [part/tool.rs][R19] | GrepToolInput 的当前四个字段 |
| R20 | [identity/mod.rs][R20] | 基础 prompt 的工具发现/使用规则 |
| R21 | [session_prompt.rs][R21] | 动态 prompt 的规划/提问/委派/后台规则 |
| R22 | [shell_analysis.rs][R22] | shell 命令效果分类 |
| R23 | [workflow_runtime.rs][R23] | 工作流侧只读 shell 判断 |

[R1]: ../../crates/agena-bundled-plugins/generated/tools-reference.md#L9
[R2]: ../../crates/agena-runtime-tools/src/tool/grep.rs#L6
[R3]: ../../crates/agena-runtime-tools/src/tool/discovery.rs#L20
[R4]: ../../crates/agena-runtime-tools/src/tool/glob.rs#L1
[R5]: ../../crates/agena-runtime-tools/src/tool/read.rs#L20
[R6]: ../../crates/agena-bundled-plugins/src/plugins/provided/fs.rs#L104
[R7]: ../../crates/agena-runtime-tools/src/tool/apply_patch.rs#L1
[R8]: ../../crates/agena-tool/src/code_search.rs#L250
[R9]: ../../crates/agena-bundled-plugins/src/plugins/provided/lsp.rs#L1
[R10]: ../../crates/agena-bundled-plugins/src/web/plugin.rs#L2334
[R11]: ../../crates/agena-bundled-plugins/src/web/browser/snapshot.js#L8
[R12]: ../../crates/agena-web/src/search.rs#L22
[R13]: ../../crates/agena-web/src/extract.rs#L3
[R14]: ../../crates/agena-web/src/spider.rs#L7
[R15]: ../../crates/agena-bundled-plugins/src/plugins/provided/notebook.rs#L1
[R16]: ../../crates/agena-memory-index/src/lib.rs#L1
[R17]: ../../crates/agena-bundled-plugins/src/plugins/provided/shell.rs#L92
[R18]: https://github.com/canxin121/agena/blob/0f8c65b5b1439022f721227941ebf908aeaf40dc/crates/agena-runtime-tools/src/snapshot_capabilities.rs#L10
[R19]: ../../crates/agena-runtime-contracts/src/part/tool.rs#L252
[R20]: ../../crates/agena-runtime-contracts/src/identity/mod.rs#L39
[R21]: ../../crates/agena-runtime-session/src/session/manager/session_prompt.rs#L1
[R22]: ../../crates/agena-tool/src/shell_analysis.rs#L737
[R23]: ../../crates/agena-bundled-plugins/src/plugins/provided/workflow/workflow_runtime.rs#L63
