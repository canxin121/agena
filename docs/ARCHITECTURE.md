# 架构概览

本文给新人提供 Agena 仓库的导航地图。详细行为描述放在子目录 README，
本文只解释「东西放在哪、为什么这样切」。

## 顶层布局

- `apps/` — 用户实际启动的可执行体
  - `agena-cli` — 统一终端入口（`agena` 二进制，默认直接启动 TUI，也承载 `exec` / `mcp-server` / `app-server` 等子命令）
  - `agena-tui` — 兼容性包装器，复用同一套 TUI 启动链
  - `agena-studio-server` — Studio Web 后端，HTTP/WebSocket
- `crates/` — 库 crate，按职责切分
  - `agena` — 核心运行时（agent / session / runtime / tool / provider /
    permission / config / memory / plugins）。当前最大的 crate，未来会拆。
  - `agena-api` — 与传输层无关的 query / command 类型
  - `agena-api-server` — 统一 transport crate，按 feature 提供 HTTP / WS / SSE / IPC / JSON-RPC
  - `agena-client` — REST/WS 客户端 SDK
  - `agena-mcp-client` / `agena-mcp-server` — Model Context Protocol 双端
  - `agena-plugin-sdk` / `agena-plugin-host` / `agena-plugin-marketplace` —
    插件三件套：作者 API、宿主装载、市场分发
  - `agena-keyring-store` — OS 钥匙串持久化
  - `agena-lsp` — LSP 客户端，用于 code-aware 工具
  - `agena-otel` — tracing-subscriber + OTLP 导出
  - `agena-rollout` — 文件系统会话回放/快照
  - `agena-scheduler` — 定时任务（cron 表达式）
  - `agena-skills` — Skill metadata 解析
  - `agena-marketplace-server` — 插件市场后端
- `ops/` — 运维脚本（构建前端、打包桌面端）
- `docs/` — 长文档
- `examples/` — 演示插件
- `packages/` — 前端 / TS 资源

## 数据流（典型一次会话）

```
user input
    │
    ▼
agena / agena-tui(compat) ──► agena::session::Manager
                              │
                              ├─► agena::permission   （工具调用门控）
                              ├─► agena::tool         （工具执行）
                              ├─► agena::provider     （LLM 调用）
                              └─► agena::event::Store （事件持久化）
                                       │
                                       ▼
                               agena-api-server (REST/WS) ──► Studio 前端
```

## 设计取舍

- **单 crate vs 多 crate**：`crates/agena` 当前是巨型 crate（~8 万行），
  正在按子模块拆分（runtime / session / providers / tools / cli 各自独立）。
  在拆分完成前，新代码应放进职责最贴近的现有子模块。
- **配置无热重载兼容性**：项目尚未发布，破坏式改 config 字段是允许的。
  但要更新 `config.full.toml` 与本文。
- **错误模型**：库 crate 用 `thiserror` 自定义 enum；二进制 crate 用 `anyhow`。
  禁止在库代码里 `unwrap()`/`expect()`，除非证明结构不变量。
- **异步模型**：tokio multi-thread runtime；CPU 密集的工具调用走
  `spawn_blocking`，并由 `Semaphore` 限流（见
  `session/manager.rs::execute_pending_tools_concurrently`）。
- **可观测性**：通过 `agena-otel` 输出 OTLP；HTTP 服务暴露
  `/healthz`、`/readyz`、`/metrics`。

## 入口推荐阅读顺序

1. `crates/agena/src/lib.rs` — 模块清单
2. `crates/agena/src/runtime/` — 运行时组装
3. `crates/agena/src/session/manager.rs` — 主循环
4. `crates/agena-api-server/src/lib.rs` — HTTP 路由表
5. `apps/agena-cli/src/main.rs` — CLI 启动器
