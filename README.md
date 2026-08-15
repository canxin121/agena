# Agena

本地优先的 LLM agent runtime：命令行、终端 UI、Studio Web 界面、后端 API、插件系统、
MCP/LSP 能力、会话存储、权限系统和多 provider 模型运行层。

## 文档

文档由源码直接生成（rustdoc），不再维护独立的 Markdown 文档：

```bash
cargo doc --workspace --no-deps --open
```

## 快速开始

```bash
mkdir -p ~/agena
cp config.example.json ~/agena/agena.json
# 编辑 ~/agena/agena.json，至少保留一个 provider 并设置对应凭据
cargo run -p agena --bin agena -- center start --workspace .
cargo run -p agena --bin agena -- config validate
cargo run -p agena --bin agena
```

一次性执行：

```bash
cargo run -p agena --bin agena -- exec "summarize this repository"
```

持续运行处理中心（HTTP API 默认监听 `127.0.0.1:3210`）：

```bash
cargo run -p agena --bin agena -- center --workspace .
```

也可以作为后台用户进程启动、查询和停止：

```bash
cargo run -p agena --bin agena -- center start --workspace .
cargo run -p agena --bin agena -- center status
cargo run -p agena --bin agena -- center stop
```

macOS launchd / Linux systemd 用户服务可安装为登录启动、失败自动重启的常驻服务：

```bash
cargo run -p agena --bin agena -- center install --workspace .
cargo run -p agena --bin agena -- center status
cargo run -p agena --bin agena -- center stop
cargo run -p agena --bin agena -- center start
cargo run -p agena --bin agena -- center uninstall
```

安装后的定义使用直接参数数组而非 shell，文件权限为用户私有；`center start/stop` 会优先
控制已安装的用户服务。未安装服务时，`center start` 仍使用 detached child 兼容路径。

TUI 默认按 `--center`、`AGENA_CENTER_URL`、本地 center record、
`http://127.0.0.1:3210` 的顺序连接处理中心；仅开发或恢复时使用
`agena tui --embedded`。`agena server` 仍是兼容别名。

Web、默认 TUI、IDE `rpc-server` 和一次性 CLI 共享同一个处理中心。会话执行、provider、
插件状态、认证、权限、usage/cost、memory、Git/snapshot 和配置诊断命令都通过公共 API；
CLI crate 不再启动自己的 Runtime。Git、snapshot、commit 和 PR 命令会校验当前目录与
处理中心 workspace 的 canonical path，不匹配时在写操作前拒绝。

`agena apply`、`agena mcp reconnect` 和 `agena mcp-server` 也由 center-owned operator API
执行。`mcp-server` 只是 stdio-to-center bridge：客户端 EOF 不会关闭 center 或取消工作。
`apply` 和 `mcp-server --workspace` 会先解析数据库中的 workspace identity，再把工具发现/
调用交给 center；每次 operator invoke 都必须携带该 `workspace_id`。center 会解析其持久化
路径，并与当前 Runtime executor 的 workspace root 做 canonical 比较；未知或不匹配的 id
会在任何工具执行前被拒绝。workspace resolve/create/update 也会把已存在目录 canonicalize
后再查找或持久化，避免 symlink（包括 macOS `/var`/`/private/var`）为同一目录创建多个 id。
所有实际工具执行和文件写入仍由 center 持有。

MCP add/remove/enable/disable 通过显式 `global`/`workspace` settings layer API 修改配置；
workspace 配置文件由 center 隐式选择，CLI 不能提交任意配置路径，修改后也由 center 决定
reload。MCP bearer keyring/file credential 与 OAuth discovery、PKCE、callback exchange、
logout/revoke 同样由 center 持有；返回值、普通配置和 endpoint record 都不包含 secret。

CLI、TUI 和 IDE bridge 可通过 `AGENA_CENTER_PASSWORD` 向启用 `--ui-password` 的 center
交换内存 bearer token，也可直接使用 `AGENA_CENTER_TOKEN`；secret 不写入 center record。
password 模式会把 password 保存在进程内的 zeroizing secret 中；center restart 或 token
失效后的第一个 HTTP/SSE handshake 收到 401 时，所有 client clones 共同完成一次重新交换并
只重放一次请求。静态 `AGENA_CENTER_TOKEN` 不会被误当作可刷新的 password。Provider
API-key、browser 和 device login 也由 center 完成。

当前仍有明确的收尾缺口：Windows service、真实平台 service 安装 smoke，以及真实 Web/TUI
画面级 submit/退出/观察组合。operator API 目前由 center auth 保护并具有服务端权威
workspace 边界，但 center 仍只组合一个 workspace-bound Runtime executor；真正的多
workspace/多租户支持还需要 per-scope executor，以及 auth principal 与 workspace grant
的绑定。处理中心与多客户端的目标边界、重连语义和分阶段迁移见
[处理中心架构 RFC](docs/processing-center-architecture.md)。

当前自动化已用隔离 fake provider + 真实 HTTP client 验证：提交客户端/SSE 断开后 center
继续完成、另一个客户端可观察并回答 user input、不同答案的并发 reply 只持久化一次且只
启动一次 continuation、cancel 与自然完成收敛到一个终态。真实 Web/TUI 进程组合、center
画面级观察，以及 launchd/systemd 的真实安装与 crash-restart 仍是剩余 gate。另一个真实
子进程 E2E 已在 provider-blocked execution 期间同时保持 Web 式 HTTP/SSE、默认 TUI PTY、
IDE RPC、MCP bridge，并运行一次性 CLI；Runtime composition audit 只记录 center PID，活动
lease 也只有一个 owner。center kill/restart reconciliation 同样已由真实子进程与
file-backed SQLite 覆盖。

## 环境要求

- Rust 1.97（见 `rust-toolchain.toml`）
- Bun（Studio Web 前端）
- SQLite（默认 `~/agena/agena.db`）
