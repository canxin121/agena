这次改进将 Agena 的网页读取串成了一条完整路径：**搜索发现 URL → 并发读取 → 检查页面状态 → 长文续读 → 有预算地爬取站点 → 检索已经抓取的证据**。默认正文提取改为显式内容区域优先、`dom_smoothie` 后备、`htmd` 转 Markdown；浏览器按页面需要启用。没有把完整爬虫库或多个浏览器驱动直接叠加进来。

调研日期为 **2026-10-05，Asia/Singapore**，Rust 工具链为 **1.97.0**，实施基线为 `a4e3425c`。此前的八来源聚合搜索已经在该提交推送到远端 `master`，包含当时主分支新增修改。本次在 `feat/web-content-pipeline` 独立 worktree 开发。调查、实验和验证使用普通 shell、Git、GitHub/crates.io HTTP、独立 Rust 程序、库测试和本地 Chrome 测试，**没有调用 Agena tools 或启动 Agena**。

**调查范围和证据强度。** 复用了上一轮 76 个 crate、37 个 GitHub 仓库、32 份 README、13 轮 GitHub 搜索、21 轮 crates.io 搜索和 11 份校验过 registry SHA-256 的发布包；这轮又做了 6 轮 crates.io 查询，成功读取或刷新 35 个 crate、9 个核心仓库及其 README。去重后总共覆盖 **84 个 crate**。覆盖量包含上一轮搜索生态资料，不意味着 84 个库都进行了抓取测试。[来源快照](rust-crawling-sources.json) 保留每条记录的时间、URL、版本、许可证、声明的 MSRV、响应哈希、失败查询和上游问题来源。

本轮实际运行比较为 **14 个样本 × 5 条对照提取链路**，再用新生产实现运行同样 14 个输入。四个公开页面复用了已保存的 HTML；其余十个是人工诊断样本。版本元数据、README 声明、源码检查、上游问题报告、实际运行结果分别说明，不能相互替代。Spider 的大元数据响应超时，随后分页版本接口返回 403；下文 `2.53.9` 来自此前成功查询，生产依赖仍是 `2.53.6`，没有伪装成本轮重新确认。

原实现的问题及本次处理如下。

| 位置 | 发现的问题 | 本次行为 |
| --- | --- | --- |
| 正文选择 | `crw-extract` 的密度/选择器策略会丢掉同级 article 回答；换 Markdown 转换器无法找回被删内容 | 先保留 main/role=main；单篇及多篇 article 保留全部不重叠区域；普通无结构页面再用 dom_smoothie |
| 技术资料 | 激进正文清理可能丢函数约束、代码、表格或短中文回答 | 回归覆盖代码缩进、围栏、表格、中文、MathML TeX、details；保留 Markdown 有意义的空白 |
| URL | canonical 提示与真实页面地址职责不同，正文链接也需要解析 | 元数据保留 requested/final/canonical；相对正文链接按实际 URL/base 解析并保留 fragment；爬取候选去掉 fragment 去重 |
| 类型/状态 | text/plain 中的 HTML 示例可能被当成 HTML；HTTP 200 空壳/验证页容易混进正常内容 | 尊重明确 MIME，支持 XHTML；返回 readable、empty、requires_javascript、blocked、too_complex |
| 批量读取 | AI 只能一页一页调用或自行组织并发 | 新增 web.fetch_many，1–8 个 URL，默认并发 4，失败分别返回，规范化重复 URL 只请求一次 |
| 站点爬取 | BFS 队列逐页 await | 同一深度按有界批次并发，默认 4、最大 8；按发现顺序处理，先预留预算再发请求 |
| 长页面 | 文本仅约 4,000 字符预览，JSON 16,000 字符，无法读后续 | fetch 返回 page_id/next_offset，web.read 连续读取不可变快照；字符切片可还原全文，不重新请求网络 |
| 爬取后取证 | 建了本地索引但没有模型可用的查询入口 | 新增 web.query，返回 chunk 预览、抓取时间、page_id 和 read_offset，与 web.read 接通 |
| 渲染 | browser.enabled 会令默认请求全部进浏览器 | 默认 HTTP；有明确 JS 壳页信号且启用浏览器时，只重试一次；显式 true/false 控制渲染 |
| 主动刷新 | 同 URL 的相似/相同正文也可能被重复检测拦截，留下旧时间 | 同一文档刷新优先更新正文和抓取时间；显式 HTTP 模式不复用渲染文档 |
| 失败和缓存 | 错误大多只在日志里；200 验证页可能缓存 | 批量读取保留逐项公开错误类型；crawl 返回 page_errors；只有完整、可读 2xx 页面进入可复用缓存/存储 |
| 模型使用说明 | 缺少并发、状态检查和续读流程 | 系统提示词按当前工具可用性添加 web 工作流，并同步工具 schema/help 和生成文档 |

**正文提取与转换库。** 版本取自来源快照；“未声明”指 crate 元数据未声明 MSRV，不代表已经验证任何 Rust 版本。表中的“已测”仅指上述固定样本。

| 库 / 版本 | 许可证 / 声明 MSRV | 可提供的能力 | 判断与采用方式 |
| --- | --- | --- | --- |
| [dom_smoothie 0.18.2](https://github.com/niklak/dom_smoothie) | MIT / 1.75 | Mozilla Readability 风格正文选择、元数据、解析配置 | **采用，作为非结构化文章后备**。比旧链路更能保留多篇回答，但默认算法仍丢短中文问答；不能单独无条件替换 |
| [dom_query 0.28.0](https://crates.io/crates/dom_query) | MIT / 1.75 | 可修改 DOM、CSS 选择、节点遍历 | **采用**，用于完整保留语义区域、解析链接、去显式噪声和转换数学注释，与 dom_smoothie 共用生态 |
| [htmd 0.5.5](https://github.com/letmutex/htmd) | Apache-2.0 / 未声明 | HTML → Markdown、代码/表格/链接转换 | **保留并改为直接依赖**。旧实现已间接使用它，本次收益主要来自正确选择内容 |
| [html-to-markdown-rs 3.17.0](https://github.com/xberg-io/html-to-markdown) | MIT / 1.88 | 多种 Markdown/结构转换配置 | 已测。与默认 dom_smoothie 组合也会丢中文回答；无法修复上游正文选择。当前固定样本未显示值得额外引入的必要收益 |
| [rs-trafilatura 0.2.2](https://github.com/Murrough-Foley/rs-trafilatura) | MIT OR Apache-2.0 / 1.85 | 正文、独立评论、元数据、Markdown、分类 | 已测；评论字段必须显式消费。技术结构、中文问答、嵌套表格有标记遗漏；暂不作为唯一默认 |
| [trafilatura 0.3.0](https://github.com/nchapman/trafilatura-rs) | Apache-2.0 / 1.85 | Rust 移植的文章/评论提取和 Markdown | 已测；部分文档函数约束、代码/表格和链接标记遗漏。不能因名称相同就视为 Python Trafilatura 等价实现 |
| [readability 0.3.0](https://crates.io/crates/readability) | MIT / 未声明 | 较早的 Readability 实现 | 2023 年发布版，生态覆盖有价值；本次没有对其单独运行，不作为优先替换方案 |
| [readabilityrs 0.1.4](https://github.com/theiskaa/readabilityrs) | Apache-2.0 / 1.83 | 正文与 Markdown 一体化 | 源码/文档候选；README 描述相对 URL 和部分配置的限制，未实测 |
| [readex 0.19.2](https://github.com/0x4D44/readex) | MIT OR Apache-2.0 / 1.85 | 多算法正文与日期提取 | 调查候选，未运行同一组样本；需要单独比较各策略的输出，不能凭组合算法数量推荐 |
| [libreadability 0.2.0](https://crates.io/crates/libreadability) | MIT / 1.83 | go-readability 的 Rust 移植 | 元数据层发现，未实测；适合作未来对照 |
| [kawat 0.1.5](https://crates.io/crates/kawat) | Apache-2.0 / 1.85 | Trafilatura 思路的正文、元数据、评论 | 元数据层发现，记录未提供 repository；来源可审查性需要补齐 |
| [justext 0.2.0](https://crates.io/crates/justext) | BSD-2-Clause / 1.83 | 段落级 boilerplate 清理 | 可作为新闻段落清理候选，保留代码/表格能力未验证 |
| [readability-rust 0.1.0](https://crates.io/crates/readability-rust) | Apache-2.0 / 未声明 | 另一同名 Readability 实现 | 仅元数据核查，避免与 readability/readabilityrs 混淆 |
| [html-cleaning 0.3.0](https://crates.io/crates/html-cleaning) | MIT OR Apache-2.0 / 1.75 | HTML 清理组件 | 可作为清理层对照，不能代替正文选择与证据完整性验证 |
| [web2md 0.1.13](https://crates.io/crates/web2md) | Apache-2.0 / 未声明 | 本地无 key 网页转 Markdown 组合 | 元数据层发现；须核查是否能注入宿主 HTTP transport，不能直接默认接受其内部下载路径 |
| [fast_html2md 0.0.63](https://github.com/spider-rs/html2md) | MIT / 未声明 | Spider 生态 Markdown 转换 | 可替代转换层，未在本轮运行；名字不同于 GPL 的 html2md |
| [html2text 0.17.1](https://github.com/jugglerchris/rust-html2text) | MIT / 1.85 | HTML 转可读文本 | 适合纯文本和检索；技术资料仍需要保留代码、链接、表格结构 |
| [deformat 0.15.3](https://crates.io/crates/deformat) | MIT OR Apache-2.0 / 1.91.0 | HTML、PDF 等格式转文本 | 扩展多格式读取的候选；本次没有把 PDF/OCR 能力并入网页工具 |
| [firecrawl/html-extractor](https://github.com/firecrawl/html-extractor) | Apache-2.0；仓库声明 1.78 | 页面分类、正文评分和 HTML 提取 | 值得后续试验；仓库 workspace 版本 0.1.0。**crates.io 同名 html-extractor 1.0.0 是另一个项目**，未确认匹配的 registry 发布 |
| [rust-trafilatura 2.2.8](https://crates.io/crates/rust-trafilatura)、[rust-readability-v2 0.6.7](https://crates.io/crates/rust-readability-v2)、[rust-domdistiller 1.0.3](https://crates.io/crates/rust-domdistiller) | 组合许可证见来源 JSON / 均声明 1.98.1 | 新的提取移植候选 | 超过当前 1.97.0 工具链，未进入运行比较；没有为引入它们提高项目 MSRV |
| [article_scraper 2.3.1](https://gitlab.com/news-flash/article_scraper)、[html2md 0.2.17](https://gitlab.com/Kanedias/html2md) | GPL-3.0-or-later / GPL-3.0+ | 新闻正文 / Markdown 转换 | 许可证和集成条件与主要候选不同，本次不新增这两项依赖 |

Python Trafilatura 的可选本地 adapter 保留：由调用方明确选择 `extractor: "trafilatura"`，需要已安装的 Python 包；不自动安装。它与表中两个 Rust 项目是独立实现。默认 `extractor: "readability"` 保持兼容名称，实际选择策略由 `extraction_strategy` 明确报告。

**爬虫调度、网络客户端与浏览器。** 网页是否可获取、是否要执行 JavaScript、正文是否选全，是三个不同问题。换一个统一框架不能自动修复全部问题。

| 库 / 版本 | 许可证 / 声明 MSRV | 能帮助的环节 | 判断与接入代价 |
| --- | --- | --- | --- |
| [crawlberg 1.9.0](https://github.com/xberg-io/crawlberg) | MIT / 1.91 | Frontier、BFS/DFS/best-first、速率/缓存、sitemap、batch/stream、HTTP→browser | 功能覆盖最广的候选之一。已读发布源码和 README；具备 Frontier/RateLimiter/CrawlStore 等 trait，但所检查 builder 没有通用宿主 HTTP transport 注入。直接迁移会涉及当前许可、DNS、重定向和取消边界，暂不整套替换 |
| [spider 2.53.9](https://github.com/spider-rs/spider) | MIT / 未声明 | 并发爬取、浏览器 feature、smart crawling | 生产已锁 2.53.6。已有依赖不代表旧 Agena crawl 在用 Spider 并发 frontier；这轮直接修复自己负责授权的调度层，借鉴 HTTP 优先路径，不同时更换整套 transport |
| [crw-crawl 0.37.2](https://crates.io/crates/crw-crawl) | AGPL-3.0 / 未声明 | 完整爬虫框架 | 与旧 crw-extract 是不同 crate；元数据审查后保留对照，不默认引入新框架 |
| [reqwest 0.13.5](https://crates.io/crates/reqwest) | MIT OR Apache-2.0 / 1.85.0 | 异步 HTTP、流式响应、TLS 等 | 继续沿用现有受控 transport，不为版本号而升级；生产使用仓库已有补丁和锁定版本 |
| [wreq 0.16.1](https://github.com/0x676e67/wreq) | Apache-2.0 / **1.98** | 浏览器风格 HTTP/TLS 指纹 | 当前工具链不兼容最新声明版本；还需考虑 BoringSSL 构建成本。指纹调整不等于保证通过验证码、IP 限制或登录 |
| [chromey 2.58.2](https://github.com/spider-rs/chromey) | MIT OR Apache-2.0 / 1.70 | 异步类型化 CDP | 已有间接依赖 2.54.0，可评估替换协议绑定；会话归属、宿主授权、取消清理仍由产品负责 |
| [chromiumoxide 0.9.1](https://github.com/mattsse/chromiumoxide) | MIT OR Apache-2.0 / 1.85 | 异步 Chrome DevTools 控制 | Crawlberg 的 browser 路线之一；与 chromey 功能重叠，本次不新增第二套大协议栈 |
| [headless_chrome 1.0.22](https://github.com/rust-headless-chrome/rust-headless-chrome) | MIT / 1.85 | Chrome 控制、截图、页面操作 | 可作对照，需要处理同步调用/线程模型和会话迁移；没有本轮运行证据支持重换驱动 |
| [playwright-rs 0.19.0](https://github.com/padamson/playwright-rust) | Apache-2.0 / 1.88 | 高层定位器、自动等待、frame、CDP 连接 | 很值得做现有 Python bridge 的替代原型；**仍需要 Playwright Node driver**，文档描述驱动下载/缓存，不能称为无外部运行时 |
| [thirtyfour 0.37.5](https://github.com/stevepryde/thirtyfour) | MIT OR Apache-2.0 / 1.88 | Selenium/WebDriver 自动化 | 适合跨浏览器自动化；还要运行和管理 WebDriver 服务 |
| [fantoccini 0.22.1](https://github.com/jonhoo/fantoccini) | MIT OR Apache-2.0 / 1.67.0 | 异步 WebDriver 客户端 | 同样依赖独立 WebDriver，不能直接等价替换现有受控 CDP context |
| [browser_oxide 0.1.3](https://crates.io/crates/browser_oxide) | MIT OR Apache-2.0 / 1.91 | 项目描述声称自有 HTML/CSS/JS/V8 和 TLS 路径 | 仅元数据发现；“stealth/无需 Chromium”是项目声明，网页兼容、权限接口与资源清理尚未验证 |
| [headless-engine 1.1.2](https://crates.io/crates/headless-engine) | MIT OR Apache-2.0 / 未声明 | 项目描述声称轻量 Rust 浏览器 | “低于 30 MB”不是本次测量；需用真实 SPA、frame、网络事件、取消测试单独验证 |
| [oxibrowser 0.25.0](https://crates.io/crates/oxibrowser) | MIT / 1.96 | 项目描述为支持 CDP 的 headless engine | 仅元数据核查，尚未验证对现有 CDP 命令和浏览器行为的兼容程度 |
| [viewpoint-core 0.4.3](https://crates.io/crates/viewpoint-core) | MIT / 1.85 | 高层浏览器自动化 API | 仅元数据核查，定位器、等待和后端生命周期需要实际比较 |
| [agent-browser](https://github.com/vercel-labs/agent-browser) | CLI/daemon 形态 | 面向 agent 的 AX snapshot、元素引用和交互 | 可以作为受控 sidecar 研究；不是无成本嵌入库，还需要进程、版本、权限、会话管理 |

浏览器能力已有较强的产品约束：每次页面读取使用独立 context；导航、重定向和子资源通过宿主策略；禁用下载、绕过 service worker；有 DOM/响应预算、整体超时和取消清理。本次复用这些能力。Chromium 自行解析 DNS，现有策略**不是连接级 DNS pinning，也不是对所有浏览器协议的完整网络沙箱**；这是沿用实现的技术边界。HTTP transport 的已批准地址固定与浏览器路径不能混为一谈。

还有一些可组合的基础库，适合按明确缺口引入。

| 库 | 适用用途 | 本次决定 |
| --- | --- | --- |
| [scraper 0.27.0](https://crates.io/crates/scraper)，ISC | CSS 选择器和静态 HTML 查询 | 搜索解析仍可使用；不是正文评分算法。不要将其许可证写成 MIT |
| [lol_html 3.0.1](https://crates.io/crates/lol_html)，BSD-3-Clause | 流式 HTML 重写/前置过滤 | 大 HTML 的预处理候选，不能代替 Readability；本次没有新引入到正文链路 |
| [quick-xml 0.42.0](https://crates.io/crates/quick-xml)，MIT | XML 流式解析、sitemap 支持基础 | 可用于未来站点发现；需要 gzip、索引递归和 URL 数量预算 |
| [sitemap 0.4.1](https://crates.io/crates/sitemap)、[sitemap-rs 0.7.0](https://crates.io/crates/sitemap-rs)，MIT | sitemap 读写/解析 | 值得后续扩展 deep-page discovery，本次没有宣称已经实现 sitemap/map 工具 |
| [feed-rs 3.0.0](https://crates.io/crates/feed-rs)，MIT | RSS/Atom/JSON Feed | 可补充新闻发现，不是通用正文爬取替代 |
| [robotstxt 0.3.0](https://crates.io/crates/robotstxt)，Apache-2.0 | robots 规则匹配 | 继续沿用既有匹配和传输验证 |
| [governor 0.10.4](https://crates.io/crates/governor)，MIT | 速率限制 | 继续在跨页、robots 和重定向请求中共享每主机节奏 |
| [Tantivy 0.26.2](https://crates.io/crates/tantivy)，MIT | 本地全文索引 | 复用现有锁定/补丁版本；新增 web.query 使 AI 能实际使用已有索引 |

上游问题是风险线索，不是本次复现结论：[dom_smoothie #222](https://github.com/niklak/dom_smoothie/issues/222) 报告 DOM clone 内存开销；[rs-trafilatura #8](https://github.com/Murrough-Foley/rs-trafilatura/issues/8) 报告 UTF-8 字节截断 panic；[#9](https://github.com/Murrough-Foley/rs-trafilatura/issues/9) 报告嵌套表格增长；[Crawlberg #578](https://github.com/xberg-io/crawlberg/issues/578) 报告浏览器 E2E 的 Chrome 残留。本次用 DOM 复杂度上限、Unicode 切片和真实 Chrome 取消测试覆盖自己的路径，没有据这些 issue 断言所有上游版本都会失败。

**固定输入比较。** 分数为预先设置的事实字符串保留数。包含正文、标题和单独评论；只有检查评论字段才能公平消费某些算法的结果。`dom + xberg` 指 dom_smoothie 与 html-to-markdown-rs，`dom + htmd` 是直接使用默认 Readability 结果，没有 Agena 的语义区域优先策略。

| 输入 | 旧 Agena/crw | dom + htmd | dom + xberg | trafilatura | rs-trafilatura | 新 Agena |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| article | 4/4 | 4/4 | 4/4 | 4/4 | 4/4 | 4/4 |
| documentation | 4/4 | 4/4 | 4/4 | 4/4 | 4/4 | 4/4 |
| discussion | 4/4 | 4/4 | 4/4 | 4/4 | 4/4 | 4/4 |
| split-forum | **1/5** | 5/5 | 5/5 | 5/5 | 5/5 | **5/5** |
| Rust release | 2/2 | 2/2 | 2/2 | 2/2 | 2/2 | 2/2 |
| 中文 Rust book | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| Tokio spawn | 3/3 | 3/3 | 3/3 | **2/3** | 3/3 | 3/3 |
| MDN async | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| technical-structure | 5/5 | 5/5 | 5/5 | **2/5** | **2/5** | 5/5 |
| short-main | 1/1 | 1/1 + 1 噪声 | 1/1 | 1/1 + 1 噪声 | 1/1 | 1/1 |
| relative-links | 2/2 | 2/2 | 2/2 | 1/2 | 1/2 | 2/2 |
| nested-table | 3/3 | 3/3 | 3/3 | 3/3 | **0/3** | 3/3 |
| chinese-forum | 5/5 | **2/5** | **2/5** | 5/5 | 3/5 | **5/5** |
| math-and-details | 4/4 | 4/4 | 4/4 | 3/4 | 3/4 | 4/4 |

新实现全部保留上述事实标记，人工噪声标记没有残留。公开页面未预设噪声标记，因此不能用该结果声称它们完全没有无关内容。语义 main 优先也有取舍：网站可能把推荐内容一起放进 main；过度激进清理则可能再次损失短问答。这里优先保证内容完整，并通过 `extraction_strategy` 说明采用了什么路径。[比较 JSON](rust-crawling-comparison.json) 保存可检查的输入哈希、标记、输出哈希和人工样本全文；[重放说明](rust-crawl-probe/README.md) 给出具体命令。

**工具使用与预算。** 以下示例是调用参数说明，本次调查没有实际调用这些 Agena tools。

```json
{"urls":["https://docs.rs/tokio/latest/tokio/task/fn.spawn.html","https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/"],"concurrency":4,"max_chars":4000}
```

上面用于 `web.fetch_many`。每个成功请求返回 `ok` 和 `usable`：`ok` 仅说明得到页面结果，`usable` 还要求完整、readable、2xx；因此 403/验证码页面不会被包装成可用正文。批次的 `partial` 表示至少一个 URL 不可用，其他 URL 的结果保留。

```json
{"page_id":"<上一次返回的快照 ID>","offset":4000,"max_chars":8000}
```

上面用于 `web.read`，应使用实际返回的 `next_offset`，不是猜测字节偏移。偏移按 Unicode scalar/字符计数；前后切片没有添加省略号或修剪，可以拼回原快照。`next_offset: null` 表示提取文本读完；若 `truncated: true`，仍不能宣称读到了完整源页面。快照 TTL 15 分钟，共享 32 MiB 加权内存上限，可能提前逐出；同 URL 的新内容生成不同 ID，不会修改旧快照。

```json
{"start_url":"https://example.com/docs/","max_pages":20,"max_depth":2,"concurrency":4,"same_host_only":true}
```

上面用于 `web.crawl`。URL 发现数也有独立上限；`max_pages` 包括失败尝试和缓存命中。抓完后用 `web.query`，例如 `{"query":"authentication","max_results":5}`，结果会说明原抓取时间，并返回 `page_id/read_offset` 继续读。查询是本地全文检索，不联网刷新页面，也不是向量语义检索。缓存与快照读取仍需重新检查 requested/final URL 的当前许可。

| 预算/控制 | 默认与硬上限 |
| --- | --- |
| fetch 初始正文 / read 每次正文 | 8,000 字符；可选 1–24,000 |
| fetch_many | 输入 1–8 个 URL；默认并发 4，最大 8；每页默认 4,000 字符，最大 8,000 |
| crawl 并发 | defaults.concurrency=4，limits.concurrency=8；调用值受配置限制 |
| 共享抓取与浏览器 | 物理抓取最多 8 个；渲染最多 2 个；每主机节奏继续适用 |
| HTTP/自动渲染整体时间 | 沿用 fetch.request.timeout_secs，默认 30 秒；自动重试消耗剩余时间，不重新获得完整预算 |
| 传输内容 | 沿用默认 5 MiB，配置硬上限 32 MiB；仍有重定向和 robots 限制 |
| 提取文本 | 2 MiB，UTF-8 边界截断并报告 truncated |
| DOM 提取 | 50,000 元素、深度 256；超出返回 too_complex |
| 页面链接 | 最多 2,000 个；返回预览仅 32 个，并报告可用数量 |
| query | 默认 5 个命中，上限 20；被拒绝的命中不输出内容 |

DOM 数量检查发生在有传输体积上限的 HTML 解析之后，它不是解析器峰值内存的严格承诺。CPU 提取在有许可数量上限的 blocking worker 中运行；超时会结束调用，已开始的 CPU 工作需要自行运行至结束，不能强制中断 Rust 线程。网络 futures 随取消丢弃；浏览器 context 通过受控生命周期清理。

系统提示词没有把所有库细节塞给模型。只有相关工具实际可用时，才注入批量并发、页面状态、续读、crawl/query 的说明；测试遍历了 64 种 web 工具可用组合，避免提示词引用不可用工具。并发规则区分独立 URL 与依赖前一步才能知道的 URL，并明确同一浏览器 tab 的状态修改应按顺序进行。

**验证和已知边界。** 本次检查覆盖库单元测试、Web 插件测试、工具 schema/help、系统提示词、文档与能力身份快照，并单独用本地 HTTP fixture 和 Chrome 验证 HTTP 壳页→渲染→正文提取、重定向/子资源拒绝、404、Unicode 字节预算、超时和取消 context 清理。验证命令和最终结果记录在 [validation JSON](rust-crawling-validation.json)。测试不是对所有线上网站可用率的承诺。

本次没有实现验证码绕过、登录态自动继承、任意 PDF/OCR、sitemap/map、无限滚动、跨域站点图或持久化分布式爬虫。JS 壳页判断是保守启发式，一些 SPA 仍需要明确 `render_js: true` 或交互浏览器。浏览器可用需要本地 Chrome/Chromium 和相应配置；默认只改成按需尝试，不会在未启用时自动安装浏览器。爬取同一 workspace 仍由 crawl lock 串行组织多个运行，单次运行内部并发。

后续若具体站点仍有缺口，应把失败 HTML 固定为回归样本，再决定是否引入新的选择器、提取器或浏览器后端。优先级是补充代表性真实输入和结果质量评估，其次是有预算的 sitemap/feed 发现；Crawlberg 整体替换、Playwright Rust bridge 迁移和新型轻量浏览器应分别做能覆盖宿主权限、取消、兼容性与部署成本的原型。
