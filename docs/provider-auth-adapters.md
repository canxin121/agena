# Provider / Auth / Adapter 架构

本文说明当前 provider 子系统的 canonical 结构，以及 provider、auth、adapter、model 四层各自负责什么。

## 总览

当前配置结构是：

```text
default
├── provider
├── adapter
├── model
└── agent

provider
├── enabled
├── default_model = "<adapter>/<model>"  # optional provider-local route default
├── auth
└── adapters
    └── <adapter>
        ├── enabled
        ├── protocol/options
        └── models
            └── <real-upstream-model-id>
                ├── enabled
                └── metadata / capabilities / variants
```

对应 TOML：

```toml
[default]
provider = "openai"
adapter = "openai"
model = "gpt-5"
agent = "build"

[providers.openai]
enabled = true

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5"]
enabled = true
```

关键约束：

- 全局默认 provider/adapter/model/agent 写在 `[default]`
- `providers.<id>.default_adapter` 和 `providers.<id>.default_model` 是 provider-local 默认选择；`default_model` 必须是真实上游 model id
- adapter 不再有 `default_model`
- model key 就是真实上游 model id，不再有 `target_model`
- provider / adapter / model 三层都支持 `enabled`
- 运行时模型选择由 `provider_id`、`adapter_id`、`model_id` 三个字段共同决定，不使用三段字符串编码

## 四层职责

### provider

`provider` 是 Agena 对外暴露的逻辑入口。

它负责：

- 稳定的 `provider_id`
- provider 级 `auth`
- 聚合一个或多个 adapters
- 暴露 provider 默认模型
- 对外提供统一的模型命名空间

CLI、HTTP API、Studio、session 持久化、model ref 都围绕 `provider_id` 工作。

### auth

`provider.auth` 只负责认证与连接身份，不负责模型路由。

它负责：

- shared endpoint / token / OAuth / ADC / SigV4 / service key 等认证来源
- refresh 生命周期
- provider-local metadata，例如 Copilot enterprise host、ChatGPT account id
- 给同一个 provider 下所有 adapters 共享认证上下文

当一个 auth 网关同时暴露多种协议时，`auth.base_url` 表示用户填写的共享入口，不一定是某个 adapter 的最终协议 base。运行时会根据 `auth.endpoint_layout` 和 adapter kind 自动派生协议后缀。

`auth` 不再拆成独立 connection 对象，也不放在 adapter 上。

### adapter

`adapter` 代表真实协议实现。

它负责：

- 选择 wire protocol
- 请求/流式协议细节
- provider-specific transport 选项
- 暴露该 adapter 下的模型集合

典型 adapter：

- `openai`
- `anthropic`
- `gemini`
- `gitlab`
- `amazon_bedrock`
- `ollama`

一个 provider 下可以有多个 adapter，但同 kind 只保留一个 canonical adapter。

### model

`providers.<id>.adapters.<adapter>.models."<model-id>"` 表示该 adapter 下一个真实上游模型的配置节点。

它负责：

- 开关控制：`enabled`
- metadata patch
- capability patch
- variants

这里的 key 就是真实上游 model id，例如：

- `"gpt-5"`
- `"claude-sonnet-4"`
- `"google/gemini-2.5-flash"`
- `"anthropic.claude-3-7-sonnet-20250219-v1:0"`

不再有 `target_model`。如果你写了 `models."gpt-5"`，那它路由到的就是上游的 `gpt-5`。

## 命名与路由

provider 内部可见模型名统一是：

```text
<adapter>/<model>
```

例如：

- `openai/gpt-5`
- `anthropic/claude-sonnet-4`
- `gemini/google/gemini-2.5-flash`

运行时选择模型时始终拆成三个字段：

- 全局默认字段：`[default] provider = "openai"`, `adapter = "openai"`, `model = "gpt-5"`
- provider-local 默认选择：`default_adapter = "openai"`, `default_model = "gpt-5"`
- 真实包含 `/` 的模型名保留在 `model`/`model_id` 字段里，例如 `model = "google/gemini-2.5-flash"`

## enabled 语义

三层都支持 `enabled`：

```toml
[providers.shared]
enabled = true
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.shared.adapters.openai]
enabled = true

[providers.shared.adapters.openai.models."gpt-4.1-mini"]
enabled = true
```

行为：

- provider disabled：整个 provider 不注册
- adapter disabled：该 adapter 不可选，也不会暴露其模型
- model disabled：该 adapter 下的具体 model 不可选

这三个开关都用于快速下线 provider、adapter 或单个模型，而不需要删配置。

默认值：

- provider：默认 `enabled = true`
- adapter：默认 `enabled = false`
- model：默认 `enabled = true`

因此只要你希望某个 adapter 真正对外提供模型，建议显式写上 `enabled = true`。

## auth 模式

`provider.auth.mode` 可选值：

```text
none
api
credential
bedrock_sigv4
google_adc
sap_ai_core
```

### `none`

用于本地无认证 provider，例如 `ollama`。

### `api`

用于显式 endpoint + API key：

```toml
[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
```

字段：

- `base_url`
- `endpoint_layout`
- `api_key`
- `api_key_env`

`endpoint_layout` 可选值：

- `auto`
- `direct`
- `protocol_root`
- `provider_routed`

含义：

- `direct`：`base_url` 已经是当前 provider 直接要使用的最终协议 base
- `protocol_root`：把 `base_url` 当成共享网关入口，为不同 adapter 自动派生 `/v1` 或 `/v1beta`
- `provider_routed`：把 `base_url` 当成 provider-routed 共享网关入口，为不同 adapter 自动派生 `/api/provider/<provider>/...`
- `auto`：运行时根据填写的 URL 形状自动推断，默认值就是它

`auto` 的判断大致是：

- `https://api.cxits.cn` -> 按 `direct` 不改动
- `https://api.cxits.cn/v1` -> 按 `protocol_root`
- `https://api.cxits.cn/v1/messages` -> 按 `protocol_root`
- `https://api.cxits.cn/v1beta/models/gemini-2.5-pro:generateContent` -> 按 `protocol_root`
- `https://api.cxits.cn/api/provider/openai/v1` -> 按 `provider_routed`

### `credential`

用于登录态 / OAuth / refresh token：

```toml
[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "...", access = "...", expires_at_ms = 4102444800000 }
```

字段：

- `issuer`
- `credential`

注意：credential 模式下不接受 `base_url`、`api_key`、`api_key_env`。

`credential` 必须带 issuer 信息，这样运行时才能知道这份 credential 是谁的，例如：

- `openai_chatgpt`
- `github_copilot`
- `gitlab`
- `atomgit`

### `bedrock_sigv4`

用于 AWS 原生签名：

```toml
[providers.bedrock.auth]
mode = "bedrock_sigv4"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com"
region = "us-east-1"
profile = "prod"
```

### `google_adc`

用于 Vertex / Google ADC。和 `api` 一样，它也需要一个共享入口的 `base_url`；区别只是凭证来源来自 Google ADC，而不是 API key。

```toml
[providers.vertex.auth]
mode = "google_adc"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
endpoint_layout = "direct"
```

### `sap_ai_core`

用于 SAP AI Core。

## adapter 与 auth 的关系

auth 决定身份来源；adapter 决定协议。

同一个 auth 可以服务多个 adapter，只要运行时组合合法。

例如：

- `github_copilot` credential 可以配 `openai` adapter
- `github_copilot` credential 也可以配 `anthropic` adapter
- `atomgit` credential 可以配 `openai` adapter，运行时使用 AtomGit 的 OpenAI-compatible gateway
- `openai_chatgpt` credential 只适合 `openai` adapter 且 `backend = "chatgpt_codex"`
- `bedrock_sigv4` 只适合 `amazon_bedrock`
- `sap_ai_core` 只适合 `openai`

如果配置了错误组合，运行时报配置错误即可，不再为旧结构做兼容转换。

## 常见示例

### OpenAI API

```toml
[default]
provider = "openai"
adapter = "openai"
model = "gpt-5"
agent = "build"

[providers.openai]
default_adapter = "openai"
default_model = "gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5"]
enabled = true
```

### ChatGPT Codex OAuth

```toml
[providers.chatgpt]
default_adapter = "openai"
default_model = "gpt-5.3-codex"

[providers.chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "...", access = "...", expires_at_ms = 4102444800000, account_id = "acct-123" }

[providers.chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"

[providers.chatgpt.adapters.openai.models."gpt-5.3-codex"]
enabled = true
```

### GitHub Copilot OpenAI

```toml
[providers."github-copilot"]
default_adapter = "openai"
default_model = "gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "...", access = "...", expires_at_ms = 4102444800000 }

[providers."github-copilot".adapters.openai]
enabled = true

[providers."github-copilot".adapters.openai.models."gpt-4o-mini"]
enabled = true
```

### AtomGit OAuth

```toml
[providers.atomgit]
default_adapter = "openai"
default_model = "Kimi-K2-Instruct"

[providers.atomgit.auth]
mode = "credential"
issuer = "atomgit"
credential = { type = "oauth", issuer = "atomgit", refresh = "...", access = "...", expires_at_ms = 4102444800000, account_id = "atomgit-user" }

[providers.atomgit.adapters.openai]
enabled = true

[providers.atomgit.adapters.openai.models."Kimi-K2-Instruct"]
enabled = true
```

### GitHub Copilot Anthropic

```toml
[providers."github-copilot-claude"]
default_adapter = "anthropic"
default_model = "claude-sonnet-4"

[providers."github-copilot-claude".auth]
mode = "credential"
issuer = "github_copilot"
credential = { type = "oauth", issuer = "github_copilot", refresh = "...", access = "...", expires_at_ms = 4102444800000 }

[providers."github-copilot-claude".adapters.anthropic]
enabled = true
auth_header = "authorization"
auth_scheme = "Bearer"
extra_beta_header = "interleaved-thinking-2025-05-14"

[providers."github-copilot-claude".adapters.anthropic.models."claude-sonnet-4"]
enabled = true
```

### Shared Multi-Adapter Provider

```toml
[providers.shared]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com"
endpoint_layout = "protocol_root"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.adapters.openai]
enabled = true

[providers.shared.adapters.openai.models."gpt-4.1-mini"]
enabled = true

[providers.shared.adapters.openai.models."gpt-4.1-mini".variants.deep]
thinking = { type = "effort", effort = "high" }

[providers.shared.adapters.anthropic]
enabled = true

[providers.shared.adapters.anthropic.models."claude-sonnet-4"]
enabled = true
```

这里：

- `openai` 会自动派生到 `https://gateway.example.com/v1`
- `anthropic` 会自动派生到 `https://gateway.example.com/v1`
- `gemini` 如果启用，会自动派生到 `https://gateway.example.com/v1beta`

### Provider-Routed Shared Gateway

```toml
[providers.provider_gateway]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.provider_gateway.auth]
mode = "api"
base_url = "https://api.cxits.cn/api/provider/openai/v1"
endpoint_layout = "provider_routed"
api_key_env = "CX_API_KEY"

[providers.provider_gateway.adapters.openai]
enabled = true

[providers.provider_gateway.adapters.openai.models."gpt-4.1-mini"]
enabled = true

[providers.provider_gateway.adapters.anthropic]
enabled = true

[providers.provider_gateway.adapters.anthropic.models."claude-sonnet-4"]
enabled = true

[providers.provider_gateway.adapters.gemini]
enabled = true

[providers.provider_gateway.adapters.gemini.models."gemini-2.5-pro"]
enabled = true
```

即使你填的是 `.../api/provider/openai/v1`，运行时也会先回退到共享 gateway root，再自动为其他 adapter 派生：

- `openai` -> `/api/provider/openai/v1`
- `anthropic` -> `/api/provider/anthropic/v1`
- `gemini` -> `/api/provider/google/v1beta`

如果你填的是完整协议 endpoint，运行时也会先把它收敛成共享入口，再按 adapter 重新拼：

- `https://api.cxits.cn/v1/messages` 会收敛成 `https://api.cxits.cn`
- `https://api.cxits.cn/v1beta/models/gemini-2.5-pro:generateContent` 会收敛成 `https://api.cxits.cn`

### Amazon Bedrock SigV4

```toml
[providers.bedrock]
default_adapter = "amazon_bedrock"
default_model = "anthropic.claude-3-7-sonnet-20250219-v1:0"

[providers.bedrock.auth]
mode = "bedrock_sigv4"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com"
region = "us-east-1"
profile = "prod"

[providers.bedrock.adapters.amazon_bedrock]
enabled = true

[providers.bedrock.adapters.amazon_bedrock.models."anthropic.claude-3-7-sonnet-20250219-v1:0"]
enabled = true
```

## 迁移后的结论

现在 provider 相关配置应理解为：

- provider 是逻辑入口
- auth 只管身份与认证
- adapter 是协议实现
- model key 是真实上游模型名
- provider 默认模型由 `default_adapter` 和 `default_model` 分别指定
- 外部运行请求也应分别传 `provider_id`、`adapter_id`、`model_id`
