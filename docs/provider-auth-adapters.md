# Provider Auth + Adapters 重构说明

本文说明当前 `provider` / `auth` / `adapter` 结构的设计目标，以及为什么 credential 现在必须归属到 provider 自己。

## 目标

旧模型里有两个根本问题：

1. `provider` 同时暴露 `base_url`、`api_key`、`api_key_env` 等认证与路由字段，语义混杂。
2. credential 通过全局 id 路由，导致“provider 的 credential”其实并不真的属于 provider。

新的结构改成：

- `provider` 是逻辑入口。
- `provider.auth` 是这个逻辑入口自己的认证配置。
- `provider.adapters` 是这个逻辑入口下的多个真实后端。
- model routes 挂在 adapter 下，再被扁平化成 provider 的可见模型表。

结构如下：

```text
provider
├── default_model
├── auth
└── adapters
    ├── <adapter_id>
    │   ├── kind
    │   ├── endpoint/options
    │   └── models
    │       └── <visible_model_id>
    │           ├── target_model
    │           └── variants / metadata / capabilities
    └── ...
```

## 关键语义

- 一个 provider 可以包含多个 adapters。
- 一个 adapter 可以暴露多个 models。
- 一个 model 可以有多个 variants。
- credential 只属于一个 provider。
- 同一个 provider 下的 adapters 共享该 provider 的 auth。
- 不同 providers 不再通过 `credential_provider_id` 共享同一条身份。

## 当前配置模型

解析后的 `ResolvedProviderConfig` 现在由这些部分组成：

- `default_model`
- `auth`
- `adapters`
- `models`

其中：

- `auth` 是 provider-level 认证配置。
- `adapters` 保存真实后端定义。
- `models` 是 provider 对外暴露的路由表，记录“这个可见模型该走哪个 adapter，对应哪个上游 model”。

## auth 的位置为什么必须在 provider 上

因为实际运行时需要共享 auth 的不是“全局多个 provider”，而是“同一个 provider 下的多个 adapters”。

典型例子：

- 一个 provider 同时暴露 OpenAI API adapter 和 ChatGPT Codex adapter。
- 它们都属于同一个逻辑入口。
- 它们应该共享同一份 provider-local credential。
- 这个 credential 的 refresh、account metadata、enterprise metadata 也应该跟着这个 provider 走。

把 credential 放在 `provider.auth` 下之后：

- 结构上更自然。
- refresh 生命周期和 provider 实例绑定。
- prompt-cache scope 可以直接跟 provider auth 对齐。
- 不再需要把 auth 再映射到一个外部共享 id。

## 运行时落地

运行时现在围绕两层来工作：

### 1. `MultiAdapterProvider`

- 对外仍然暴露一个 provider id。
- 对内持有多个真实 adapter provider。
- 根据 model route 把请求转发到对应 adapter。
- 单 adapter provider 支持 passthrough。

### 2. provider-managed credential

- `ManagedCredential` 可以直接持有 provider-local `AuthData`。
- token refresh 会更新 provider 自己的那份 `AuthData`。
- OpenAI / Copilot 这种还要读取 `account_id` / `enterprise_url` 的 provider，不再需要额外查询全局 auth store 元数据。

## canonical 配置示例

### 单 adapter

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

### 多 adapter，共享同一个 provider-local OAuth

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

## 兼容性说明

旧的 flat provider 配置仍然是输入兼容层：

- root-level `kind`
- `base_url`
- `api_key` / `api_key_env`

这些字段会被 lower 到新的 `provider.auth + provider.adapters` 结构。

但 canonical 写法已经变了：

- 新配置不要再写 `credential_provider_id`
- 新配置不要再把 credential 当成 provider 外部的共享对象
- 新配置应该直接写到 `[providers.<id>.auth]`

## 约束

- 多 adapter provider 必须显式声明每个 adapter 的 `models`。
- 同一个可见 model id 不能在多个 adapters 下重复声明。
- `ollama` 只能配 `auth.mode = "none"`。
- `copilot` 和 OpenAI `backend = "chatgpt_codex"` 只接受 provider-local OAuth credential，不接受直接 `secret` / `secret_env`。

## 迁移建议

如果你以前依赖“多个 provider 共用一个 `credential_provider_id`”，迁移时应当先问自己：

- 这是不是其实应该是一个 provider 下的多个 adapters？

如果答案是是，那么把它们折叠成一个 provider，shared auth 放在 `provider.auth`。

如果答案是否，那么就给每个 provider 各自配置自己的 `auth.credential`，不要再共享同一套身份。
