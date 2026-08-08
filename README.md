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
cargo run -p agena --bin agena -- config validate
cargo run -p agena --bin agena
```

一次性执行：

```bash
cargo run -p agena --bin agena -- exec "summarize this repository"
```

## 环境要求

- Rust 1.97（见 `rust-toolchain.toml`）
- Bun（Studio Web 前端）
- SQLite（默认 `~/agena/agena.db`）
