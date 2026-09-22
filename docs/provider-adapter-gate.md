# AI 调用云端工具时的 adapter 门槛

只增加执行时校验，不改变工具发现、列表、搜索、帮助、manifest、权限配置或厂商请求参数。不推断模型完整能力，不重构 Provider 配置。

## 完整 adapter 对应规则

按配置中真实的七种 adapter 明确覆盖；只判断 AI 调用者所属协议组，不改变插件执行请求的协议。

| AI 当前实际选定 adapter | 可调用的厂商工具 | 处理依据 |
| --- | --- | --- |
| `openai_responses` | 已注册的 `chatgpt.cloud_*` | OpenAI 协议组 |
| `openai_chat_completions` | 已注册的 `chatgpt.cloud_*` | OpenAI 协议组，不应遗漏 |
| `anthropic` | 已注册的 `claude.cloud_*` | Anthropic 协议组 |
| `gemini` | 已注册的 `gemini.cloud_*` | Google Gemini 协议组 |
| `ollama` | 不放行上述三家的云端工具 | 不把本地/自建模型视为厂商云服务 |
| `gitlab` | 不放行上述三家的云端工具 | GitLab 是独立路由，不按其内部模型名称猜厂商 |
| `amazon_bedrock` | 不放行上述三家的云端工具 | AWS Bedrock 的凭据/执行入口不等于 Anthropic 直连 |
| 未知、拼错、无法解析或跨组调用 | 拒绝本次云端工具调用 | 保守拒绝，不自动切换 |

OpenAI Chat Completions 放行的是 **AI 发起的 Agena 工具调用**；厂商插件仍独立构造其支持的服务请求。没有把 Responses 的 hosted-tool 定义直接塞入 Chat Completions，原生请求的能力校验没有放宽。

Provider 自定义标签、模型名字、ChatGPT 订阅等配置名不是 adapter 身份。未显式选择 adapter 时继续用实际模型路由/default adapter。本次不为 Bedrock/GitLab 猜测底层厂商，也不改变端点或凭据选取。

## OpenAI adapter 范围

Agena 的 OpenAI adapter 只保留 Responses 与 Chat Completions 两种。配置中的其他 OpenAI 协议名按未知 adapter 拒绝，不做兼容映射或自动回退。Google Gemini 自己的 WebSocket 流传输模式属于 Google adapter 内部选项，不属于 OpenAI adapter。

## 运行时行为

AI 工具执行器携带可信的 adapter 快照。检查使用工具注册表解析出的 canonical identity，覆盖紧凑名、完整名以及经过 `tools_call` 分发的调用；原本不注册为调用别名的 wire 名继续返回工具不可用，不新增别名；工具输入中的 `adapter`/`provider` 字段不影响门槛。

在执行预检进入插件 hook 前检查一次，在普通或流式插件实际执行前再次检查，避免已准备调用在切换执行 scope 后绕过。普通、并行和审批恢复的 AI 执行路径均安装门槛。错误返回 `CapabilityUnavailable`，capability 为 `provider_tool_adapter`，包含要求的和实际 adapter；不发送厂商请求、不自动切换服务商、不把失败作为可重复执行的请求。

发现/列表/帮助使用原目录；允许看见另一家工具，只在 AI 实际调用时拒绝。原生 Shell、文件、浏览器、MCP 等非厂商云端工具不受此门槛影响。没有 AI adapter 门槛的显式 application/host 工具调用保持原行为；用户直接发送附件仍使用已有媒体路由与权限检查，不改成厂商工具调用。

## 这不是完整兼容性认证

adapter 匹配仅是第一道粗粒度门槛，不证明实际端点支持该厂商的所有工具，也不验证账号套餐、模型版本、地区或独立插件凭据是否可用。插件原有网络、文件权限、输入验证和远端错误处理继续生效；本次不改 endpoint/credential 选择，不增加自动探测或跨 Provider 授权框架。

## 测试

- 32 个已注册云工具在错误/未知 adapter 下被拒绝，覆盖全部名字形式与伪造输入；拒绝前没有 HTTP 请求和上传工件。
- 四个匹配 adapter（含两个 OpenAI 协议）可通过原工具流程访问本机模拟服务。
- 已准备调用在执行 scope 改为错误 adapter 后仍被拒绝。
- 执行门槛前后的完整工具定义和帮助相同；两份生成目录与修改前逐字节一致。
- 原生文件/终端在未知 adapter 下仍可执行。
- 实际默认、按模型选择、显式选择、禁用和不存在的 adapter，以及模型目录 wrapper 均有解析测试。
- 合成模型发起真实 `tools_call`，验证顺序和并行运行中的拒绝，以及正确 default adapter 放行；拒绝作为工具结果反馈给模型。

测试仅使用本地合成插件、输入和 HTTP 服务，没有读取真实 API 密钥或发起付费厂商请求。实现已提交为 `ef00f41f`；本地验证日志位于 `.tmp-artifacts/openai-adapter-cleanup/`，该目录不进入 Git。服务未因本次变更自动重启。
## 前一轮验收记录（覆盖补齐前）

本次选定测试共 621 项通过，0 失败，其中包括 10 项新增 adapter 门槛/路由回归。四个相关核心 crate 的严格 `--all-targets -- -D warnings` Clippy、六个下游 runtime/session/MCP/TUI/application/API 目标编译、生成契约一致性、仓库不变量、格式与 diff 检查均通过。

工具定义、schema、生成帮助文件与改动前逐字节一致，执行工具总数仍为 134。测试使用本机合成插件和 HTTP 服务，无真实厂商付费调用。匹配规则只是简单门槛，未扩展为模型/套餐/端点能力探测。

该段是前一轮门槛验证的历史记录；其“未 commit/push”描述只代表当时快照。当前实现的 durable source reference 为 `ef00f41f`。

验收调度说明：最终 621 项选定回归使用单线程测试调度。前一轮并行调度中两项原有取消时序测试失败，原断言和生产取消逻辑未改；完整串行复跑均通过。新增门槛测试中的并行 AI 调用场景仍真实并发执行，测试未被跳过。失败日志保留在 `final3-tests.log`。

## adapter 完整性回归

测试对真实 `ProviderKind` 的七个变体使用无兜底分支的穷尽匹配，再验证七种 adapter × 32 个已注册厂商工具，共 224 个组合。新增 enum 变体却未明确测试策略时会编译失败，避免继续把新增 adapter 悄悄落到通用拒绝分支。

匹配组验证能够通过运行时预检；不匹配组验证预检和直接执行都会拒绝，且不发送 HTTP 或写入厂商工件。OpenAI 两种入口另有真实 `model → tools_call → executor → model` 合成回归，并检查 Chat Completions 仍不能调用 Claude 工具。发现/帮助在全部七种 adapter 和未解析状态下保持一致。

当前七种 adapter 的结果见 `docs/provider-adapter-gate-status.json`；本地日志位于 `.tmp-artifacts/openai-adapter-cleanup/`，不作为克隆仓库后的持久证据。
