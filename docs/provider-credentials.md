# Provider 与 Credential 关系说明

本文专门说明 Agena 里 provider、credential、auth store、`auth_provider_id` 之间的关系。

本文覆盖的核心实现位于：

- `crates/agena/src/config/raw.rs`
- `crates/agena/src/config/types.rs`
- `crates/agena/src/config/registry.rs`
- `crates/agena/src/provider/credential.rs`
- `crates/agena/src/provider/auth/{types,store,manager}.rs`
- `crates/agena-api-server/src/rest.rs`

如果只看 `config.toml`，很容易把 “provider 配置” 和 “credential 存储” 混成一个概念。实际上 Agena 把它们拆成了两层：

- provider 是运行时模型后端定义，来自 `[providers.<id>]`
- credential 是认证材料，来自配置文件、环境变量、auth store，或者 provider 自己的 fallback 机制

## 1. 总体关系

运行时大致可以概括成下面这条链路：

```text
config.toml
  └── [providers.<provider_id>]
        └── kind = "openai" / "gitlab" / ...
              └── RawProviderConfig
                    └── ResolvedProviderConfig
                          └── ProviderDefinition
                                └── build_provider_registry()
                                      └── ModelProvider

auth store
  └── <auth_provider_id> -> AuthData
        └── Api | OAuth | WellKnown

运行时构建 provider 时，会把上面两层拼起来：
  provider 配置决定“怎么连后端”
  credential 决定“拿什么身份去连”
```

最关键的点是：`provider_id` 和 `auth_provider_id` 有时相同，有时不同。

## 2. 先分清三个概念

### 2.1 `provider_id`

`provider_id` 是 `[providers.<id>]` 里的 `<id>`，也是运行时引用 provider 的主键。

它会出现在：

- `providers.<id>` 配置路径
- model ref，例如 `openai/gpt-4.1-mini`
- CLI 命令，例如 `agena provider models <provider-id>`
- HTTP API，例如 `/api/v1/providers/{provider_id}/models`
- Studio 的 provider 列表

`provider_id` 不等于 `kind`。它只是一个名字，可以是：

- `openai`
- `openrouter`
- `lmstudio`
- `codex`
- `github-copilot`
- `google-vertex`

### 2.2 `kind`

`kind` 表示 provider 的实现族，也就是底层用哪段 Rust 代码。

当前内建 `kind` 一共有 12 个：

```text
ollama
openai
openai_compatible
sap_ai_core
anthropic
gemini
codex
gitlab
copilot
amazon_bedrock
google_vertex
cloudflare_ai_gateway
```

同一个 `kind` 可以挂多个不同的 `provider_id`。例如：

```toml
[providers.openrouter]
kind = "openai_compatible"

[providers.lmstudio]
kind = "openai_compatible"
```

### 2.3 `auth_provider_id`

`auth_provider_id` 表示某个 provider 在运行时应该去 auth store 里读哪一个 credential。

只有部分 provider kind 需要显式配置它：

- `codex`
- `gitlab`
- `copilot`

其余 provider 大多数默认使用“和自己同名的 provider_id”去找 credential。

## 3. Credential 的数据模型

auth store 里的 credential 统一建模为 `AuthData`，当前有 3 种：

| 类型 | 结构 | 典型用途 |
| --- | --- | --- |
| `Api` | `{ key }` | 普通 API key / bearer token |
| `OAuth` | `{ refresh, access, expires_at_ms, account_id?, enterprise_url? }` | OpenAI、GitLab、GitHub Copilot 登录 |
| `WellKnown` | `{ key, token }` | 保存“已知附带信息”，当前主要用于内部辅助状态 |

### 3.1 `Api`

这是最常见的类型。通常来自：

- `agena login <provider-id> --api-key`
- Studio 的 “Save API Key”
- `PUT /api/v1/auth/providers/{provider_id}/api-key`

对于大部分 HTTP provider，`Api.key` 就会被当成：

- `Authorization: Bearer <key>`
- 自定义 header
- query parameter

具体怎么用，由 provider 实现自己决定。

### 3.2 `OAuth`

`OAuth` 同时保存：

- `refresh`
- `access`
- `expires_at_ms`
- 可选 `account_id`
- 可选 `enterprise_url`

当前内建的 OAuth 流程主要用于：

- OpenAI
- GitLab
- GitHub Copilot

但并不是所有使用 OAuth 的 provider 都有同样的运行时策略：

- `openai`：支持自动 refresh
- `codex`：要求 OpenAI OAuth，并在 provider 内部主动 refresh
- `gitlab`：支持自动 refresh
- `copilot`：读取 auth store 中的 refresh 或 access，但不走 OpenAI/GitLab 那套 refresh 逻辑

### 3.3 `WellKnown`

`WellKnown` 不是通用的用户 API key 模型，更像“内部保存的已知状态”。

当前最重要的例子是：

- `gitlab-instance`

GitLab 浏览器登录成功后，runtime 会额外写入：

- `gitlab` -> OAuth token
- `gitlab-instance` -> `WellKnown { key = instance_url, token = "" }`

`gitlab-instance` 不会暴露给公开 auth provider API，它是内部用来记住 GitLab 实例地址的。

## 4. Credential 存在哪里

`ResolvedConfig::auth_store()` 会把 auth store 后端配置成三种之一：

- `file`
- `keyring`
- `auto`

默认是 `auto`：

- 优先操作系统 keyring
- keyring 不可用时回退到文件

默认 auth 文件路径：

- `AGENA_AUTH_FILE`
- 否则 `~/.agena/auth.json`

keyring 模式下，磁盘上的 `auth.json` 更像索引，secret 本体在系统 keyring 里。

## 5. 运行时到底怎么解析 credential

不要把所有 provider 的 credential 解析顺序理解成完全一样。Agena 里至少有 3 套解析辅助函数：

- `required_managed_secret`
- `resolved_managed_secret`
- `resolved_or_deferred_managed_secret`

它们的行为不同。

### 5.1 通用理解

多数 provider 可以按下面顺序理解：

1. 直接写在配置里的 secret，例如 `api_key`、`access_token`
2. 配置里声明的环境变量，例如 `api_key_env`、`access_token_env`
3. auth store 里的 credential
4. provider 特有 fallback，例如 Vertex ADC、Bedrock SigV4、SAP AI Core service key

### 5.2 但有几个实现细节很重要

#### A. 大部分 HTTP provider 允许 “延迟到请求时再读 env”

`openai`、`openai_compatible`、`anthropic`、`gemini`、`cloudflare_ai_gateway`、`google_vertex` 的静态 token 模式、`amazon_bedrock` 的 bearer 模式，主要走的是 `resolved_or_deferred_managed_secret`。

这意味着：

- 如果 `api_key_env` 已经配置
- 但当前进程启动时环境变量还没值

provider registry 仍然可以先构建成功，真正发请求时才报缺少环境变量。

如果同名 auth store credential 已存在，它会优先使用 auth store。

#### B. GitLab 不是 “先声明 env 名，再运行时报错” 的模式

GitLab 会先判断是否“当前就存在直接 secret”：

- `api_key` 非空，或者
- `api_key_env` 指向的环境变量当前有值

只要成立，就走“直接 API key 模式”。

否则才走：

- `auth_provider_id` 指向的 auth store OAuth 模式

也就是说，GitLab 的 “直接 key 模式” 和 “OAuth 模式” 是构建 registry 时二选一的。

#### C. SAP AI Core 还有一条额外 fallback

`sap_ai_core` 的顺序大致是：

1. `api_key`
2. `api_key_env`
3. auth store 同名 credential
4. `AICORE_SERVICE_KEY`

如果前三者都没有，且设置了 `AICORE_SERVICE_KEY`，runtime 会把它解析成 service key，并在请求前换成临时 token。

#### D. Google Vertex 和 Amazon Bedrock 都有“非 auth store 型 fallback”

- `google_vertex`
  - 有 `access_token` / `access_token_env` 时用静态 token
  - 没有时走 Google ADC
- `amazon_bedrock`
  - 有 `api_key` / `api_key_env` 时走 bearer
  - 没有时走 SigV4

## 6. 所有内建 provider kind 一览

下面这张表按“provider kind”列出所有内建 provider 的 credential 关系。

| kind | 常见 `provider_id` 示例 | 主要配置字段 | credential 来源 | `auth_provider_id` 关系 | 备注 |
| --- | --- | --- | --- | --- | --- |
| `ollama` | `ollama` | `base_url`, `default_model` | 无 | 不涉及 | 本地模型，不需要 auth store |
| `openai` | `openai`, `openai-reasoning` | `base_url`, `default_model`, `api_key`, `api_key_env`, `api_mode`, `stream_mode` | 直接 key / env / 同名 auth store | 默认就是同名 provider id | auth store 里若是 OAuth，可自动 refresh |
| `openai_compatible` | `openrouter`, `lmstudio`, `vllm`, `groq`, `opencode*` 以外普通兼容端 | `base_url`, `default_model`, `api_key`, `api_key_env`, `auth_header`, `auth_scheme` | 直接 key / env / 同名 auth store | 默认同名 | 通常实际用 API key；代码也接受可提供 access token 的 auth store 记录 |
| `sap_ai_core` | `sap-ai-core` | `base_url`, `default_model`, `api_key`, `api_key_env` | 直接 key / env / 同名 auth store / `AICORE_SERVICE_KEY` | 默认同名 | `AICORE_SERVICE_KEY` 会换取临时 token |
| `anthropic` | `anthropic` | `base_url`, `default_model`, `api_key`, `api_key_env`, `auth_header`, `auth_scheme` | 直接 key / env / 同名 auth store | 默认同名 | 默认 header 是 `x-api-key` |
| `gemini` | `gemini` | `base_url`, `default_model`, `api_key`, `api_key_env` | 直接 key / env / 同名 auth store | 默认同名 | 默认把 key 放在 query parameter `key=`；部分场景可改成 header 模式 |
| `codex` | `codex` | `default_model`, `auth_provider_id` | auth store OAuth | 必须从 `auth_provider_id` 读 | 默认 `auth_provider_id = "openai"`；不走普通 API key 配置 |
| `gitlab` | `gitlab`, `gitlab-self` | `instance_url`, `ai_gateway_url`, `default_model`, `auth_provider_id`, `api_key`, `api_key_env` | 直接 key / env，或 auth store OAuth | 有 direct key 时看自己；否则看 `auth_provider_id` | OAuth 模式下 runtime 会自动 refresh，且会保存 `gitlab-instance` |
| `copilot` | `github-copilot`, `github-copilot-enterprise` | `base_url`, `models_url`, `default_model`, `auth_provider_id` | auth store OAuth 的 refresh/access | 总是从 `auth_provider_id` 读 | 企业版还依赖 `enterprise_url` 来推导实际 base URL |
| `amazon_bedrock` | `bedrock` | `base_url`, `default_model`, `region`, `api_key`, `api_key_env`, `profile`, `access_key_id`, `secret_access_key`, `session_token` | bearer key / env，或 SigV4 | 默认同名，但 SigV4 不读 auth store | `access_key_id` 和 `secret_access_key` 必须成对 |
| `google_vertex` | `google-vertex`, `vertex` | `base_url`, `default_model`, `access_token`, `access_token_env` | 静态 token / env，或 Google ADC | 默认同名，但 ADC 不读 auth store | 内部把 token 当 bearer 使用 |
| `cloudflare_ai_gateway` | `cloudflare-ai-gateway` | `base_url`, `default_model`, `api_key`, `api_key_env` | 直接 key / env / 同名 auth store | 默认同名 | model id 必须是 `provider/model` 这种 unified 格式 |

`opencode` 和 `opencode-go` 虽然配置上属于 `openai_compatible`，但 runtime 会走专门的 `OpencodeProvider` 实现，而不是普通的 `OpenAiCompatibleProvider`。

## 7. `provider_id` 与 `auth_provider_id` 的映射规则

HTTP API 的 `/api/v1/auth/providers` 并不是简单返回 `config.providers.keys()`，而是按 provider kind 推导“这个 provider 应该暴露哪个 auth provider id”。

内建规则如下：

| provider 定义 | 对外可见 auth provider id |
| --- | --- |
| `openai` / `anthropic` / `gemini` / `openai_compatible` / `sap_ai_core` / `cloudflare_ai_gateway` / `google_vertex` / `amazon_bedrock` / `ollama` | 就是 `provider_id` 自己 |
| `codex` | `config.auth_provider_id`，默认 `openai` |
| `copilot` | `config.auth_provider_id` |
| `gitlab`，且配置了当前可用 direct key/env | `provider_id` 自己 |
| `gitlab`，且没有 direct key/env | `config.auth_provider_id`，默认 `gitlab` |

这条规则决定了：

- Studio “Credentials” 页面会显示哪些条目
- `/api/v1/auth/providers` 会返回哪些 provider id
- `configured = true` 是对哪个 auth provider id 生效

但要特别注意：

- “某个 auth provider id 会出现在 API / Studio 里” 不等于 “对应 runtime provider 一定会读取 auth store”
- 当前 `ollama` 本身不消费 credential
- `google_vertex` 走 ADC 时不读 auth store
- `amazon_bedrock` 走 SigV4 时不读 auth store

也就是说，auth provider 列表更像“可管理 credential 的公开命名空间”，不是“实际 credential 消费路径”的严格镜像。

### 7.1 典型例子：`codex -> openai`

配置示例：

```toml
[providers.codex]
kind = "codex"
default_model = "gpt-5.3-codex"
auth_provider_id = "openai"
```

这表示：

- 运行时 provider id 是 `codex`
- 但 credential 不存到 `codex`
- 而是存到 `openai`

因此：

- `agena provider models codex` 查的是 `codex`
- `agena login openai --browser` 存的是 `openai` OAuth
- `codex` provider 会消费 `openai` 这条 OAuth credential

`codex` 还比普通 `openai` 更严格：

- 它要求 OAuth credential
- 不是普通 API key

### 7.2 典型例子：GitLab 的“双身份”

示例一，走 OAuth：

```toml
[providers.gitlab]
kind = "gitlab"
auth_provider_id = "gitlab"
```

此时：

- provider id = `gitlab`
- auth provider id = `gitlab`
- credential 放在 auth store 的 `gitlab`
- 另有内部辅助条目 `gitlab-instance`

示例二，provider 叫 `gitlab-self`，但 OAuth 仍复用 `gitlab`：

```toml
[providers.gitlab-self]
kind = "gitlab"
instance_url = "https://gitlab.example.com"
auth_provider_id = "gitlab"
```

只要没设置 direct `api_key` / `api_key_env`，它就会去读：

- auth store `gitlab`

而不是 `gitlab-self`。

示例三，provider 叫 `gitlab-self`，并且配置了 direct key：

```toml
[providers.gitlab-self]
kind = "gitlab"
instance_url = "https://gitlab.example.com"
api_key_env = "GITLAB_TOKEN"
```

只要 `GITLAB_TOKEN` 当前有值，runtime 就切到 direct key 模式：

- 对外 auth provider id 变成 `gitlab-self`
- 不再走 `auth_provider_id = "gitlab"` 那条 OAuth

### 7.3 典型例子：`github-copilot-enterprise`

企业版 Copilot 的关键不只是 token，还包括 `enterprise_url`。

设备登录时如果传了企业域名：

- auth store 写入的 provider id 是 `github-copilot-enterprise`
- OAuth 记录里还会保存 `enterprise_url`

provider 在运行时会根据这个 `enterprise_url` 推导真正的 base URL，例如：

```text
https://copilot-api.<enterprise-domain>
```

如果你把 provider 配成企业版，但 auth store 里没有 `enterprise_url`，请求时会失败。

## 8. 各 provider 的 credential 关系详解

这一节按“最容易混淆的关系”展开。

### 8.1 `openai`

`openai` kind 的 credential 选择逻辑：

1. `api_key`
2. `api_key_env`
3. auth store 同名条目
4. 如果只声明了 `api_key_env` 但当前环境变量为空，会把这个缺失推迟到真正请求时暴露

同名 auth store 条目既可以是：

- `Api`
- `OAuth`

当它是 `OAuth` 时，runtime 会用 `OpenAiOAuth` 策略自动 refresh。

但要注意两个层面：

- 配置上，任何 `provider_id` 的 `openai` kind 都能去读同名 auth store
- 操作入口上，内建 browser/device/refresh 只为 `openai` 这个 auth provider id 提供了一等支持

也就是说，如果你定义了：

```toml
[providers.my-openai]
kind = "openai"
```

那么：

- `agena login my-openai --api-key` 是可行的
- 但 `--browser` / `--device` 的一等流程并不是为 `my-openai` 设计的

### 8.2 `openai_compatible`

这是一个“壳层实现”，很多第三方后端都能接到这里。

典型 provider id：

- `openrouter`
- `lmstudio`
- `vllm`
- `groq`

credential 关系最简单：

- 优先 `api_key`
- 再 `api_key_env`
- 再 auth store 同名条目

一般实践上就是 API key。代码层面如果 auth store 里是可提供 access token 的记录，也能工作，但没有专门的 OAuth 登录流程。

### 8.3 `anthropic`

和 `openai_compatible` 很像，也是：

- `api_key`
- `api_key_env`
- auth store 同名条目

默认认证头是：

```text
x-api-key
```

如果 base URL 是 Anthropic 官方 host，provider 还会自动补 Anthropic beta 相关 header。

### 8.4 `gemini`

`gemini` 的 credential 默认不是放 `Authorization` 头，而是：

```text
?key=<api_key>
```

不过在部分包装场景里也可以切成 header 模式。比如 `OpencodeProvider` 内部复用了 `GeminiProvider`，会改成 `x-goog-api-key` header。

### 8.5 `codex`

`codex` 是一个非常特殊的 provider：

- 它不接受普通 `api_key` / `api_key_env` 配置
- 它的配置只有 `default_model` 和 `auth_provider_id`
- 它要求 auth store 里的 OAuth credential

默认配置就是：

```toml
[providers.codex]
kind = "codex"
auth_provider_id = "openai"
```

因此可以把它理解成：

- provider 叫 `codex`
- 认证身份借用 `openai`
- 但请求目标不是普通 OpenAI API，而是 Codex 专用端点

### 8.6 `gitlab`

`gitlab` 的 credential 有两种完全不同的运行模式：

- direct key 模式
- OAuth 模式

#### direct key 模式

触发条件：

- `api_key` 有值，或者
- `api_key_env` 指向的环境变量当前有值

此时：

- provider 直接拿这个 key 去访问 GitLab instance
- 再换取一段 direct access token
- 并把 direct access token 缓存一段时间

#### OAuth 模式

触发条件：

- 没有可用 direct key

此时：

- provider 从 `auth_provider_id` 读取 OAuth
- 读取的是 access token
- access 过期时可自动 refresh
- 浏览器登录时还会保存 `gitlab-instance`

### 8.7 `copilot`

`copilot` 不是简单拿 OAuth access token 当 bearer，而是优先读取：

1. `refresh`
2. 如果 `refresh` 为空，再读 `access`

也就是 `RefreshOrAccess` 选择器。

对 `github-copilot-enterprise` 来说，credential 之外还有一个隐含依赖：

- `enterprise_url`

没有它就无法从公共地址正确推导企业版 Copilot API 域名。

### 8.8 `amazon_bedrock`

Bedrock 有两条鉴权路线：

#### bearer 模式

只要配置了：

- `api_key`
- 或 `api_key_env`

就把它当作 bearer token，底层走 OpenAI-compatible 样式请求。

#### SigV4 模式

如果没有 bearer key，就走 AWS SigV4：

- `profile`
- `access_key_id`
- `secret_access_key`
- `session_token`

其中：

- `access_key_id` 和 `secret_access_key` 必须成对
- 也可以完全不写静态 key，交给 AWS SDK 环境解析

SigV4 这条线不使用 Agena auth store。

### 8.9 `google_vertex`

Vertex 也是双模：

#### static token 模式

配置了：

- `access_token`
- 或 `access_token_env`

就直接用它。

#### ADC 模式

如果没有 static token，就改走 Google ADC：

- `GOOGLE_APPLICATION_CREDENTIALS`
- gcloud default application credentials
- 或其他 GCP ambient identity

ADC 这条线也不经过 Agena auth store。

### 8.10 `sap_ai_core`

`sap_ai_core` 的特殊点不是 auth store，而是 `AICORE_SERVICE_KEY`。

可以把它理解成：

- 正常情况下像别的 HTTP provider 一样用 key
- 如果没 key，就允许使用一段 SAP service key 来动态换 token

### 8.11 `cloudflare_ai_gateway`

它的 credential 关系跟普通 `openai_compatible` 类似：

- `api_key`
- `api_key_env`
- auth store 同名条目

但模型命名要求更严格，必须是 unified model id，例如：

```text
workers-ai/@cf/meta/llama-3.1-8b-instruct
openai/gpt-4.1-mini
```

### 8.12 `ollama`

最简单：

- 不读 auth store
- 不要 API key
- 只要 `base_url` 和 `default_model`

## 9. 哪些 credential 能在 CLI / API / Studio 里直接管理

从“用户可操作入口”看，Agena 目前支持的管理能力如下。

### 9.1 通用能力

只要某个 auth provider id 是公开的，REST 和 Studio 就都支持：

- 写 API key
- 删除 credential

对应入口：

- CLI: `agena login <provider-id> --api-key ...`
- CLI: `agena logout <provider-id>`
- REST: `PUT /api/v1/auth/providers/{provider_id}/api-key`
- REST: `DELETE /api/v1/auth/providers/{provider_id}`
- Studio: Credentials 页的 Save / Delete

### 9.2 只对特定 provider 提供的一等登录流

| provider id | Browser OAuth | Device Flow | Refresh |
| --- | --- | --- | --- |
| `openai` | 支持 | 支持 | 支持 |
| `gitlab` | 支持 | 不支持 | 支持 |
| `github-copilot` | 不支持 | 支持 | 不支持专门 refresh 接口 |
| `github-copilot-enterprise` | 不支持单独 browser | 通过 device flow + enterprise domain | 不支持专门 refresh 接口 |
| 其他 provider id | 不支持 | 不支持 | 不支持 |

### 9.3 Studio 里为什么会显示某些 credential 条目

`/api/v1/auth/providers` 返回的是：

- 当前配置推导出的 public auth provider ids
- 加上 auth store 里已经存在的 provider ids
- 再排除内部条目，例如 `gitlab-instance`

所以你可能会在 Studio 里看到：

- 一个 provider 没配在 `config.toml` 里
- 但因为 auth store 里还有旧 credential，所以仍然被列出来

这时它通常会表现为：

- `configured = false`
- `credential_present = true`

## 10. 样例配置里能看到的 provider 关系

仓库当前样例配置里，已经体现了几种典型关系：

### 10.1 普通同名 provider

```toml
[providers.openai]
kind = "openai"
api_key_env = "OPENAI_API_KEY"
```

这里 provider id 和 auth provider id 都是 `openai`。

### 10.2 provider 和 auth provider 分离

```toml
[providers.codex]
kind = "codex"
auth_provider_id = "openai"
```

这里 provider id 是 `codex`，auth provider id 是 `openai`。

### 10.3 企业版 Copilot

```toml
[providers."github-copilot"]
kind = "copilot"
auth_provider_id = "github-copilot"
```

如果你另外再定义一个企业版 provider，也可以写成：

```toml
[providers."github-copilot-enterprise"]
kind = "copilot"
auth_provider_id = "github-copilot-enterprise"
```

此时企业域信息来自对应 auth store credential 的 `enterprise_url`。

## 11. Plugin provider 是另一层

除了上面 12 个内建 kind，runtime 还允许 plugin 通过 `provider.list` hook 动态增删 provider。

这类 provider 的特点是：

- 会出现在最终 provider registry 中
- 但不一定来自 `[providers.<id>]`
- credential 语义也不一定遵循本文这套内建规则

所以本文的 “所有 provider” 指的是：

- 所有内建 `ProviderDefinition`
- 以及它们和内建 auth store / credential 机制的关系

plugin 自己注入的 provider，要看对应 plugin 的实现。

## 12. 实际使用时最重要的判断规则

如果你要快速判断一个 provider 的 credential 应该怎么配，可以按这套顺序：

1. 先看它的 `kind`。
2. 再看它是否有 `auth_provider_id`。
3. 如果有 `api_key` / `api_key_env` 这类字段，确认它是 “直接 key 模式” 还是 “auth store 模式也可用”。
4. 如果是 `codex`、`gitlab`、`copilot`，优先看它们的特殊映射关系。
5. 如果是 `google_vertex` 或 `amazon_bedrock`，再判断是否会落到 ADC / SigV4 这种非 auth store 路径。

一句话概括：

- `provider_id` 决定你在 runtime 里“叫它什么”
- `kind` 决定你“怎么连它”
- `credential` 决定你“拿什么身份连它”
- `auth_provider_id` 决定你“去 auth store 的哪一格拿这个身份”
