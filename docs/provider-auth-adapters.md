# Provider / Auth / Adapter 架构

本文统一说明当前 provider 子系统的整体结构和认证模型，也就是：

- `provider` 是什么
- `auth` 放在哪里，为什么放在那里
- `provider.auth` 支持哪些模式
- 这些 auth 模式和各类 adapter 的约束关系
- `adapter` 负责什么
- model route 为什么挂在 adapter 下

## 设计目标

现在的配置模型要解决的是三件事：

1. 把“逻辑入口”和“真实后端协议实现”拆开
2. 把“认证”收敛到 provider 自己，而不是放成外部共享对象
3. 让一个 provider 可以自然暴露多个 adapter 和多个 routed models

所以 canonical 结构是：

```text
provider
├── default_model
├── auth
└── adapters
    ├── <adapter_kind>
    │   ├── protocol/options
    │   └── models
    │       └── <visible_model_id>
    │           ├── target_model
    │           └── metadata / capabilities / variants
    └── ...
```

对应到 TOML：

```toml
[providers.<provider_id>]
default_model = "..."

[providers.<provider_id>.auth]
mode = "..."

[providers.<provider_id>.adapters.<adapter_kind>]
default_model = "..."

[providers.<provider_id>.adapters.<adapter_kind>.models.<visible_model_id>]
target_model = "..."
```

## 三层职责

### 1. provider

`provider` 是 Agena 对外暴露的逻辑入口。

它负责：

- 对外提供稳定的 `provider_id`
- 暴露默认模型
- 持有 provider-level `auth`
- 聚合一个或多个 adapters
- 对外暴露扁平化后的 routed model 表

CLI、HTTP API、Studio、model ref 都是围绕 `provider_id` 工作的，而不是围绕 adapter id 工作。

### 2. auth

`provider.auth` 是这个 provider 自己的认证配置。

它负责：

- 提供 endpoint / token / OAuth / ADC / SigV4 / service key 等认证来源
- 决定 refresh 生命周期
- 持有 provider-specific metadata
- 作为同一个 provider 下所有 adapters 的共享认证上下文

这就是为什么 `auth` 一定要挂在 provider 上，而不是挂在 adapter 上，也不是挂成 provider 外部共享对象。

### 3. adapter

`adapter` 代表真实的协议实现和后端行为。

它负责：

- 选择 wire protocol
- 处理 provider-specific transport 选项
- 决定请求走哪组 endpoint path / header 语义 / stream 语义
- 暴露这个 adapter 下面有哪些 routed models

例如：

- `openai`：OpenAI 风格 richer transport，支持 `responses` / `chat`
- `openai_compatible`：纯 OpenAI-compatible chat-completions transport
- `anthropic`：Anthropic `/v1/messages`
- `gemini`：Gemini 官方协议
- `gitlab`：GitLab Duo / AI Gateway
- `amazon_bedrock`：AWS 原生 Bedrock
- `ollama`：本地 Ollama

## 为什么 auth 不放在 adapter 上

因为真实需求不是“每个 adapter 都有一套完全独立认证”，而是“同一个逻辑 provider 下多个 adapters 共享同一套身份和 metadata”。

典型例子：

- 一个 Copilot provider 同时暴露 `openai` 和 `anthropic`
- 一个 shared gateway provider 同时暴露 `openai` 和 `anthropic`
- 一个 provider 同时暴露 `openai` 和 `openai` backend 的不同模型路由

这些情况下，共享的是 provider 这层的身份，而不是 adapter 各自独立登录。

如果你真的需要不同认证，那应该拆成两个 provider，而不是在一个 provider 里塞两个同 kind adapter。

## 为什么 model routes 挂在 adapter 下

因为“一个可见模型最终该走哪个真实协议实现”本身就是 adapter 级决策。

例如：

- `gpt-4o-mini` 走 `openai`
- `claude-sonnet-4` 走 `anthropic`
- `amazon.nova-pro-v1:0` 走 `openai_compatible`

因此 canonical 路径是：

```text
providers.<id>.adapters.<adapter_kind>.models.<visible_model_id>
```

而不是 provider 根下直接堆一张没有来源的 model 表。

运行时会把这些路由扁平化成 provider 对外可见的模型表，但配置上的归属仍然在 adapter 下。

## 一个 provider 为什么只保留一个同 kind adapter

因为 adapter kind 表示的就是“协议实现类别”。

同一个 provider 下如果再出现两个同 kind adapter，通常说明你其实在混两件事：

- 想用不同认证
- 想用不同 endpoint/base_url

这两件事都不应该通过“同 provider 下复制同 kind adapter”解决：

- 不同认证：拆 provider
- 同认证下多个模型：放到同一个 adapter 的 `models`

所以现在 adapter key 就是 kind，本身不再额外声明 `kind = "..."`，也不再支持“同 provider 下多个同类 adapter 各自起名字”的模型。

## Multi-adapter provider 的运行时形态

运行时会围绕 `MultiAdapterProvider` 工作：

- 对外仍然只暴露一个 `provider_id`
- 对内持有多个真实 adapter provider
- 根据 routed model 决定把请求转发到哪个 adapter

单 adapter provider 则可以直接 passthrough。

## `provider.auth` 支持哪些模式

| mode | 主要字段 | 典型用途 |
| --- | --- | --- |
| `none` | 无 | `ollama` |
| `api` | `base_url`、`api_key`、`api_key_env` | OpenAI API、Anthropic、Gemini、OpenRouter、GitLab direct token、OpenAI-compatible gateway |
| `credential` | `issuer`、`credential` | ChatGPT Codex OAuth、GitHub Copilot OAuth、GitLab OAuth |
| `bedrock_sigv4` | `base_url`、`region`、`profile`、`access_key_id`、`secret_access_key`、`session_token` | AWS 原生 Bedrock |
| `google_adc` | 无 | Google ADC / Vertex OpenAI-style endpoint |
| `sap_ai_core` | `base_url`、`api_key`、`api_key_env`、`service_key_env` | SAP AI Core |

其中：

- `api` 适合“静态 endpoint + 静态 token”
- `credential` 适合“provider-managed OAuth / 登录 / refresh”
- `google_adc`、`bedrock_sigv4`、`sap_ai_core` 是 provider-specific auth 模式

## `credential` 在这里是什么意思

`credential` 不是 provider 之外的独立对象，而是 `provider.auth` 里的一个具体字段，用来承载 provider-local 的 `AuthData`。

典型例子：

### OpenAI ChatGPT OAuth

```toml
[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }
```

### GitHub Copilot OAuth

```toml
[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, enterprise_url = "github.example.com" }
```

### GitLab OAuth

```toml
[providers.gitlab.auth]
mode = "credential"
issuer = "gitlab"
credential = { type = "oauth", issuer = "gitlab", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000 }
```

重要 metadata：

- `account_id`：ChatGPT Codex 请求头和 prompt-cache scope 会用到
- `enterprise_url`：Copilot enterprise host 解析和 prompt-cache scope 会用到

## 运行时怎么解析 auth

认证解析顺序取决于 `mode`。

### `mode = "api"`

顺序是：

1. `api_key`
2. `api_key_env`

### `mode = "credential"`

顺序是：

1. 读取 inline `credential`
2. 根据 `issuer` 和 provider 实现决定 refresh 策略

refresh 发生时，会直接更新 provider 自己持有的 `AuthData`。

### `mode = "google_adc"`

运行时使用 Google ADC 解析 access token，不依赖 `api_key` / `api_key_env`。

### `mode = "bedrock_sigv4"`

运行时使用 AWS SigV4 签名。`access_key_id` 和 `secret_access_key` 如果显式配置，必须成对出现；否则走 AWS 默认 credential chain / profile。

### `mode = "sap_ai_core"`

顺序是：

1. 先看 `api_key`
2. 再看 `api_key_env`
3. 如果没有 direct token，再从 `service_key_env` 读取 SAP service key，并运行时换取 token

默认的 service key 环境变量名是 `AICORE_SERVICE_KEY`。

## auth 和 adapter 的约束关系

这是最重要的一部分。`auth` 不是任意 adapter 都能随便配。

### `none`

- 只支持 `ollama`

### `api`

- 支持 `openai`
- 支持 `openai_compatible`
- 支持 `anthropic`
- 支持 `gemini`
- 支持 `gitlab`
- 不支持 `ollama`
- 不支持 `openai` 的 `backend = "chatgpt_codex"`

### `credential`

- `openai`
  - `issuer = "openai_chatgpt"` 时，只允许 `backend = "chatgpt_codex"`
  - `issuer = "github_copilot"` 时，只允许 `backend = "api"`
- `anthropic`
  - 目前只支持 `issuer = "github_copilot"`
- `gitlab`
  - 只支持 `issuer = "gitlab"`
- 其他 adapter 当前不支持 `credential`

### `google_adc`

- 只支持 `openai`
- 并且要求 `capability_family = "gemini"`

### `bedrock_sigv4`

- 只支持 `amazon_bedrock`

### `sap_ai_core`

- 只支持 `openai_compatible`

不再允许 `openai + capability_family = "openai_compatible"` 这种伪装写法。

## provider-specific 规则

### GitHub Copilot

- OAuth 认证写在 `provider.auth`
- GPT / Codex / Gemini 这类 Copilot OpenAI-style 模型走 `openai`
- Claude 这类 Copilot `/v1/messages` 模型走 `anthropic`
- 不要把 Copilot 的 Gemini 模型配到 `gemini` adapter

### ChatGPT Codex

- 只能用 `openai`
- 只能用 `backend = "chatgpt_codex"`
- 只能用 `credential`
- `issuer` 必须是 `openai_chatgpt`

### Amazon Bedrock

- AWS 原生接口：`amazon_bedrock` + `bedrock_sigv4`
- OpenAI-compatible token endpoint：`openai_compatible` + `api`

### SAP AI Core

- 走 OpenAI-compatible 协议
- canonical 适配器是 `openai_compatible`
- 认证模式是 `sap_ai_core`

## 当前 canonical 配置示例

### 单 adapter provider

```toml
[providers.openai]
default_model = "gpt-4.1"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
default_model = "gpt-4.1"
```

### 多 adapter provider，共享同一份 auth

```toml
[providers.shared]
default_model = "fast"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.adapters.openai]
default_model = "gpt-4.1"

[providers.shared.adapters.openai.models.fast]
target_model = "gpt-4.1-mini"

[providers.shared.adapters.anthropic]
default_model = "claude-sonnet-4"

[providers.shared.adapters.anthropic.models.coder]
target_model = "claude-sonnet-4"
```

### Copilot provider，共享一份 OAuth，同时暴露 OpenAI-style 和 Anthropic-style 模型

```toml
[providers."github-copilot"]
default_model = "gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "copilot-refresh", access = "copilot-access", expires_at_ms = 4102444800000 }

[providers."github-copilot".adapters.openai]
default_model = "gpt-4o-mini"

[providers."github-copilot".adapters.openai.models."gpt-4o-mini"]
target_model = "gpt-4o-mini"

[providers."github-copilot".adapters.anthropic]
default_model = "claude-sonnet-4"

[providers."github-copilot".adapters.anthropic.models.claude]
target_model = "claude-sonnet-4"
```

### ChatGPT Codex provider

```toml
[providers.openai_chatgpt]
default_model = "gpt-5.3-codex"

[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.openai_chatgpt.adapters.openai]
backend = "chatgpt_codex"
default_model = "gpt-5.3-codex"
```

### SAP AI Core provider

```toml
[providers.sap]
default_model = "anthropic/claude-sonnet-4"

[providers.sap.auth]
mode = "sap_ai_core"
base_url = "https://api.example.com/v2"
service_key_env = "AICORE_SERVICE_KEY"

[providers.sap.adapters.openai_compatible]
default_model = "anthropic/claude-sonnet-4"
```

## 关键约束

- 一个 provider 可以有多个 adapters
- 一个 adapter 下可以有多个 models
- 多 adapter provider 必须显式声明每个 adapter 的 `models`
- 同一个可见 model id 不能在多个 adapters 下重复声明
- 多 adapter provider 的 `default_model` 必须指向一个已声明的 routed model
- 同一个 provider 下只保留一个同 kind adapter

## 刷新、缓存和 provider metadata

provider-local auth 的运行时行为现在是：

- `ManagedCredential` 直接持有 provider 自己的认证材料
- refresh 直接更新 provider 内部 `AuthData`
- prompt-cache scope 与 provider auth 绑定
- OpenAI / Copilot 这类需要 `account_id` / `enterprise_url` 的 provider，直接从 provider-local auth 读 metadata

这意味着：

- 同一个 provider 下多个 adapters 可以自然共享 refresh 后的认证状态
- 不同 providers 之间不会共享认证状态

## REST / Studio 对外的 auth 对象

对外暴露的 auth id 就是 `provider_id` 本身。

例如：

- `openai_chatgpt`
- `github-copilot`
- `gitlab`

登录、刷新和 UI 展示都围绕 provider 自身进行，不再额外映射到 provider 之外的共享认证对象。

## 配置迁移方向

如果你以前的思维模型还是：

- provider 根上直接挂 `base_url` / `api_key`
- model 直接挂在 provider 根上
- auth/credential 是 provider 之外的共享对象
- 同一个 provider 里复制多个同类 adapter

那迁移目标应该统一变成：

- provider 只做逻辑入口
- auth 统一收进 `provider.auth`
- 协议实现统一收进 `provider.adapters.<adapter_kind>`
- model routes 统一挂到 adapter 下
- 如果多个后端共享同一份认证，就放到同一个 provider 的多个 adapters
- 如果需要不同认证，就拆成不同 provider

不要再设计：

- provider 之外的共享 auth 路由
- `credential_provider_id`
- 同一个 provider 下两个同类 adapter 只是为了挂不同 credential

## 进一步阅读

- [配置说明](configuration.md)
