# Provider Auth 与 Credential 关系说明

本文说明 Agena 当前的 canonical provider auth 结构。

核心结论只有一条：

- credential 属于某一个 provider 的 `auth`。
- credential 不再是一个可被多个 provider 共享的独立全局主键。
- provider 可以包含多个 adapters；这些 adapters 共享同一个 provider-level auth。

相关实现主要位于：

- `crates/agena/src/config/raw.rs`
- `crates/agena/src/config/types.rs`
- `crates/agena/src/config/registry.rs`
- `crates/agena/src/provider/credential.rs`
- `crates/agena/src/provider/openai.rs`
- `crates/agena/src/provider/copilot.rs`
- `crates/agena-api-server/src/rest.rs`

## 1. 四个概念

- provider：一个逻辑模型入口，对应 `[providers.<provider_id>]`。
- adapter：provider 下的某个真实后端，例如 OpenAI API、ChatGPT Codex、GitLab Duo、Copilot。
- auth：provider 级认证策略，对应 `[providers.<provider_id>.auth]`。
- credential：认证材料本身，直接挂在 provider 的 `auth` 里，或来自 `secret` / `secret_env`。

结构关系现在是：

```text
config.toml
  └── [providers.<provider_id>]
        ├── default_model
        ├── auth
        └── adapters.<adapter_id>
              └── kind / endpoint / models

运行时：
  provider 决定逻辑入口
  adapter 决定真实后端
  provider.auth 决定这整个 provider 用什么 credential
```

最重要的语义变化：

- `provider_id` 是公开主键。
- `adapter_id` 只是 provider 内部路由名。
- 不再有 canonical 的 `credential_provider_id`。
- REST / Studio 对外也以 `provider_id` 作为 auth 对象，而不是再映射到共享 credential id。

## 2. `provider.auth` 支持哪些模式

| mode | 主要字段 | 典型用途 |
| --- | --- | --- |
| `none` | 无 | `ollama` |
| `secret` | `secret`、`secret_env`、`credential` | OpenAI、Anthropic、Gemini、GitLab direct token、Copilot OAuth、ChatGPT Codex OAuth |
| `bedrock_sigv4` | `profile`、`access_key_id`、`secret_access_key`、`session_token` | AWS SigV4 |
| `google_adc` | 无 | Google ADC |
| `sap_ai_core` | `secret.*`、`credential`、`service_key_env` | SAP AI Core |

其中最常见的是 `secret` 模式。它不是只表示“API key 明文”，而是统一表示“这个 provider 用 secret-like credential 认证”，来源可以是：

1. `secret`
2. `secret_env`
3. `credential`
4. provider-specific fallback

## 3. `secret` 模式里的 `credential`

`credential` 的类型是 `AuthData`。它直接内嵌在 provider 配置里，不再单独引用全局 credential id。

常见形态：

### 3.1 API key

```toml
[providers.openai.auth]
credential = { type = "api", key = "sk-example" }
```

### 3.2 OAuth

```toml
[providers.openai_chatgpt.auth]
credential = { type = "oauth", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-123" }
```

### 3.3 Copilot Enterprise OAuth

```toml
[providers.copilot.auth]
credential = { type = "oauth", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, enterprise_url = "github.example.com" }
```

`OAuth` payload 里两个 metadata 很重要：

- `account_id`：OpenAI ChatGPT Codex 请求头和 prompt-cache scope 会用到。
- `enterprise_url`：Copilot enterprise base URL 解析和 prompt-cache scope 会用到。

## 4. 运行时怎么解析 credential

多数 provider 的解析顺序如下：

1. 配置里的 `secret`
2. 配置里的 `secret_env`
3. 配置里的 inline `credential`
4. provider-specific fallback

几个特殊规则：

- `ollama` 必须用 `mode = "none"`。
- `google_vertex` 没有静态 token 时可用 `mode = "google_adc"`。
- `amazon_bedrock` 没有 bearer secret 时可用 `mode = "bedrock_sigv4"`。
- `sap_ai_core` 在 direct secret 和 inline credential 之外，还能走 `service_key_env`。
- `copilot` 只接受 provider-local OAuth credential，不接受直接 `secret` / `secret_env`。
- OpenAI `backend = "chatgpt_codex"` 只接受 provider-local OAuth credential，不接受直接 `secret` / `secret_env`。
- GitLab 如果此刻存在 direct secret，会走 direct token；否则走 provider-local OAuth credential。

## 5. 刷新和缓存由谁负责

现在 credential 的刷新、缓存 scope 和 provider metadata 由 provider 自己处理：

- `ManagedCredential::auth_data_shared(...)` 持有 provider 内部的 `AuthData`。
- token refresh 会直接更新这份 provider-local `AuthData`。
- OpenAI / Copilot 不再额外依赖全局 auth store 元数据；这些字段直接跟着 provider-local credential 走。

这意味着：

- 一个 provider 的 OAuth refresh 生命周期天然绑定在这个 provider 上。
- 同一个 provider 下多个 adapters 可以共享这份 credential。
- 不同 providers 不再通过 `credential_provider_id` 共享一套身份。

## 6. REST / Studio 对外暴露哪个 auth id

现在对外暴露的 auth id 就是 `provider_id` 本身。

例如：

- `openai_chatgpt` 的登录对象是 `openai_chatgpt`
- `gitlab-self` 的登录对象是 `gitlab-self`
- `copilot-enterprise` 的登录对象是 `copilot-enterprise`

不会再出现“两个不同 provider 对外都映射到同一个共享 credential id”这种 canonical 语义。

## 7. 常见配置示例

### 7.1 单 adapter，直接 env secret

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

### 7.2 一个 provider 共享一套 OAuth，同时路由 API 和 ChatGPT Codex

```toml
[providers.shared]
default_model = "fast"

[providers.shared.auth]
credential = { type = "oauth", refresh = "refresh-token", access = "access-token", expires_at_ms = 4102444800000, account_id = "acct-shared" }

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

### 7.3 两个 provider 各自持有独立 credential

```toml
[providers.primary]
default_model = "gpt-4.1"

[providers.primary.auth]
credential = { type = "api", key = "sk-primary" }

[providers.primary.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1"

[providers.secondary]
default_model = "gpt-4.1-mini"

[providers.secondary.auth]
credential = { type = "api", key = "sk-secondary" }

[providers.secondary.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
```

### 7.4 GitLab OAuth

```toml
[providers.gitlab]
default_model = "claude-sonnet-4-5"

[providers.gitlab.auth]
credential = { type = "oauth", refresh = "gitlab-refresh", access = "gitlab-access", expires_at_ms = 4102444800000 }

[providers.gitlab.adapters.duo]
kind = "gitlab"
instance_url = "https://gitlab.com"
ai_gateway_url = "https://cloud.gitlab.com"
default_model = "claude-sonnet-4-5"
```

## 8. 迁移建议

如果你之前依赖这些旧字段：

- `credential_provider_id`
- root-level `auth_provider_id`
- 多个 provider 共享同一条外部 credential

现在应当迁移为：

- 每个 provider 把自己的 credential 写到 `[providers.<id>.auth]`
- 同一个 provider 下需要共享 credential 时，只共享给它自己的 adapters
- 不再把 credential 作为 provider 之外的独立路由层

旧的 flat provider 字段仍然只是输入兼容层；新的配置和新的运行时模型都应以 provider-local `auth.credential` 为准。
