# Agena

## Repository Layout

当前仓库已经统一重组为：

- `apps/`：可运行的 Rust 应用
- `crates/`：可复用的 Rust library crates
- `packages/`：前端包
- `ops/`：安装、打包、发布脚本
- `artifacts/`：生成的发布产物
- `docs/`：产品与接口文档

其中关键入口如下：

- CLI app：`apps/agena-cli`
- Unified v1 API server app：`apps/agena-studio-server`
- Unified v1 API server crate：`crates/agena-api-server`
- Studio server app：`apps/agena-studio-server`
- Studio desktop app：`apps/agena-studio-desktop`
- Studio web package：`packages/agena-studio-web`

## Agena Studio

`agena-studio` 相关入口现在统一如下：

- 详细说明：`[docs/agena-studio.md](docs/agena-studio.md)`
- 根仓库 CI：`[.github/workflows/ci.yml](.github/workflows/ci.yml)`
- Studio 发布流：`[.github/workflows/agena-studio-release.yml](.github/workflows/agena-studio-release.yml)`
- 后端打包：`ops/agena-studio/scripts/package-backend.sh` / `ops/agena-studio/scripts/package-backend.ps1`
- 桌面整包：`ops/agena-studio/desktop/build-full.sh` / `ops/agena-studio/desktop/build-full.ps1`
- 桌面 bundle 输出：`artifacts/agena-studio/desktop/`
- 桌面构建缓存：`artifacts/t/desktop/` 与 `artifacts/t/cef/`
- 后端安装脚本：`ops/agena-studio/scripts/install-service.sh` / `ops/agena-studio/scripts/install-service.ps1`

常用命令：

```bash
cargo check --workspace --locked
bash ops/agena-studio/scripts/package-backend.sh
bash ops/agena-studio/desktop/build-full.sh
```

Windows PowerShell：

```powershell
cargo check --workspace --locked
./ops/agena-studio/scripts/package-backend.ps1
./ops/agena-studio/desktop/build-full.ps1
```

## Unified v1 API

HTTP / SSE / WS 入口现在统一收敛到：

- library crate：`crates/agena-api-server`
- runnable app：`apps/agena-studio-server`

这样 Web / Studio / TUI 共用同一套统一的 `v1` server surface。

启动服务：

```bash
cargo run -p agena-studio-server -- --help
```

默认行为：

- 监听地址：`127.0.0.1:8765`
- 默认数据库：`~/.agena/agena.db`
- 默认 workspace root：当前工作目录

也可显式指定：

说明：

- HTTP server 入口位于 `apps/agena-studio-server`，使用 `cargo run -p agena-studio-server -- --help`
- 仓库根目录执行 `cargo test` 默认覆盖 `agena`
- 执行 `cargo test --workspace` 会覆盖当前 workspace 内的统一 server / client / app 包
- 工程化检查：
  - `cargo fmt --all --check` — 风格 (`rustfmt.toml`)
  - `cargo clippy --workspace --all-targets -- -D warnings` — lint (`clippy.toml`)
  - `cargo deny check` — 依赖许可证 / advisories (`deny.toml`)
  - `cargo audit` — RustSec advisories

已提供的资源接口：

- `GET /api/v1/health`
- `GET /api/v1/runtime`
- `POST /api/v1/runtime/reload`
- `GET /api/v1/auth/providers`
- `POST /api/v1/auth/providers/openai/browser/start`
- `POST /api/v1/auth/providers/openai/browser/finish`
- `POST /api/v1/auth/providers/openai/device/start`
- `POST /api/v1/auth/providers/openai/device/poll`
- `POST /api/v1/auth/providers/gitlab/browser/start`
- `POST /api/v1/auth/providers/gitlab/browser/finish`
- `POST /api/v1/auth/providers/github-copilot/device/start`
- `POST /api/v1/auth/providers/github-copilot/device/poll`
- `GET /api/v1/auth/providers/{provider_id}`
- `PUT /api/v1/auth/providers/{provider_id}/api-key`
- `DELETE /api/v1/auth/providers/{provider_id}`
- `POST /api/v1/auth/providers/{provider_id}/refresh`
- `GET /api/v1/providers`
- `GET /api/v1/providers/{provider_id}/models`
- `GET|POST /api/v1/workspaces`
- `POST /api/v1/workspaces/resolve`
- `GET|PUT|DELETE /api/v1/workspaces/{workspace_id}`
- `GET|POST /api/v1/sessions`
- `GET|PUT|DELETE /api/v1/sessions/{session_id}`
- `GET /api/v1/sessions/{session_id}/state`
- `GET /api/v1/sessions/{session_id}/events`
- `GET /api/v1/sessions/{session_id}/events/stream`
- `POST /api/v1/sessions/{session_id}/turns`
- `POST /api/v1/sessions/{session_id}/continue`
- `POST /api/v1/sessions/{session_id}/permission-replies`
- `POST /api/v1/sessions/{session_id}/user-input-replies`
- `GET /api/v1/sessions/{session_id}/messages`
- `GET /api/v1/messages/{message_id}`
- `GET /api/v1/messages/{message_id}/parts`
- `GET /api/v1/message-parts/{part_id}`
- `GET|POST /api/v1/permission-rules`
- `GET|PUT|DELETE /api/v1/permission-rules/{rule_id}`

分页与懒加载设计：

- 列表接口统一使用**不透明游标**（`cursor` + `limit`），而不是脆弱的 `offset/limit`
- `sessions` / `workspaces` / `permission-rules` 默认按最近更新时间倒序分页
- `messages` / `session events` 采用“按旧分页、按时间正序返回”的窗口设计，适合聊天 UI 的上翻加载
- 消息默认 `parts=summary`，只返回 part 摘要，不加载重型 `detail_json`
- 需要完整内容时再使用 `parts=full` 或单独请求 `/message-parts/{part_id}`
- session 执行动作接口只返回轻量状态、pending request 和最新事件位点，避免把整段历史反复回传

会话执行与并发控制：

- 现在已经打通 `submit turn / continue / permission reply / user input reply` 四类 runtime 接口
- `GET /api/v1/sessions/{session_id}/state` 可在页面刷新后恢复 blocked / pending / latest event seq
- 支持 `GET /api/v1/sessions/{session_id}/events/stream` 做 SSE 增量订阅
- 支持 session `version` + `If-Match` 的乐观并发控制，冲突时返回 `409`
- 未显式提供 `model` 时，会优先复用 session 最近一次模型；若 runtime 只配置了一个 provider，则自动退回该 provider 的默认模型

运行时与鉴权管理：

- `GET /api/v1/runtime` 提供当前 generation、config path、active mode、watch paths、provider/plugin/runtime 状态
- `POST /api/v1/runtime/reload` 提供手动 reload 入口
- `GET /api/v1/auth/providers*` 提供 provider credential 状态读取
- `PUT /api/v1/auth/providers/{provider_id}/api-key` / `DELETE ...` 提供 API key 管理
- 提供 OpenAI / GitLab browser OAuth 与 OpenAI / GitHub Copilot device login 的用户态登录接口
- public auth API 会隐藏内部凭据键（例如 `gitlab-instance`），避免把实现细节暴露给前端
- auth 读取接口只返回安全摘要，不回传明文 secret

Workspace 管理补充：

- `POST /api/v1/workspaces/resolve` 支持按 path 规范化解析 workspace，并可通过 `create_if_missing=true` 实现幂等式“存在即返回，不存在就创建”

这套设计把分页、懒加载和执行状态统一到了同一套资源语义上，适合直接给 Web / App 前端做无限滚动和局部 hydration。

更完整的接口说明见：

- [docs/http-api.md](docs/http-api.md)

## Config / Mode（新增）

现在提供完整的强类型配置体系：

- 默认配置路径：`~/.agena/config.toml`
- 可用 `AGENA_CONFIG` 指定配置文件
- 可用 `mode` / `AGENA_MODE` / `--mode` 切换配置 mode
- 配置优先级：**默认值 < 配置文件 < mode 叠加 < 环境变量 < CLI 覆盖**
- `ResolvedConfig::build_provider_registry()` 可直接把配置转成 provider registry

### CLI

```bash
agena config resolve --format toml
agena config validate
agena config mode --list
agena --mode prod -c providers.openai.default_model=gpt-5 config resolve --format json
```

### 支持的 provider config kind

- `preset`（基于 `models.dev` 的 provider 预设）
- `openai`
- `openai_compatible`
- `anthropic`
- `gemini`
- `codex`
- `gitlab`
- `copilot`
- `amazon_bedrock`
- `google_vertex`
- `cloudflare_ai_gateway`
- `alias`

示例见仓库根目录 `config.example.toml`。

### `preset` provider（基于 models.dev）

`agena` 现在支持：

- 通过 `kind = "preset"` 显式注册任意 `models.dev` provider id
- provider 元数据从 `models.dev` 拉取并缓存到 `~/.agena/provider-presets.json`
- 可用 `AGENA_PROVIDER_PRESETS_PATH` 指向本地 preset JSON，用于离线运行或测试

示例：

```toml
[providers.openrouter]
kind = "preset"

[providers."github-copilot"]
kind = "preset"

[providers."google-vertex"]
kind = "preset"

```

说明：

- `preset` 会按 `models.dev` 元数据补齐默认 `base_url`、默认 `default_model`、鉴权环境变量和少数 provider 的集成 header。
- 仍然是**显式注册**：只有配置文件里声明的 provider 才会进入 `agena` 的 provider registry。

## Plugin System（新增）

现在 `agena` 已经支持基于 `abi_stable` 的 dynamic plugin：

- 稳定 ABI 边界：`abi_stable`
- tool input schema：`schemars`
- 生命周期 hook：
  - `before_tool`
  - `after_tool`
  - `shell_env`
- 自定义 tool 会和内建 tool 一起下发给模型 provider

### 配置

```toml
[plugins]
enabled = true
paths = ["plugins"]
```

说明：

- `paths` 可填目录，也可直接填某个动态库文件。
- 相对路径相对配置文件目录解析。
- 如果省略 `paths`，默认会扫描配置文件同目录下的 `plugins/`。

### Sample Plugin

仓库里提供了一个完整样例：

- [examples/echo_plugin/Cargo.toml](/home/canxin/Git/ai/agena/examples/echo_plugin/Cargo.toml)
- [examples/echo_plugin/src/lib.rs](/home/canxin/Git/ai/agena/examples/echo_plugin/src/lib.rs)
- [examples/echo_plugin/README.md](/home/canxin/Git/ai/agena/examples/echo_plugin/README.md)

构建：

```bash
cd examples/echo_plugin
cargo build --release
```

然后把 `target/release/` 或生成出的动态库路径加入 `[plugins].paths` 即可。

## Provider / Model（显式注册）

当前 provider 架构改为：**仅显式注册**。

- 不再从环境变量自动发现/自动注册 provider。
- provider 参数通过构造函数显式传入。
- 自定义 provider 通过“内部 provider 别名”机制实现（不是新写协议栈）。

### 全局运行时参数（显式）

```rust
use std::time::Duration;
use agena::provider::{
    ProviderHttpClientConfig, ProviderRegistry, ProviderRuntimeConfig,
    ProviderRequestRetryConfig, ProviderStreamReplayConfig,
};

let client = ProviderRegistry::build_http_client(ProviderHttpClientConfig {
    timeout: Duration::from_secs(120),
    connect_timeout: Duration::from_secs(15),
})?;

let mut registry = ProviderRegistry::with_runtime_config(ProviderRuntimeConfig {
    request_retry: ProviderRequestRetryConfig {
        max_retries: 1,
        base_delay: Duration::from_millis(250),
        max_delay: Duration::from_millis(2_000),
    },
    stream_replay: ProviderStreamReplayConfig {
        max_retries_after_output: 1,
        max_tracked_events: 2_048,
    },
});
```

### 显式注册 provider（示例）

```rust
use agena::provider::OpenAiProvider;

registry.register(OpenAiProvider::new(
    client.clone(),
    "sk-xxx",
    "https://api.openai.com/v1",
    "gpt-4.1-mini",
));
```

### 自定义 provider（内部别名）

你可以把新 provider id 映射到内部 provider，实现“同一实现、多入口名”：

```rust
use agena::provider::ProviderAliasRegistration;

registry.register_alias(
    ProviderAliasRegistration::new("my-openai", "openai")
        .with_default_model("gpt-4.1-mini"),
)?;
```

行为说明：

- 调用 `my-openai` 实际执行的是内部 `openai` provider 代码。
- 返回结果和流式事件的 `provider_id` 会重写为别名 id（`my-openai`）。

### OpenAI-compatible 可显式自定义的项

通过 `OpenAiCompatibleProvider::new(...)` + builder 方法显式设置：

- `provider_id`
- `api_key`
- `base_url`
- `default_model`
- `auth_header` / `auth_scheme`
- `extra_headers`
- `stream_mode`（SSE / Realtime WS）
- `realtime_ws_url`

## 鉴权（OAuth / API Key）

鉴权能力现在收敛在 `agena::provider::auth` 模块：

- `AuthManager<FileAuthStore>`：统一管理 API key / OAuth token
- `FileAuthStore`：默认存储在 `~/.agena/auth.json`（可用 `AGENA_AUTH_FILE` 覆盖）

支持能力：

- OpenAI
  - API Key
  - OAuth Browser（PKCE）
  - OAuth Refresh（Codex token 续期）
- GitHub Copilot
  - Device Code（github.com 与 enterprise）
- GitLab
  - OAuth Authorization Code + PKCE
- Anthropic
  - API Key

示例（Rust）：

```rust
use agena::provider::auth::{AuthManager, FileAuthStore};

let mgr = AuthManager::new(FileAuthStore::new(FileAuthStore::default_path()));
mgr.set_anthropic_api_key("sk-ant-...")?;
```

OpenAI Browser OAuth（两段式）：

```rust
let start = mgr.start_openai_browser_login("http://localhost:1455/auth/callback")?;
// 打开 start.authorize_url，拿到 code 后：
let auth = mgr
    .finish_openai_browser_login(code, start.pkce_verifier, "http://localhost:1455/auth/callback")
    .await?;
```

OpenAI Browser OAuth（自动 callback 等待）：

```rust
use std::time::Duration;
use agena::provider::auth::wait_for_oauth_callback;

let redirect = "http://localhost:1455/auth/callback";
let start = mgr.start_openai_browser_login(redirect)?;
// 先打开 start.authorize_url 到浏览器
let callback = wait_for_oauth_callback(1455, &start.state, Duration::from_secs(300))?;
let _auth = mgr
    .finish_openai_browser_login(callback.code, start.pkce_verifier, redirect)
    .await?;
```

GitLab 也支持自动 callback 等待：

```rust
let (url, _auth) = mgr
    .gitlab_browser_login_auto("https://gitlab.com", 1455, Duration::from_secs(300))
    .await?;
```

Copilot Device Code（轮询）：

```rust
use agena::provider::auth::CopilotDeployment;

let s = mgr.start_copilot_login(CopilotDeployment::GitHubCom).await?;
// 用户在 s.verification_url 输入 s.user_code，随后循环 poll:
let maybe_auth = mgr
    .poll_copilot_login(s.device_code, CopilotDeployment::GitHubCom)
    .await?;
```

## 当前 provider 行为细节（本阶段）

- Codex (`openai` OAuth)
  - 请求前检查过期并自动 refresh
  - refresh 后回写 `auth.json`
  - 请求使用 Codex endpoint（`chatgpt.com/backend-api/codex/responses`）
  - 自动携带 `ChatGPT-Account-Id`（如果 token claims 可提取）

- GitHub Copilot
  - 支持 `github-copilot` / `github-copilot-enterprise`
  - 自动注入 `openai-intent: conversation-edits` / `x-initiator` / `User-Agent`
  - 视觉请求启发式设置 `Copilot-Vision-Request: true`
  - 模型路由规则：GPT-5（非 `gpt-5-mini`）走 `/responses`，其余走 `/chat/completions`

- OpenAI
  - 支持 `responses` / `chat.completions` 双路径（`OPENAI_API_MODE` 可控）
  - `auto` 下对高阶模型（如 gpt-5/o3/o4）优先走 responses

- Anthropic
  - 支持 `tool_use` / `tool_result` 块映射
  - usage 映射包含 cache write/read token

- Gemini
  - completion / stream 的 `provider_metadata` 会带回候选元数据（如 safety/grounding）

## 流式输出（SSE）

- `CodexProvider.complete_stream()`：使用 `responses` + `stream=true`，实时解析 SSE data 帧并输出 `TextDelta`。
- `CopilotProvider.complete_stream()`：
  - GPT-5（非 `gpt-5-mini`）走 `responses` 流式；
  - 其余走 `chat/completions` 流式；
  - 均会增量输出 `TextDelta`，结束时输出 `Completed`。
- `OpenAiProvider.complete_stream()`：OpenAI Responses 原生流式。
- `AnthropicProvider.complete_stream()`：Anthropic Messages 原生流式（`content_block_delta`）。
- `GeminiProvider.complete_stream()`：Gemini `streamGenerateContent` 流式（自动做增量 diff）。
- `OpenAiCompatibleProvider.complete_stream()`：通用 `chat/completions` SSE 流式（适配大量 provider）。

## 迁移说明（Breaking Changes）

本轮 provider 重构后，以下字段已升级为强类型：

1. `ProviderMessage.content`
   - 旧：`String`
   - 新：`ProviderContent`（`Text` / `Parts`）

2. `CompletionResponse.finish_reason`
   - 旧：`Option<String>`
   - 新：`Option<CompletionFinishReason>`

3. `CompletionResponse.usage`
   - 旧：`Option<MessageUsage>`
   - 新：`Option<CompletionUsage>`

4. `CompletionResponse` 新增字段
   - `tool_calls: Vec<CompletionToolCall>`
   - `provider_metadata: Option<serde_json::Value>`

5. `CompletionStreamEvent::Completed` 新增字段
   - `provider_metadata: Option<serde_json::Value>`

6. `CompletionStreamEvent` 新增事件
   - `ToolCallDelta`（用于流式 tool call 参数增量）

兼容性说明：

- 旧的纯文本调用仍可继续使用 `ProviderMessage::new(role, "text")`。
- 如需快速兼容旧逻辑，可通过 `ProviderMessage::as_text_lossy()` 获取文本降级结果。
