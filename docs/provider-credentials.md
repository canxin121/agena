# Provider Auth 与 Credential 关系说明

本文说明 Agena 里 provider、adapter、credential、auth store 之间的关系。当前的 canonical 配置结构是 `provider.auth + provider.adapters`；旧的 flat provider 字段仍兼容，但只是输入兼容层。

相关实现主要位于：

- `crates/agena/src/config/raw.rs`
- `crates/agena/src/config/types.rs`
- `crates/agena/src/config/registry.rs`
- `crates/agena/src/provider/auth/{types,store,manager}.rs`
- `crates/agena-api-server/src/rest.rs`

结构设计和迁移背景见 [Provider Auth + Adapters 重构说明](provider-auth-adapters.md)，字段列表见 [配置说明](configuration.md)。

## 1. 先分清四个概念

- provider：一个逻辑模型入口，来自 `[providers.<provider_id>]`。
- adapter：provider 下的一个真实后端，例如 OpenAI API、ChatGPT Codex、GitLab Duo、Vertex。
- auth：provider 级认证策略，定义在 `[providers.<provider_id>.auth]`，供同一个 provider 下的 adapters 共享。
- credential：认证材料本身，可能来自 inline secret、环境变量、auth store，或者 provider 特有 fallback。

关系大致如下：

```text
config.toml
  └── [providers.<provider_id>]
        ├── default_model
        ├── auth
        └── adapters.<adapter_id>
              └── kind / endpoint / model routes

auth store
  └── <credential_provider_id> -> AuthData
        └── Api | OAuth | WellKnown

运行时：
  provider 决定“对外暴露哪个逻辑入口”
  adapter 决定“连哪个后端”
  auth / credential 决定“拿什么身份去连”
```

## 2. 三个常见 id

### 2.1 `provider_id`

`provider_id` 是 `[providers.<id>]` 的 `<id>`。它会出现在：

- model ref，例如 `openai/gpt-4.1-mini`
- CLI 命令，例如 `agena provider models openai`
- HTTP API，例如 `/api/v1/providers/openai/models`
- Studio 的 provider 列表

`provider_id` 只是逻辑名字，不等于 `kind`。

### 2.2 `adapter_id`

`adapter_id` 是 `[providers.<id>.adapters.<adapter_id>]` 的 `<adapter_id>`。它只在 provider 内部用来区分不同后端，不会成为公开的 provider 主键。

典型例子：

- `api`
- `codex`
- `duo`
- `aws`

### 2.3 `credential_provider_id`

`credential_provider_id` 表示 runtime 去 auth store 里读哪一个 credential。

在旧配置里，这个概念常叫 `auth_provider_id`。现在 canonical 字段名是：

```toml
[providers.openai.auth]
credential_provider_id = "openai"
```

多个 provider 完全可以共享同一个 `credential_provider_id`。

## 3. `provider.auth` 支持哪些模式

| mode | 主要字段 | 典型用途 | 是否依赖 auth store |
| --- | --- | --- | --- |
| `none` | 无 | `ollama` | 否 |
| `secret` | `secret`、`secret_env`、`credential_provider_id` | OpenAI、Anthropic、Gemini、OpenRouter、GitLab direct token、Vertex 静态 token、Bedrock bearer token | 可选 |
| `bedrock_sigv4` | `profile`、`access_key_id`、`secret_access_key`、`session_token` | AWS SigV4 | 否 |
| `google_adc` | 无 | Google ADC | 否 |
| `sap_ai_core` | `secret.*`、`credential_provider_id`、`service_key_env` | SAP AI Core | 可选 |

实现层面的约束：

- `none` 只支持 `ollama` adapters。
- `google_adc` 只支持 `google_vertex` adapters。
- `bedrock_sigv4` 只支持 `amazon_bedrock` adapters。
- `sap_ai_core` 只支持 `sap_ai_core` adapters。
- `secret` 不支持 `ollama`。
- `copilot` 和 OpenAI `backend = "chatgpt_codex"` 虽然也走 `secret` 模式，但只允许 auth-store credential，不允许直接 `secret` / `secret_env`。

## 4. `secret` 模式怎么找 credential

多数 provider 可以按下面顺序理解：

1. 直接写在配置里的 `secret`
2. 配置里声明的 `secret_env`
3. auth store 中 `credential_provider_id` 对应的 credential
4. provider 特有 fallback

几个重要细节：

- 对大部分 HTTP provider 来说，如果只配置了 `secret_env`，即使当前进程启动时环境变量还没值，provider registry 也可能先构建成功，真正发请求时再报缺少环境变量。
- GitLab 不一样。它会在构建 registry 时判断“当前是否有 direct secret”。如果 `secret` 非空，或者 `secret_env` 指向的环境变量此刻非空，就走 direct token；否则改走 auth store OAuth。
- `copilot` 和 `chatgpt_codex` 只接受 auth store credential，不接受 direct secret。
- `google_vertex` 没有静态 token时可以走 ADC。
- `amazon_bedrock` 没有 bearer secret时可以走 SigV4。
- `sap_ai_core` 在 direct secret 和 auth store 之外，还可以用 `service_key_env` 指向的 service key。

## 5. auth store 里到底存什么

auth store 里的 credential 统一建模为 `AuthData`，当前主要有三类：

| 类型 | 结构 | 典型用途 |
| --- | --- | --- |
| `Api` | `{ key }` | 普通 API key / bearer token |
| `OAuth` | `{ refresh, access, expires_at_ms, account_id?, enterprise_url? }` | OpenAI、GitLab、GitHub Copilot 登录 |
| `WellKnown` | `{ key, token }` | 内部辅助状态 |

最重要的内部 `WellKnown` 条目是 `gitlab-instance`：

- GitLab 浏览器登录成功后，runtime 会保存 `gitlab` 对应的 OAuth token。
- 同时会额外保存 `gitlab-instance`，用来记住实例地址。
- `gitlab-instance` 不会暴露给公开的 auth provider API。

auth store 后端仍由 `[auth]` 控制：

- `file`
- `keyring`
- `auto`

默认 `auto` 优先 OS keyring，不可用时回退到 `~/.agena/auth.json`。

## 6. REST / Studio 对外暴露哪个 auth provider id

Studio 和 REST auth API 需要决定“对外显示哪些 auth providers”。现在这件事是从解析后的 `provider.auth` 推导出来的，不再直接依赖旧的 flat `auth_provider_id` 字段。

规则是：

1. 如果 provider 当前配置了 direct `secret`，或者 `secret_env` 指向的环境变量此刻有值，那么对外 auth provider id 使用 `provider_id` 自己。
2. 否则如果配置了 `credential_provider_id`，就对外暴露那个 id。
3. 否则回退到 `provider_id`。
4. `google_adc`、`bedrock_sigv4`、`none` 这类不读 auth store 的模式，不会暴露为 auth provider。
5. 内部条目例如 `gitlab-instance` 仍然会被隐藏。

这个规则解释了几个常见现象：

- 一个共享 credential 的多 provider 配置，只要都写 `credential_provider_id = "openai"`，UI / REST 里就会围绕 `openai` 这套 credential 工作。
- `gitlab-self` 如果配置了 direct `secret_env = "GITLAB_TOKEN"`，UI 里看到的 auth provider id 会是 `gitlab-self`，因为它现在不再依赖共享的 OAuth。
- `gitlab-self` 如果没有 direct secret，但配置了 `credential_provider_id = "gitlab"`，UI 里会复用 `gitlab` 这套 OAuth。

## 7. 各 adapter kind 的常见 auth 方式

| adapter kind | 常见 auth mode | 备注 |
| --- | --- | --- |
| `openai` | `secret` | `backend = "api"` 常用 direct API key；`backend = "chatgpt_codex"` 只支持 auth-store OpenAI OAuth |
| `openai_compatible` | `secret` | OpenRouter、LM Studio、vLLM、Groq 等通常都在这里 |
| `anthropic` | `secret` | 默认 header 是 `x-api-key` |
| `gemini` | `secret` | 常见是 API key；运行时会按 Gemini 规则放到请求里 |
| `gitlab` | `secret` | 既支持 direct token，也支持 OAuth；浏览器登录会额外写 `gitlab-instance` |
| `copilot` | `secret` | 但只支持 auth-store OAuth / refresh-access credential |
| `amazon_bedrock` | `secret` 或 `bedrock_sigv4` | bearer 和 SigV4 二选一 |
| `google_vertex` | `secret` 或 `google_adc` | 静态 token 或 ADC 二选一 |
| `sap_ai_core` | `sap_ai_core` | direct secret、auth store、service key 都可能参与 |
| `cloudflare_ai_gateway` | `secret` | model id 通常是 `provider/model` |
| `ollama` | `none` | 本地 endpoint，无 credential |

## 8. 配置示例

### 8.1 单 adapter，直接 API key / env

```toml
[providers.openai]
default_model = "gpt-4.1"

[providers.openai.auth]
secret_env = "OPENAI_API_KEY"

[providers.openai.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1"
```

### 8.2 多个 provider 共享一套 OpenAI credential

```toml
[providers.primary]
default_model = "gpt-4.1"

[providers.primary.auth]
credential_provider_id = "shared-openai"

[providers.primary.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1"

[providers.secondary]
default_model = "gpt-4.1-mini"

[providers.secondary.auth]
credential_provider_id = "shared-openai"

[providers.secondary.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
```

### 8.3 一个 provider 共享 OpenAI credential，同时路由 API 和 ChatGPT Codex

```toml
[providers.shared]
default_model = "fast"

[providers.shared.auth]
credential_provider_id = "openai"

[providers.shared.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1"

[providers.shared.adapters.api.models.fast]
target_model = "gpt-4.1-mini"

[providers.shared.adapters.codex]
kind = "openai"
backend = "chatgpt_codex"
default_model = "gpt-5-codex"

[providers.shared.adapters.codex.models.coder]
target_model = "gpt-5-codex"
```

### 8.4 GitLab 复用默认 OAuth

```toml
[providers.gitlab]
default_model = "claude-sonnet-4-5"

[providers.gitlab.auth]
credential_provider_id = "gitlab"

[providers.gitlab.adapters.duo]
kind = "gitlab"
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
```

### 8.5 GitLab 自定义 provider id，但复用同一套 OAuth

```toml
[providers.gitlab-self]
default_model = "claude-sonnet-4-5"

[providers.gitlab-self.auth]
credential_provider_id = "gitlab"

[providers.gitlab-self.adapters.duo]
kind = "gitlab"
instance_url = "https://gitlab.example.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
```

### 8.6 Bedrock SigV4

```toml
[providers.bedrock]
default_model = "anthropic.claude-3-5-sonnet-20240620-v1:0"

[providers.bedrock.auth]
mode = "bedrock_sigv4"
profile = "prod"

[providers.bedrock.adapters.aws]
kind = "amazon_bedrock"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com"
default_model = "anthropic.claude-3-5-sonnet-20240620-v1:0"
region = "us-east-1"
```

### 8.7 Vertex ADC

```toml
[providers.vertex]
default_model = "google/gemini-2.5-flash"

[providers.vertex.auth]
mode = "google_adc"

[providers.vertex.adapters.api]
kind = "google_vertex"
base_url = "https://us-central1-aiplatform.googleapis.com/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
default_model = "google/gemini-2.5-flash"
```

## 9. 兼容性与迁移

旧的 flat provider 配置仍然兼容：

- 根节点的 `kind`、`base_url`、`api_key`、`api_key_env`、`auth_provider_id` 会被 lower 到新的 `provider.auth` 或隐式 `default` adapter。
- 根节点的 `[providers.<id>.models]` 会被 lower 到新的 routed model 表。

但一旦你开始使用 `providers.<id>.adapters`，就必须把这些 legacy 字段一起迁移进去，不要在同一个 provider 里混用两套结构。

如果你只想记一句话：

- `provider_id` 决定“这个逻辑入口叫什么”
- `adapter` 决定“真正连哪个后端”
- `credential_provider_id` 决定“去 auth store 的哪一格拿身份”
