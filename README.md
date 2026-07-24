# Agena

Agena 是一个本地优先的 LLM agent runtime。仓库同时包含命令行、终端 UI、Studio Web 界面、后端 API、插件系统、通过 plugin 暴露的 MCP/LSP 能力、会话存储、权限系统和多 provider 模型运行层。

文档入口如下：

- [配置说明](docs/configuration.md): `agena.json`、环境变量、CLI 覆盖、provider、权限、插件和运行时默认值。
- [Provider Auth 与 Credential](docs/provider-credentials.md): 新的 `provider.auth + provider.adapters` 结构、provider-local credential 语义和运行时刷新规则。
- [内置插件与工具完整参考](docs/plugins-and-tools-reference.md): 当前构建加载的内置插件、工具协议、输入 schema、输出结构和运行时核对方式。

## 仓库布局

```text
apps/
  agena/                     # `agena` 唯一二进制入口，默认启动 TUI，也承载 exec/config/provider/session 等命令
  agena/                     # 唯一 binary，同时承载 HTTP server module

crates/
  agena/                     # 核心 runtime、配置、会话、权限、provider、事件、数据库、provided plugin tools
  agena-api/                 # API wire type，不绑定具体传输
  agena-api-server/          # HTTP/REST、SSE、WebSocket、IPC、JSON-RPC transport
  agena-client/              # Rust client SDK
  agena-plugin-host/         # 插件 host、transport、manifest、状态、日志、quota
  agena-plugin-sdk/          # 插件侧 SDK 和 hook/host API 类型
  agena-mcp-client/          # MCP client manager，供 agena.mcp plugin bridge 使用
  agena-lsp/                 # LSP registry/client，供 agena.lsp plugin 使用
  agena-skills/              # skill 发现，供 agena.skills plugin 使用

packages/
  agena-web-ui/              # Vue Agena 前端

scripts/
  dependencies/              # 依赖报告脚本
  agena/                     # Web、后端服务构建、打包和部署脚本

examples/
  echo_plugin/               # cdylib 插件示例
  echo_plugin_stdio/         # stdio 插件示例
  multi_tool_plugin_stdio/   # 多 tool stdio 插件示例
```

## 环境要求

- Rust toolchain: 仓库使用 `rust-toolchain.toml`，workspace package 要求 Rust `1.93`。
- Bun: Studio Web 使用 Bun、Vite、Vue 和 TypeScript。
- SQLite: 后端和 TUI 使用 SQLite 数据库，默认路径为 `~/agena/agena.db`。
- 可选: `gh` 用于部分 GitHub/PR 命令。

## 快速开始

准备最小配置：

```bash
mkdir -p ~/agena
cp config.example.json ~/agena/agena.json
```

编辑 `~/agena/agena.json`，至少保留一个 provider，并设置对应凭据。例如示例文件默认启用 Anthropic：

```bash
export ANTHROPIC_API_KEY=...
```

验证配置：

```bash
cargo run -p agena --bin agena -- config validate
cargo run -p agena --bin agena -- config resolve --format json
```

启动终端 UI：

```bash
cargo run -p agena --bin agena
```

执行一次性提示词：

```bash
cargo run -p agena --bin agena -- exec "summarize this repository"
```

查看 provider：

```bash
cargo run -p agena --bin agena -- provider list
cargo run -p agena --bin agena -- provider models anthropic
```

## Rust 开发构建

仓库根目录的默认 Cargo 成员是 `agena`；完整工作区构建和检查必须显式使用
`--workspace`。日常编辑优先只检查库，运行时也指定需要的正式二进制，避免链接
不相关目标：

```bash
cargo check -p agena --lib
cargo run -p agena --bin agena
```

`dsv4f` MCP fixture 和真实 provider 探针属于独立的内部工具包，不参与普通 CLI
构建：

```bash
cargo test -p agena-e2e --locked
cargo build -p agena-e2e --locked
```

开发构建使用显式的快速 dev profile：增量代码生成、无 LTO、有限调试信息；
`release`/`dist` 仍只用于发布。合并前再运行完整工作区验证：

```bash
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## 启动 Studio

开发前端：

```bash
bun install --cwd packages/agena-web-ui
bun run --cwd packages/agena-web-ui dev
```

启动后端 API 服务：

```bash
cargo run --bin agena -- server \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace-root "$PWD" \
  --cors-origin http://localhost:5173
```

只提供 API 时可以不传 `--ui-dir`。如果要由后端直接服务构建后的 UI：

```bash
bun run --cwd packages/agena-web-ui build
cargo run --bin agena -- server \
  --host 127.0.0.1 \
  --port 3210 \
  --workspace-root "$PWD" \
  --ui-dir packages/agena-web-ui/dist
```

Studio 服务公开：

- `GET /health`: Studio 层健康检查。
- `GET /auth/session`、`POST /auth/session`: 可选 UI 密码登录。
- `/api/v1/...`: Agena 后端 API。
- `/healthz`、`/readyz`、`/metrics`: API server liveness、readiness 和指标。

## 配置入口

全局配置文件固定为 `~/agena/agena.json`。工作区可以额外放置 `<workspace>/.agena/agena.json` 作为局部 partial 配置。配置层级按以下顺序合并，后者覆盖前者：

1. 内置默认值。
2. 全局 JSON 配置文件。
3. 工作区 JSON 配置文件。
4. 环境变量 overlay。
5. CLI 全局 `--set key=value` 覆盖。

常用覆盖示例：

```bash
cargo run -p agena --bin agena -- \
  --set default.provider=anthropic \
  --set default.adapter=anthropic \
  --set default.model=claude-sonnet-4-6 \
  config resolve
```

更多字段、默认值、环境变量和 merge 规则见 [配置说明](docs/configuration.md)。

## 常用命令

```bash
# TUI
agena
agena tui --workspace /path/to/repo

# 一次性执行
agena exec "explain crates/agena-api-server"
agena review --base main

# 会话
agena sessions list
agena resume --last
agena continue --last
agena fork <session-id> --message <message-id>

# 配置与诊断
agena config validate
agena config resolve --format json
agena diagnostics

# 用量统计（JSON）
agena usage --period thirty-days --timezone-offset-minutes 480
agena usage --period month --provider openai --include-subagents false

# provider 与 auth
agena provider list
agena provider models <provider-id>
agena login openai --browser
agena auth list
agena logout openai

# 插件
agena plugin status
agena plugin inspect <plugin-id>
agena plugin logs <plugin-id>
```

在 TUI 中输入 `/usage`（也可用 `/stats`、`/analytics`），或在非编辑状态按 `U`，可以打开完整的交互式用量面板。面板提供 9 种周期、provider/model 筛选、subagent 开关、3 种排序方式，以及按日、provider、model、session 的多维图表和表格。

## 开发检查

Rust workspace regression / e2e:

```bash
cargo fmt --all --check
cargo test -p agena --locked git_
```

Live suites (manual, requires provider credentials and network access):

```bash
cargo test --locked -p agena --test live_provider_catalog -- --ignored
```

Studio Web:

```bash
bun install --cwd packages/agena-web-ui
bun run --cwd packages/agena-web-ui typecheck
```

## 数据与状态位置

- 主配置: 固定为 `~/agena/agena.json`。
- 数据库: 默认 `~/agena/agena.db`，可用 `AGENA_DATABASE_URL` 或 `AGENA_DATABASE_PATH` 改写。
- Provider 凭据: 保存在主配置的 `[providers.<id>.auth]` 下，登录和 refresh 会直接回写该 provider。
- 插件存储: 默认 `~/agena/plugin-storage`，可用 `AGENA_PLUGIN_STORAGE_DIR` 改写。
- Marketplace cache: 默认 `~/agena/marketplace`，可用 `AGENA_MARKETPLACE_DIR` 改写。

## 实现来源

关键实现文件：

- CLI: `apps/agena/src/main.rs`、`crates/agena-cli/src/`
- Runtime/config: `crates/agena-runtime/src/runtime/`、`crates/agena-runtime/src/config/`
- Session/event/db: `crates/agena-runtime/src/session/`、`crates/agena-runtime/src/event/`、`crates/agena-storage-sqlite/src/`
- API: `crates/agena-api/`、`crates/agena-api-server/`、`crates/agena-client/`
- Server: `apps/agena/src/server/`、`packages/agena-web-ui/`
- Plugins: `crates/agena-plugin-host/`、`crates/agena-plugin-sdk/`
