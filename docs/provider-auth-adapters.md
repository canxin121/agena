# Provider Auth + Adapters 重构说明

本文说明这次 `provider` / `credential` 重构的目标结构、落地方案和迁移路径。

## 目标

当前问题有两个：

1. `provider` 同时暴露 `base_url`、`api_key`、`api_key_env`、`auth_provider_id`，会让人误以为 `provider` 和 `credential` 是并列概念。
2. 一个 `credential` 事实上可能被多个 provider 复用，所以“一个 provider 对应一套独立 credential”这个模型也不成立。

这次重构把语义改成：

- `provider` 表示一个面向业务使用的逻辑入口。
- `credential` 是 `provider.auth` 的一种来源，而不是和 provider 并列。
- 一个 `provider` 可以包含多个 `adapter`。
- 每个 `adapter` 暴露一组模型。
- 每个模型继续支持 `variants`。

结构关系变成：

```text
provider
├── auth
├── adapters
│   ├── <adapter_id>
│   │   ├── kind
│   │   ├── endpoint/options
│   │   └── models
│   │       └── <visible_model_id>
│   │           ├── target_model
│   │           └── variants / capabilities / metadata patch
└── default_model
```

## 已实现内容

### 1. 新的 provider 解析模型

`ResolvedProviderConfig` 已经从旧的单一 `definition` 结构重构为：

- `default_model`
- `auth`
- `adapters`
- `models`

其中：

- `auth` 是共享认证配置。
- `adapters` 是 provider 下的多个后端适配器。
- `models` 是扁平化后的可见模型路由表，记录“这个模型走哪个 adapter，对应哪个上游 model”。

### 2. 新的 auth 模式

目前支持这些 provider 级认证模式：

- `none`
- `secret`
- `bedrock_sigv4`
- `google_adc`
- `sap_ai_core`

其中 `secret` 是最通用的共享认证模式，支持三种来源：

- 直接 secret
- 环境变量
- auth store credential（`credential_provider_id`）

这就把“直接 API key”和“credential”统一成了 provider 的一种认证配置。

### 3. 新的 adapter 结构

每个 adapter 独立保存自己的：

- `kind`
- `base_url` / `instance_url` / `region` 等 endpoint 选项
- `default_model`
- `models`

认证字段不再散落在 adapter 本身，而是从 provider 级 `auth` 注入。

### 4. 运行时 MultiAdapterProvider

运行时新增了 `MultiAdapterProvider`：

- 对外仍然暴露一个 provider id。
- 对内持有多个真实 adapter provider。
- 负责把可见 model id 路由到对应 adapter 的 `target_model`。
- 支持单 adapter passthrough。
- 支持多 adapter 显式模型路由。

### 5. legacy 配置兼容

旧配置依然能工作。

当前实现会把 legacy provider 配置自动 lower 成新结构：

- 原来的单 provider 会变成一个隐式 `default` adapter。
- 原来的 `api_key` / `api_key_env` / `auth_provider_id` 会被 lower 到 `provider.auth`。
- 原来的 `[providers.<id>.models]` 会被 lower 到新的 model route 表。

这意味着：

- 新结构已经是内部唯一运行时模型。
- 旧配置只是一个兼容输入层。

## 新配置示例

### 单 adapter，直接 API key

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

### 单 adapter，auth store credential

```toml
[providers.openai]
default_model = "gpt-4.1"

[providers.openai.auth]
credential_provider_id = "openai"

[providers.openai.adapters.api]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1"
```

### 多 adapter，共享一套 auth

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

## 约束

为了避免多 adapter 模型名冲突，这次实现加了一个硬约束：

- 多 adapter provider 必须显式声明每个 adapter 下暴露哪些 `models`。

这样路由关系在配置期就是确定的，不需要运行时猜测。

## 兼容性策略

这次重构采用的是“内部彻底重构，外部渐进迁移”：

1. 新配置结构已经可用。
2. 旧配置继续兼容。
3. runtime、REST auth 暴露逻辑、model 路由逻辑都基于新结构执行。
4. 后续可以逐步把文档、示例配置、UI 编辑器迁移到新结构。

## 后续建议

这次实现已经把核心数据模型和运行时路由落地，但还有几件值得继续推进的工作：

1. 在 `config.full.toml` 和 `docs/configuration.md` 里加入新结构示例。
2. 给 Studio 增加 `provider.auth` / `provider.adapters` 可视化编辑能力。
3. 对 legacy flat provider 配置增加 deprecation 提示，逐步推动迁移。
4. 补一轮针对多 adapter provider 的 API / session 端到端测试。
