# AI 调用厂商云端工具时的 adapter 门槛

Agena 只在 **AI 实际执行厂商云端工具** 时检查当前运行时 adapter。工具发现、列表、搜索和帮助不按 adapter 过滤。

这是一道粗粒度执行门槛，用于阻止明显错误的跨厂商调用；它不替代模型能力、账号权限、地区、套餐或具体端点能力校验。

## 当前 adapter 规则

| 当前实际 adapter | 可以执行的厂商云端工具 |
| --- | --- |
| `openai_responses` | `chatgpt.cloud_*` |
| `openai_chat_completions` | `chatgpt.cloud_*` |
| `anthropic` | `claude.cloud_*` |
| `gemini` | `gemini.cloud_*` |
| `ollama` | 不执行上述三家的厂商云端工具 |
| `gitlab` | 不执行上述三家的厂商云端工具 |
| `amazon_bedrock` | 不执行上述三家的厂商云端工具 |
| 未知、无法解析或跨组调用 | 拒绝 |

OpenAI 只保留 Responses 与 Chat Completions 两个 adapter。其他 OpenAI 协议名不是 alias，也不会自动映射到这两个 adapter。

Google Gemini 自己的 WebSocket 流传输属于 Gemini adapter 内部实现，不属于 OpenAI adapter。

## adapter 身份来源

执行门槛只使用运行时已经解析出的可信 adapter：

- 显式选择的 adapter；
- 模型路由选择的 adapter；
- Provider 的 default adapter。

Provider 名称、模型名称、工具输入中的 `provider` / `adapter` 字段都不能覆盖这个结果。

因此：

- 一个叫作 `openai` 的 Provider 不会自动获得 OpenAI 工具；
- 一个模型名里包含 `claude` 的 Bedrock/GitLab 路由不会自动获得 Anthropic 直连工具；
- 模型不能通过伪造工具参数切换厂商。

## 执行行为

门槛在两个位置检查：

1. tool invocation 预检进入插件 hook 之前；
2. 普通或流式插件真正执行之前。

这样即使一个调用已经准备好，但随后运行时 scope 的 adapter 改变，真正执行时仍会再次拒绝。

覆盖的 AI 执行路径包括：

- 顺序 tool calls；
- 并行 tool calls；
- 暂停/审批后恢复的 tool calls。

拒绝时返回 `CapabilityUnavailable`，capability 为 `provider_tool_adapter`，并说明要求的 adapter 组和当前实际 adapter。拒绝发生在厂商网络请求、文件上传和本地 provider artifact 创建之前。

Agena 不会：

- 自动切换 Provider；
- 自动换 adapter；
- 失败后调用另一家云端工具；
- 把工具参数中的 adapter 当作授权；
- 因工具目录中存在另一家工具就允许执行。

## OpenAI Chat Completions 的含义

`openai_chat_completions` 可以让 AI 发起 Agena function/tool call，因此允许调用 `chatgpt.cloud_*`。

这并不意味着 Agena 把 OpenAI Responses 的 hosted-tool JSON 声明直接发送给 Chat Completions。厂商插件仍然独立构造其实际支持的 OpenAI 请求；Provider adapter 自己的原生工具协议校验不会被这道门槛放宽。

## 与工具发现的关系

工具目录仍然是全量当前目录。

因此使用 Anthropic 模型时，AI 仍可能在搜索结果中看到 OpenAI/Google 云端工具，但真正调用错误厂商工具时会在执行前被拒绝。

这样可以保持 Tool API 简单，同时保证执行边界不会因为模型记住了一个错误工具名而被绕过。

## 非厂商云端工具

本门槛不影响 Agena 原生工具，例如：

- `shell.run`
- `fs.read`
- `fs.apply_patch`
- 浏览器工具
- MCP 工具
- memory / tasks / plan 等运行时工具

这些能力继续使用自己的权限和运行时边界。

用户直接发送图片、文件或其他附件也不经过厂商云端工具门槛；该路径使用媒体输入自己的 Provider 路由绑定和模态能力检查，见 `docs/media-inputs.md`。

## 当前覆盖原则

配置中的真实 `ProviderKind` 共有七种：

- `ollama`
- `openai_responses`
- `openai_chat_completions`
- `anthropic`
- `gemini`
- `gitlab`
- `amazon_bedrock`

adapter 门槛测试对这个枚举使用穷尽匹配。新增 ProviderKind 时，测试代码必须显式决定它属于哪个云端工具组，否则编译/测试会暴露缺口。

测试矩阵覆盖七种 adapter × 当前 32 个厂商云端工具，并额外覆盖：

- 匹配 adapter 放行；
- 跨厂商 adapter 拒绝；
- 未解析 adapter 拒绝；
- 模型参数伪造 adapter 无效；
- 已准备调用在 scope 改变后再次拒绝；
- 顺序/并行模型工具循环；
- 非厂商原生工具不受影响；
- 工具发现和帮助不因 adapter 改变。

所有这类测试使用合成 Provider/插件或本机 HTTP fixture，不要求真实厂商 API key，也不会发起付费调用。
