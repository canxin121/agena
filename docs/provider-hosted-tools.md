# 厂商插件：仅保留云端业务能力

> 媒体重写新增的图片/文档理解、云端文件生命周期和用户直接发送/剪贴板契约见 `docs/media-inputs.md`。下文保留上一阶段 17 个云端入口的历史验收记录，不作为当前工具数量。

本次收缩以 `b8196002` 的 39 个厂商入口为基线：保留 17 个，移除 22 个公开入口。其他 103 个执行工具保持不变；总执行工具从 142 变为 120。主模型的工具调用协议、Agena 原生文件/终端/浏览器/记忆/MCP/子任务实现没有被删除。

## 名称直接标明云端执行

所有保留入口统一采用 `厂商.cloud_能力`。名称、模型看到的 summary/help、执行中的标题、完成后的结果视图都明确显示厂商云端，不能只靠插件品牌名猜测执行位置。

- 云端执行：例如 `chatgpt.cloud_shell`、`claude.cloud_code_execution`、`gemini.cloud_image_edit`。
- 本地能力：`shell.run`、`fs.read`、`web.browser_*` 等名称和权限契约保持不变。
- 发送范围：prompt 和显式提供的输入会发送给配置的厂商端点。云端不会自动获得本地项目或整个会话；图片编辑会上传已获读取权限的图片，结果作为单独工件保存。
- 标识分层：`cloud_` 只属于 Agena 公共工具名；厂商 API 的 `type`、Anthropic 的 versioned name、响应类型和内部计费 operation 均保持原值，不能把 `cloud_shell` 发给厂商代替 `shell`。

### 原名到新名

| 原公开名称 | 新公开名称 | 执行位置 |
| --- | --- | --- |
| `chatgpt.web_search` | `chatgpt.cloud_web_search` | OpenAI 云端 |
| `chatgpt.file_search` | `chatgpt.cloud_file_search` | OpenAI 云端 |
| `chatgpt.code_interpreter` | `chatgpt.cloud_code_interpreter` | OpenAI 云端 |
| `chatgpt.image_generation` | `chatgpt.cloud_image_generation` | OpenAI 云端 |
| `chatgpt.image_edit` | `chatgpt.cloud_image_edit` | OpenAI 云端 |
| `chatgpt.shell` | `chatgpt.cloud_shell` | OpenAI 云端 |
| `claude.web_search` | `claude.cloud_web_search` | Anthropic 云端 |
| `claude.web_fetch` | `claude.cloud_web_fetch` | Anthropic 云端 |
| `claude.code_execution` | `claude.cloud_code_execution` | Anthropic 云端 |
| `claude.advisor` | `claude.cloud_advisor` | Anthropic 云端 |
| `gemini.code_execution` | `gemini.cloud_code_execution` | Google 云端 |
| `gemini.url_context` | `gemini.cloud_url_context` | Google 云端 |
| `gemini.google_search` | `gemini.cloud_google_search` | Google 云端 |
| `gemini.file_search` | `gemini.cloud_file_search` | Google 云端 |
| `gemini.google_maps` | `gemini.cloud_google_maps` | Google 云端 |
| `gemini.image_generation` | `gemini.cloud_image_generation` | Google 云端 |
| `gemini.image_edit` | `gemini.cloud_image_edit` | Google 云端 |

这 17 个原名不再作为可执行别名注册。使用简写、`agena.` 全名或旧 wire 名都会返回明确的新名称和云端数据边界，要求重新查看 `tools_help` 后发起独立调用；不静默跳转，不执行本地回退。旧会话的历史名称及其专用结果展示仍可读取。

若权限规则、工具白名单或保存的快捷调用使用完整旧名称，需要显式改为对应的新名称。本次不会自动放宽、重写或迁移用户权限配置；插件 ID、模型/API key 配置名称和厂商端点保持不变。

响应增加 `public_tool_name`、`execution_location=vendor_cloud`、`execution_provider` 和 `local_workspace_automatically_available=false`。原有 `tool` 字段仍表示厂商 API operation，供计费和历史结果兼容；界面分别标注公共云端工具与 API operation，避免混淆。

## 执行边界

厂商插件负责请求托管服务、管理输入/输出、来源和计费用量，不负责执行模型返回的本地 Shell、补丁、函数、记忆或计算机动作。`chatgpt` 是 OpenAI API 适配器，不是 ChatGPT 产品登录/订阅。凭据仍使用配置的 API 环境变量，未擅自迁移账号配置。

`chatgpt.cloud_shell` 默认发送 `environment.type=container_auto`；只允许该模式和带 `cntr_` ID 的 `container_reference`。`local`、未知类型、null、字符串环境，以及伪造的客户端 `shell_call_output` 输入都会被拒绝，不会回退到 Agena Shell。云端容器不自动获得本地项目文件；需要通过厂商支持的文件资源显式提供输入。

在本地读取输入图片、写出图像/响应回执，并不把云端图像或计算能力变成本地业务执行器。这些 I/O 继续受原有权限声明约束。配置自定义 `base_url` 的管理员仍需信任该代理；`vendor_hosted_only` 表达请求/响应协议边界，不是对远端实际机器的加密证明。

## 保留的 17 个入口

| 插件 | 入口 | 实际能力 |
| --- | --- | --- |
| chatgpt | cloud_web_search | OpenAI 托管 Web 搜索 |
| chatgpt | cloud_file_search | OpenAI 向量库检索 |
| chatgpt | cloud_code_interpreter | OpenAI 托管计算容器 |
| chatgpt | cloud_image_generation | OpenAI 图像生成 |
| chatgpt | cloud_image_edit | OpenAI 图像编辑 |
| chatgpt | cloud_shell | 仅 OpenAI 托管 Shell 容器 |
| claude | cloud_web_search | Anthropic 服务器搜索 |
| claude | cloud_web_fetch | Anthropic 服务器抓取 |
| claude | cloud_code_execution | Anthropic 托管代码/命令环境 |
| claude | cloud_advisor | Anthropic 顾问模型推理 |
| gemini | cloud_code_execution | Google 托管代码执行 |
| gemini | cloud_url_context | Google URL 上下文 |
| gemini | cloud_google_search | Google 搜索 grounding |
| gemini | cloud_file_search | Google File Search 存储检索 |
| gemini | cloud_google_maps | Google Maps grounding |
| gemini | cloud_image_generation | Gemini 图像生成 |
| gemini | cloud_image_edit | Gemini 图像编辑 |

名称注册不意味着所有模型、账号和 API 版本都支持它。File Search 仍需要相应云端知识库；Advisor 需要独立的顾问模型参数；本次没有声称已用真实付费接口验证所有组合。

## 移除的公开入口

### 客户端执行和纯声明：13 个

- `chatgpt.computer`、`chatgpt.computer_use_preview`、`chatgpt.local_shell`、`chatgpt.apply_patch`、`chatgpt.function`、`chatgpt.custom`、`chatgpt.namespace`
- `claude.bash`、`claude.computer`、`claude.memory`、`claude.text_editor`
- `gemini.computer_use`、`gemini.function`

这些入口产生应用侧动作或定义工具，本身不是新的厂商托管业务执行器。对应本地动作应独立选择 Agena 原生工具，走自己的权限和版本检查。删除包装不等于删除主模型 Function Calling。

### 不再作为独立业务工具公开：9 个

- `chatgpt.web_search_preview`：旧兼容入口，迁移到 `chatgpt.cloud_web_search`，不做静默改写。
- `chatgpt.mcp`、`claude.mcp_toolset`、`gemini.mcp_server`：由指定 MCP 服务器实际执行，不宣称是模型厂商托管能力。继续使用 Agena 原生 `mcp.tools.search` / `mcp.tools.call`；本次没有新建一套远端连接器。
- `chatgpt.programmatic_tool_calling`、`chatgpt.tool_search`、`claude.tool_search_bm25`、`claude.tool_search_regex`：编排/工具发现机制不再平铺成业务入口。没有用未连接任何工具的空实现替代；将来确有完整云端工作流时可在内部适配层实现，不能添加隐式本地执行回退。
- `gemini.retrieval`：当前公开 Interactions 工具类型中未确认，停止暴露；明确改用已配置的 File Search 或 Google Search。

## 迁移与历史记录

退役名称（紧凑名、`agena.` 全名和原 wire 名）返回结构化的不可重试迁移提示，列出可查询的原生/云端候选，说明本次没有执行动作和没有自动跳转。`tools_help` 同样解释退役原因，批量帮助仍保留其他正常项。

删除的只是注册入口和执行包装。旧会话、历史响应回执，以及这些旧名称的标题/展示逻辑保留，不通过删除历史或把历史名字改成新名字来伪装兼容。

## 请求与响应的防回退约束

请求发出前验证：不可重新声明 `tools`/`functions`、嵌入 MCP 配置、选择客户端执行模式或提交客户端回调结果。图像请求也使用这一约束，不能通过 `request_options` 或图像 `options` 添加函数工具。正常文本中的 JSON 示例不会被当成可执行协议。

响应保留用量、响应 ID、回执和服务器结果，但不向上层提供可执行的本地待办：

- `execution_boundary: vendor_hosted_only`
- `local_actions_executed: false`
- `client_execution_allowed: false`
- `pending_calls: []`
- `execution_evidence`：已观察到的托管调用、匹配结果、未确认的托管调用和边界异常

OpenAI `custom_tool_call.status=completed` 只表示调用生成完成，不证明本地已执行。若托管请求意外返回 Function/Custom/Computer/本地 Shell 等客户端动作，结果为 `blocked_client_execution`，不展示其执行参数作为下一步操作，不执行/自动重放；原始响应回执仅供排查。

Anthropic 的 `server_tool_use` 与对应 `*_tool_result` 按 ID 关联；已完成搜索/计算不被误标为需要本地执行。`pause_turn` 是继续厂商托管工作，不要求伪造 `tool_result`。OpenAI hosted Shell 的 `shell_call` 与 `shell_call_output` 配对；只有调用没有执行结果时，不因顶层 completed 就宣称执行已完成。Google 的 `code_execution_call/result` 同样属于托管协议。

协议扫描有节点数和深度上限；超限返回未完成校验/阻止状态，不把未检查的剩余内容判安全。这里只解释执行责任，不承诺模型必定选用所提供工具：没有工具执行证据的纯文本回复不会虚构托管调用次数。

## 协议依据

本次在线核对的官方文档：

- OpenAI Shell：https://developers.openai.com/api/docs/guides/tools-shell
- Anthropic 工具分类与版本：https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-reference
- Anthropic Advisor：https://platform.claude.com/docs/en/agents-and-tools/tool-use/advisor-tool
- Google Interactions：https://ai.google.dev/gemini-api/docs/interactions-overview
- Google Code Execution：https://ai.google.dev/gemini-api/docs/code-execution

当前 Anthropic 文档的 Web Search、Web Fetch、Code Execution 不需要旧实验 beta header，因此移除这些硬编码 header；Advisor 使用 `advisor-tool-2026-03-01`。用户显式配置的其他 headers 不被擅自修改。

## 验证方式

`provider_hosted_only.rs` 通过真实 PluginHost/ToolExecutor 调用本机模拟 API，逐项检查全部 17 个保留入口的实际请求、错误参数零网络请求、22 个退役入口的迁移、原生 Shell/文件操作仍可用、服务器结果不重复执行，以及 hosted pause/resume。

测试的 API key 是明确的假值，并且仅设置在隔离 HOME 的子测试进程中；不读取真实密钥、不发起真实厂商付费调用。原有权限、展示、目录快照、运行时与 Provider 路由测试继续运行。最终通过结果应以本目录的验收状态文件为准，而不是仅凭注册数量判断完成。

## 托管边界验收历史（云端名称调整前）

- 共 **568 项测试通过，0 失败、0 忽略**，19 个测试套件；包括完整的原生工具回归、17 个保留入口的模拟 API 请求、全部退役别名、非法参数零网络请求及返回客户端动作的阻止处理。
- 九个核心 crate 的 `--all-targets` Clippy 以 `-D warnings` 通过。
- Runtime、Session、MCP Server、TUI App、Application、API Server 六个下游目标的 `--all-targets` 编译通过。
- 生成工具参考与能力身份快照均与编译结果一致；与变更前快照逐项比较，**103 个原生执行工具所属插件的定义和 schema 完全一致**。
- 19 项仓库不变量检查、格式检查和 `git diff --check` 通过。

专项回归还发现并修复了一个旧的结果合并问题：原始厂商 `status` 曾能覆盖本地归类的 `outcome`。现在原始状态保存在 `provider_reported_outcome`，不能覆盖执行边界、阻止状态和回执事实。

本次固定源码指纹：

```text
10f76d062e66bef0c427a304830de7a1d3a4c8bf845f6ae186f0bd0906ae8751
```

机器可读结果：`docs/provider-hosted-tools-status.json`。完整 gate 命令、退出码、日志哈希和源码一致性记录：`.tmp-artifacts/provider-hosted-only/final-results.json`。最初失败的集成日志保留，没有以旧版测试结果替代新源码验收。

代码与生成目录已更新，尚未提交或推送 Git，也没有替换运行中的服务。厂商托管执行位置按官方协议和请求模式约束；模拟 API 验证不等于真实账号/模型/区域的付费 API 兼容性认证。
## 云端名称调整后的验收结果

本次统一了 17 个云端工具的公开名称、summary/help、调用标题、结果字段和旧名称迁移提示；没有改变厂商 API 的工具类型、计费 operation、模型/API key 配置或任何原生工具 schema。改名前的 22 个退役包装仍然退役，其迁移建议已指向新的有效云端名称。

最终 **574 项测试通过，0 失败、0 忽略**，19 个测试套件。九个核心 crate 的严格 Clippy（`--all-targets -- -D warnings`）、六个下游 runtime/session/MCP/TUI/application/API 目标编译、工具参考与能力身份快照一致性、19 项仓库不变量以及格式/差异检查均通过。

专项测试逐项覆盖 17 个名称的简写、全名和 wire 名；初始/完成/空结果标题保留 cloud 标识；历史结果仍有原有专用视图；旧名称不触发网络请求或自动跳转；请求里的厂商原始工具类型保持不变。回归还修复了 wire 名被下划线归一化导致 cloud 标题丢失，以及迁移消息尾部被紧凑错误文本截断的问题。

103 个原生执行工具所属插件的定义和 schema 与改名前逐项一致，总执行工具仍为 120。API 契约测试只使用本机模拟服务、假凭据与隔离测试进程，不构成真实付费接口或所有模型版本的在线认证。

本次验证源码指纹：

```text
2ba42787bc642e6d0a26def9de7f36b80a07ad14f1086d6c069ebfe1050bb3ed
```

最新机器可读状态为 `docs/provider-hosted-tools-status.json`；每项命令、退出码、日志摘要和源码一致性证据在 `.tmp-artifacts/provider-cloud-names/final2-results.json`。本次未 commit/push，也未重启运行中的 Agena/MCP 服务；新名称在服务加载新构建后生效。
